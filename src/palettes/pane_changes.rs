//! Which panes changed since you last looked at them, and what the AI agents
//! in them are doing: the marks Find Pane draws on its tree.
//!
//! tmux keeps no per-pane activity time, and a program that redraws in place
//! (Claude Code, vim, htop) leaves `history_bytes` and the cursor where they
//! were, so what's compared is a pane's *screen text*. Its fingerprint is the
//! pane's size plus a hash of that text. The fingerprint as of your last look
//! is kept on the pane itself, in the [`SEEN_OPTION`] pane option: it outlives
//! the popup, needs no state file, and goes away with the pane.
//!
//! A pane counts as seen when:
//! - it is in the window you were looking at as the palette opened;
//! - its preview has been up for [`DWELL`] (arrowing past it doesn't count);
//! - you jump to it with Enter;
//! - you leave it, if tmux reports that (`focus-events on`): the plugin's
//!   `pane-focus-out` hook runs `tmux-palette mark-seen <pane>`, which is
//!   [`mark_seen_now`].
//!
//! A pane that has never been looked at, or whose size changed since (a resize
//! or zoom redraws it with no new output), starts over as seen rather than
//! marked.
//!
//! An agent's state comes from the [`STATE_OPTION`] pane option, which the
//! agent's own hooks set (see the README for Claude Code's). Without one, a
//! Claude Code pane counts as working while its status line reads
//! "esc to interrupt".

use std::cell::RefCell;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::tmux::tmux;

/// Pane option holding the pane's fingerprint as of the last time you saw it.
pub const SEEN_OPTION: &str = "@palette-seen";
/// Pane option an agent's hooks set to `working`, `waiting` or `done`.
pub const STATE_OPTION: &str = "@palette-agent-state";

/// How long a pane's preview must stay up before the pane counts as seen.
const DWELL: Duration = Duration::from_millis(1000);
/// The panes' screens are read again at most this often: once a live-preview
/// tick at the default `previewRefresh`, and no more on fast typing.
const MIN_REFRESH: Duration = Duration::from_millis(250);
/// Non-blank rows read from the bottom of a Claude Code screen for its status
/// line, which sits above the input box (and any to-do list under it).
const STATUS_ROWS: usize = 15;
/// What Claude Code's status line starts with: its spinner's frames.
const CLAUDE_SPINNER: &[char] = &['·', '✢', '*', '✳', '✶', '✻', '✽'];

/// What an AI agent in a pane is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    Working,
    /// Stopped to ask you something: a permission prompt, a question.
    Waiting,
    Done,
}

/// A pane to keep an eye on, as the tree lists it.
#[derive(Clone, Debug, Default)]
pub struct PaneInfo {
    /// How tmux is told which pane (its `%N` id).
    pub id: String,
    pub session: String,
    /// The agent detected in it (`claude`, `codex`, ...), or `""`.
    pub agent: String,
    /// In the window you were looking at when the palette opened.
    pub on_screen: bool,
}

/// Find Pane's view of which panes changed. Shared by the tree's renderer, the
/// preview (which says what you're looking at) and the keys, so the state sits
/// behind a `RefCell`.
#[derive(Default)]
pub struct PaneChanges {
    tracker: RefCell<Tracker>,
}

impl PaneChanges {
    /// Watch these panes. Nothing is read yet: the launcher builds the tree
    /// just to measure the popup, so the first [`PaneChanges::refresh`] does.
    pub fn track(&self, panes: Vec<PaneInfo>) {
        let mut t = self.tracker.borrow_mut();
        t.panes = panes.into_iter().map(Watched::new).collect();
        t.refreshed = None;
    }

    /// Read the panes' screens again (at most every [`MIN_REFRESH`]) and store
    /// what has been seen since. Runs before each frame.
    pub fn refresh(&self) {
        let now = Instant::now();
        let ids: Vec<String> = {
            let mut t = self.tracker.borrow_mut();
            let recent = t
                .refreshed
                .is_some_and(|r| now.duration_since(r) < MIN_REFRESH);
            if t.panes.is_empty() || recent {
                return;
            }
            t.refreshed = Some(now);
            t.panes.iter().map(|w| w.info.id.clone()).collect()
        };
        let snaps = snapshot(&ids);
        let marks = {
            let mut t = self.tracker.borrow_mut();
            t.apply(&snaps);
            t.settle(now)
        };
        store_seen(&marks);
    }

    /// The pane whose preview is showing, or `None`: starts the [`DWELL`]
    /// clock when it's a different one.
    pub fn viewing(&self, id: Option<&str>) {
        self.tracker.borrow_mut().view(id, Instant::now());
    }

    /// Store the previewed pane as seen if it has been up long enough. Runs
    /// before each key, which may move off it or close the popup.
    pub fn settle(&self) {
        let marks = self.tracker.borrow_mut().settle(Instant::now());
        store_seen(&marks);
    }

    /// Store `id` as seen as it stands (Enter jumps to it).
    pub fn mark_seen(&self, id: &str) {
        let marks: Vec<_> = self.tracker.borrow_mut().mark(id).into_iter().collect();
        store_seen(&marks);
    }

    /// Whether `id` shows output you haven't seen.
    pub fn changed(&self, id: &str) -> bool {
        self.tracker.borrow().find(id).is_some_and(Watched::changed)
    }

    /// What the agent in `id` is doing, when known.
    pub fn state(&self, id: &str) -> Option<AgentState> {
        self.tracker.borrow().find(id).and_then(|w| w.state)
    }

    /// How many of `session`'s panes show output you haven't seen.
    pub fn changed_in(&self, session: &str) -> usize {
        self.tracker
            .borrow()
            .panes
            .iter()
            .filter(|w| w.info.session == session && w.changed())
            .count()
    }
}

/// Store `id` as seen as it stands now: the `pane-focus-out` hook's
/// `mark-seen <pane>`, run as you leave a pane.
pub fn mark_seen_now(id: &str) {
    let marks: Vec<_> = snapshot(&[id.to_string()])
        .into_iter()
        .map(|s| (s.id, s.fingerprint))
        .collect();
    store_seen(&marks);
}

// ---- the panes' state -------------------------------------------------------

#[derive(Default)]
struct Tracker {
    panes: Vec<Watched>,
    /// The previewed pane, and since when.
    viewing: Option<(String, Instant)>,
    /// When the screens were last read; `None` until the first time.
    refreshed: Option<Instant>,
}

struct Watched {
    info: PaneInfo,
    /// The fingerprint as of your last look (from [`SEEN_OPTION`]).
    seen: String,
    /// The fingerprint now; `""` until read.
    now: String,
    state: Option<AgentState>,
}

impl Watched {
    fn new(info: PaneInfo) -> Watched {
        Watched {
            info,
            seen: String::new(),
            now: String::new(),
            state: None,
        }
    }

    /// New output since you last saw it. A working agent's screen changes all
    /// the time, and saying so would only repeat "working"; once it stops,
    /// what it left behind counts.
    fn changed(&self) -> bool {
        !self.info.on_screen
            && !self.now.is_empty()
            && !self.seen.is_empty()
            && self.seen != self.now
            && self.state != Some(AgentState::Working)
    }
}

impl Tracker {
    fn find(&self, id: &str) -> Option<&Watched> {
        self.panes.iter().find(|w| w.info.id == id)
    }

    /// Take in a fresh read. tmux holds the baseline (we write it there), so
    /// it's read back each time; a pane missing from the read (gone, or the
    /// read failed) keeps what it had.
    fn apply(&mut self, snaps: &[Snap]) {
        for w in &mut self.panes {
            let Some(s) = snaps.iter().find(|s| s.id == w.info.id) else {
                continue;
            };
            w.seen = s.seen.clone();
            w.now = s.fingerprint.clone();
            // A state is only believed on a pane running an agent: one that
            // exited without its last hook firing would leave it behind.
            w.state = if w.info.agent.is_empty() {
                None
            } else {
                agent_state(&s.state, w.info.agent == "claude", &s.screen)
            };
        }
    }

    fn view(&mut self, id: Option<&str>, now: Instant) {
        match id {
            Some(id) if self.viewing.as_ref().is_some_and(|(v, _)| v == id) => {}
            Some(id) => self.viewing = Some((id.to_string(), now)),
            None => self.viewing = None,
        }
    }

    /// Count as seen what has been by `now`, returning the (pane, fingerprint)
    /// pairs to store.
    fn settle(&mut self, now: Instant) -> Vec<(String, String)> {
        let watched = self
            .viewing
            .as_ref()
            .filter(|(_, since)| now.duration_since(*since) >= DWELL)
            .map(|(id, _)| id.clone());
        let mut marks = Vec::new();
        for w in &mut self.panes {
            if w.now.is_empty() || w.seen == w.now {
                continue;
            }
            let seen = w.info.on_screen
                || no_baseline(&w.seen, &w.now)
                || watched.as_deref() == Some(w.info.id.as_str());
            if seen {
                w.seen = w.now.clone();
                marks.push((w.info.id.clone(), w.now.clone()));
            }
        }
        marks
    }

    fn mark(&mut self, id: &str) -> Option<(String, String)> {
        let w = self.panes.iter_mut().find(|w| w.info.id == id)?;
        if w.now.is_empty() || w.seen == w.now {
            return None;
        }
        w.seen = w.now.clone();
        Some((w.info.id.clone(), w.now.clone()))
    }
}

/// Whether `seen` is no baseline for `now`: never stored, or stored at another
/// size.
fn no_baseline(seen: &str, now: &str) -> bool {
    seen.is_empty() || size_of(seen) != size_of(now)
}

/// The `80x24` half of a fingerprint.
fn size_of(fingerprint: &str) -> &str {
    fingerprint.split_once(':').map_or("", |(size, _)| size)
}

// ---- agents -----------------------------------------------------------------

/// The agent's state: what its hooks last set, else, for Claude Code, whether
/// its status line says it's working.
fn agent_state(option: &str, claude: bool, screen: &str) -> Option<AgentState> {
    match option.trim() {
        "working" => Some(AgentState::Working),
        "waiting" => Some(AgentState::Waiting),
        "done" | "idle" => Some(AgentState::Done),
        _ => (claude && claude_working(screen)).then_some(AgentState::Working),
    }
}

/// Claude Code's status line while it works: a spinner frame, then e.g.
/// `Cogitating… (12s · esc to interrupt)`. Only rows near the bottom are read,
/// and only ones led by a spinner frame, since the conversation above can
/// quote the phrase.
fn claude_working(screen: &str) -> bool {
    screen
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(STATUS_ROWS)
        .any(|l| l.starts_with(CLAUDE_SPINNER) && l.contains("esc to interrupt"))
}

// ---- reading and writing tmux -----------------------------------------------

/// One pane as a snapshot read it.
#[derive(Debug, PartialEq)]
struct Snap {
    id: String,
    fingerprint: String,
    /// [`SEEN_OPTION`]'s value.
    seen: String,
    /// [`STATE_OPTION`]'s value.
    state: String,
    screen: String,
}

/// FNV-1a: small, and the same in every build. A fingerprint outlives the
/// binary that stored it, so std's `DefaultHasher`, free to change between
/// Rust releases, won't do.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn fingerprint(width: &str, height: &str, screen: &str) -> String {
    format!("{}x{}:{:016x}", width, height, fnv1a(screen.as_bytes()))
}

/// A captured screen as compared: trailing spaces and blank rows dropped, so
/// it reads the same wherever it sat in the batch.
fn screen_text(rows: &[&str]) -> String {
    let mut rows: Vec<&str> = rows.iter().map(|r| r.trim_end()).collect();
    while rows.last().is_some_and(|r| r.is_empty()) {
        rows.pop();
    }
    rows.join("\n")
}

/// A marker no screen can hold, leading the header line before each capture.
fn nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("tmux-palette-snapshot-{}-{}", std::process::id(), nanos)
}

/// One tmux call reading every pane: a header line (`display-message`; spaces
/// between fields, since tmux escapes tabs outside a UTF-8 locale), then its
/// screen as text (`capture-pane`, no colors: a cursor or highlight moving is
/// not new output).
fn snapshot_args(ids: &[String], nonce: &str) -> Vec<String> {
    let header = format!(
        "{} #{{pane_id}} #{{pane_width}} #{{pane_height}} #{{{}}} #{{{}}}",
        nonce, SEEN_OPTION, STATE_OPTION
    );
    let mut args: Vec<String> = Vec::new();
    for id in ids {
        if !args.is_empty() {
            args.push(";".into());
        }
        for a in ["display-message", "-p", "-t", id, &header, ";"] {
            args.push(a.to_string());
        }
        for a in ["capture-pane", "-p", "-t", id] {
            args.push(a.to_string());
        }
    }
    args
}

fn parse_snapshot(out: &str, nonce: &str) -> Vec<Snap> {
    struct Head<'a> {
        id: &'a str,
        width: &'a str,
        height: &'a str,
        seen: &'a str,
        state: &'a str,
    }
    let snap = |h: Head, rows: &[&str]| {
        let screen = screen_text(rows);
        Snap {
            id: h.id.to_string(),
            fingerprint: fingerprint(h.width, h.height, &screen),
            seen: h.seen.to_string(),
            state: h.state.to_string(),
            screen,
        }
    };
    let mut snaps = Vec::new();
    let mut current: Option<(Head, Vec<&str>)> = None;
    for line in out.split('\n') {
        match line.strip_prefix(nonce).and_then(|r| r.strip_prefix(' ')) {
            Some(fields) => {
                if let Some((h, rows)) = current.take() {
                    snaps.push(snap(h, &rows));
                }
                // The state is last and taken whole: a hook could put spaces in.
                let mut f = fields.splitn(5, ' ');
                let mut next = || f.next().unwrap_or("");
                let head = Head {
                    id: next(),
                    width: next(),
                    height: next(),
                    seen: next(),
                    state: next(),
                };
                current = (!head.id.is_empty()).then(|| (head, Vec::new()));
            }
            None => {
                if let Some((_, rows)) = current.as_mut() {
                    rows.push(line);
                }
            }
        }
    }
    if let Some((h, rows)) = current {
        snaps.push(snap(h, &rows));
    }
    snaps
}

/// `tmux <args>`'s output, or `None` if it failed.
fn run_tmux(args: &[String]) -> Option<String> {
    let out = Command::new("tmux").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Read `ids`. A failing command stops the rest of a tmux batch, so if one
/// went away meanwhile, read again without it.
fn snapshot(ids: &[String]) -> Vec<Snap> {
    if ids.is_empty() {
        return Vec::new();
    }
    let nonce = nonce();
    if let Some(out) = run_tmux(&snapshot_args(ids, &nonce)) {
        return parse_snapshot(&out, &nonce);
    }
    let live = tmux(&["list-panes", "-a", "-F", "#{pane_id}"]);
    let alive: Vec<String> = ids
        .iter()
        .filter(|id| live.lines().any(|l| l == id.as_str()))
        .cloned()
        .collect();
    if alive.is_empty() || alive.len() == ids.len() {
        return Vec::new();
    }
    run_tmux(&snapshot_args(&alive, &nonce))
        .map(|out| parse_snapshot(&out, &nonce))
        .unwrap_or_default()
}

fn store_seen_args(marks: &[(String, String)]) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    for (id, fingerprint) in marks {
        if !args.is_empty() {
            args.push(";".into());
        }
        for a in ["set-option", "-p", "-t", id, SEEN_OPTION, fingerprint] {
            args.push(a.to_string());
        }
    }
    args
}

/// Write the baselines, in one call; if a pane that went away stopped the
/// batch, one at a time.
fn store_seen(marks: &[(String, String)]) {
    if marks.is_empty() || run_tmux(&store_seen_args(marks)).is_some() {
        return;
    }
    if marks.len() > 1 {
        for m in marks {
            let _ = run_tmux(&store_seen_args(std::slice::from_ref(m)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: &str = "N0NCE";

    /// What tmux prints for one pane of a snapshot.
    fn block(id: &str, size: (u32, u32), seen: &str, state: &str, screen: &str) -> String {
        format!(
            "{NONCE} {id} {} {} {seen} {state}\n{screen}\n",
            size.0, size.1
        )
    }

    fn info(id: &str, agent: &str, on_screen: bool) -> PaneInfo {
        PaneInfo {
            id: id.into(),
            session: "main".into(),
            agent: agent.into(),
            on_screen,
        }
    }

    fn tracker(panes: Vec<PaneInfo>) -> Tracker {
        Tracker {
            panes: panes.into_iter().map(Watched::new).collect(),
            ..Default::default()
        }
    }

    fn snap(id: &str, seen: &str, screen: &str) -> Snap {
        Snap {
            id: id.into(),
            fingerprint: fingerprint("80", "24", screen),
            seen: seen.into(),
            state: String::new(),
            screen: screen.into(),
        }
    }

    #[test]
    fn fingerprints_are_stable_and_size_aware() {
        // Known FNV-1a vectors: the stored value must not drift between builds.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fingerprint("80", "24", ""), "80x24:cbf29ce484222325");
        assert_eq!(size_of("80x24:cbf29ce484222325"), "80x24");
        assert_eq!(size_of("garbage"), "");

        assert!(no_baseline("", "80x24:1"));
        assert!(no_baseline("100x24:1", "80x24:1"));
        assert!(!no_baseline("80x24:2", "80x24:1"));
    }

    #[test]
    fn a_screen_reads_the_same_wherever_it_sat_in_the_batch() {
        assert_eq!(screen_text(&["$ ls  ", "foo", "", "  "]), "$ ls\nfoo");
        assert_eq!(screen_text(&["", ""]), "");
        // In the middle of a batch a capture ends at the next header; at the
        // end it ends with the output's last newline: the same screen either way.
        let a = block("%1", (80, 24), "", "", "one\ntwo\n\n");
        let b = block("%2", (80, 24), "", "", "one\ntwo\n\n");
        let snaps = parse_snapshot(&format!("{a}{b}"), NONCE);
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].fingerprint, snaps[1].fingerprint);
        assert_eq!(snaps[0].screen, "one\ntwo");
    }

    #[test]
    fn parses_a_snapshot() {
        let out = format!(
            "{}{}",
            block("%1", (80, 24), "80x24:abc", "", "$ make\nok"),
            block("%7", (120, 40), "", "waiting for you", "✻ Thinking…")
        );
        let snaps = parse_snapshot(&out, NONCE);
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].id, "%1");
        assert_eq!(snaps[0].seen, "80x24:abc");
        assert_eq!(snaps[0].fingerprint, fingerprint("80", "24", "$ make\nok"));
        assert_eq!(snaps[0].screen, "$ make\nok");
        // Empty fields stay in place, and the state is taken whole.
        assert_eq!(snaps[1].seen, "");
        assert_eq!(snaps[1].state, "waiting for you");
        assert!(snaps[1].fingerprint.starts_with("120x40:"));

        // A screen that quotes a header without the nonce is just text.
        let tricky = block("%1", (80, 24), "", "", "other #{pane_id} 80 24\nx");
        let snaps = parse_snapshot(&tricky, NONCE);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].screen, "other #{pane_id} 80 24\nx");
        assert!(parse_snapshot("noise before any header\n", NONCE).is_empty());
    }

    #[test]
    fn batches_one_tmux_call() {
        let ids = vec!["%1".to_string(), "%2".to_string()];
        let args = snapshot_args(&ids, NONCE);
        let header = format!("{NONCE} #{{pane_id}} #{{pane_width}} #{{pane_height}} #{{@palette-seen}} #{{@palette-agent-state}}");
        let expected: Vec<String> = [
            "display-message",
            "-p",
            "-t",
            "%1",
            &header,
            ";",
            "capture-pane",
            "-p",
            "-t",
            "%1",
            ";",
            "display-message",
            "-p",
            "-t",
            "%2",
            &header,
            ";",
            "capture-pane",
            "-p",
            "-t",
            "%2",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(args, expected);

        let marks = vec![
            ("%1".to_string(), "a".to_string()),
            ("%2".to_string(), "b".to_string()),
        ];
        assert_eq!(
            store_seen_args(&marks).join(" "),
            "set-option -p -t %1 @palette-seen a ; set-option -p -t %2 @palette-seen b"
        );
    }

    #[test]
    fn panes_never_seen_or_resized_start_out_seen() {
        let mut t = tracker(vec![info("%1", "", false), info("%2", "", false)]);
        let resized = Snap {
            seen: "120x40:0".into(),
            ..snap("%2", "", "x")
        };
        t.apply(&[snap("%1", "", "hello"), resized]);
        let marks = t.settle(Instant::now());
        assert_eq!(marks.len(), 2, "both get a baseline");
        assert!(!t.find("%1").unwrap().changed());
        assert!(!t.find("%2").unwrap().changed());
    }

    #[test]
    fn new_output_since_the_last_look_is_marked() {
        let old = fingerprint("80", "24", "$ make");
        let mut t = tracker(vec![info("%1", "", false), info("%2", "", false)]);
        t.apply(&[
            snap("%1", &old, "$ make\nerror!"),
            snap("%2", &old, "$ make"),
        ]);
        assert!(t.settle(Instant::now()).is_empty(), "nothing seen yet");
        assert!(t.find("%1").unwrap().changed());
        assert!(!t.find("%2").unwrap().changed());

        // Jumping to it stores it as seen, once.
        assert_eq!(t.mark("%1").map(|(id, _)| id).as_deref(), Some("%1"));
        assert!(!t.find("%1").unwrap().changed());
        assert!(t.mark("%1").is_none());
        assert!(t.mark("%9").is_none());
    }

    #[test]
    fn panes_on_screen_are_always_seen() {
        let old = fingerprint("80", "24", "a");
        let mut t = tracker(vec![info("%1", "", true)]);
        t.apply(&[snap("%1", &old, "b")]);
        assert_eq!(t.settle(Instant::now()).len(), 1);
        assert!(!t.find("%1").unwrap().changed());
    }

    #[test]
    fn a_preview_counts_once_it_has_been_up_a_moment() {
        let old = fingerprint("80", "24", "a");
        let mut t = tracker(vec![info("%1", "", false), info("%2", "", false)]);
        t.apply(&[snap("%1", &old, "b"), snap("%2", &old, "b")]);
        let start = Instant::now();

        // Arrowing past %1 doesn't count.
        t.view(Some("%1"), start);
        t.view(Some("%2"), start + DWELL / 2);
        assert!(t.settle(start + DWELL).is_empty());
        // Looking at %2 a while does, and only %2.
        let marks = t.settle(start + DWELL / 2 + DWELL);
        assert_eq!(marks, vec![("%2".to_string(), t.panes[1].now.clone())]);
        assert!(t.find("%1").unwrap().changed());
        // Seeing the same pane again doesn't restart its clock...
        t.view(Some("%2"), start + DWELL * 5);
        assert!(t
            .viewing
            .as_ref()
            .is_some_and(|(_, s)| *s == start + DWELL / 2));
        // ...and with no preview showing, nothing is being looked at.
        t.view(None, start + DWELL * 6);
        assert!(t.viewing.is_none());
    }

    #[test]
    fn a_pane_missing_from_a_read_keeps_what_it_had() {
        let old = fingerprint("80", "24", "a");
        let mut t = tracker(vec![info("%1", "", false)]);
        t.apply(&[snap("%1", &old, "b")]);
        t.apply(&[]);
        assert!(t.find("%1").unwrap().changed());
    }

    #[test]
    fn agent_state_comes_from_its_hooks_then_the_screen() {
        assert_eq!(agent_state("working", false, ""), Some(AgentState::Working));
        assert_eq!(
            agent_state(" waiting\n", false, ""),
            Some(AgentState::Waiting)
        );
        assert_eq!(agent_state("done", true, ""), Some(AgentState::Done));
        assert_eq!(agent_state("", false, ""), None);

        let busy =
            "> earlier answer\n\n✻ Cogitating… (12s · esc to interrupt)\n\n╭──╮\n│ > │\n╰──╯\n";
        // Claude Code without hooks: its status line says.
        assert_eq!(agent_state("", true, busy), Some(AgentState::Working));
        assert_eq!(agent_state("", true, "> done\n╭──╮\n│ > │\n╰──╯"), None);
        // ...but only for Claude Code, and the hooks win.
        assert_eq!(agent_state("", false, busy), None);
        assert_eq!(agent_state("done", true, busy), Some(AgentState::Done));
        // The phrase quoted in the conversation is not the status line.
        assert_eq!(
            agent_state("", true, "it shows (esc to interrupt) while working\n> "),
            None
        );
        // Nor is a status line scrolled far up.
        let far = format!(
            "✻ Working… (esc to interrupt)\n{}",
            "line\n".repeat(STATUS_ROWS)
        );
        assert_eq!(agent_state("", true, &far), None);
    }

    #[test]
    fn states_need_an_agent_and_working_hides_the_mark() {
        let old = fingerprint("80", "24", "a");
        let mut t = tracker(vec![info("%1", "claude", false), info("%2", "", false)]);
        let mut busy = snap("%1", &old, "b");
        busy.state = "working".into();
        let mut stale = snap("%2", &old, "b");
        stale.state = "working".into();
        t.apply(&[busy, stale]);

        let (claude, shell) = (t.find("%1").unwrap(), t.find("%2").unwrap());
        assert_eq!(claude.state, Some(AgentState::Working));
        assert!(!claude.changed(), "working says it already");
        // Left behind on a plain shell, the state is ignored.
        assert_eq!(shell.state, None);
        assert!(shell.changed());
    }
}
