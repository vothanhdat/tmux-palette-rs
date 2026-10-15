#!/usr/bin/env python3
"""Soundtrack for the vertical short: an original music bed plus sound effects
synced to the picture. Everything is synthesized here (numpy only); there are
no samples or recordings, so the result is free to post anywhere.

    audio.py plan   CLIP...        print each scene's length, snapped to the beat
    audio.py render OUT.wav CLIP...

`plan` prints the scene lengths make.sh cuts the clips to, then the closing
card's length and the crossfade into it, all in seconds. Every cut lands on a
beat, the first on a bar line (where the beat drops) and the last on one too
(where the closing chord hits).

`render` writes the mix, loudness-normalized for the platforms (-14 LUFS).
The music bed is chords, bass, drums and plucks at 108 BPM. On top of it:

* a key click for every keystroke in the scene's tape (demo/short/<clip>.tape),
  moved onto the frame where the screen actually changes, since a recording
  drifts a little from its script;
* a whoosh into each cut, and a riser into the closing card.
"""
import itertools
import json
import math
import re
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

import numpy as np

SR = 48000
BPM = 108
BEAT = 60 / BPM
MAX_TRIM = 0.35  # seconds a scene's (idle) tail may lose to land on a beat
MAX_PAD = 1.2  # ...or be held on its last frame
OUTRO_BEATS = 8
XFADE_BEATS = 1
LOUDNESS = -14  # LUFS, what YouTube and TikTok normalize to

REPO = Path(__file__).resolve().parents[2]
rng = np.random.default_rng(108)


# --- timing ------------------------------------------------------------------


def duration(clip):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", clip],
        capture_output=True, text=True, check=True,
    )
    return float(out.stdout)


def plan(durations):
    """Beats per scene: each within MAX_TRIM/MAX_PAD of its clip, the first a
    whole number of bars, and the total too; the least nudging overall wins."""
    options = []
    for i, d in enumerate(durations):
        near = round(d / BEAT)
        fits = [b for b in range(max(1, near - 2), near + 4)
                if d - MAX_TRIM <= b * BEAT <= d + MAX_PAD and (i > 0 or b % 4 == 0)]
        options.append(fits or [4 * math.ceil(d / BEAT / 4) if i == 0 else math.ceil(d / BEAT)])
    best = None
    for combo in itertools.product(*options):
        if sum(combo) % 4:
            continue
        cost = sum(abs(b * BEAT - d) for b, d in zip(combo, durations))
        if best is None or cost < best[0]:
            best = (cost, list(combo))
    if best is None:  # no combination fits: hold the last scene to the bar
        combo = [o[0] for o in options]
        combo[-1] += -sum(combo) % 4
        return combo
    return best[1]


def parse_time(s):
    return float(s[:-2]) / 1000 if s.endswith("ms") else float(s.rstrip("s"))


KEY = re.compile(
    r"^(Ctrl\+\S+|Alt\+\S+|Enter|Tab|Backspace|Escape|Space|Up|Down|Left|Right|Delete)"
    r"(?:@(\S+))?(?:\s+(\d+))?$"
)


def tape_keys(path, st=None):
    """(seconds into the recording, kind) of every key the tape presses on
    camera, and the recording's scripted length. VHS waits the typing speed
    (or @time) after each key, and records nothing while hidden."""
    st = st if st is not None else {"t": 0.0, "shown": True, "speed": 0.1, "keys": []}
    for raw in Path(path).read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        cmd, _, arg = line.partition(" ")
        if cmd != "Type":
            arg = arg.split(" #")[0].strip()
        if cmd == "Source":
            tape_keys(REPO / arg, st)
        elif cmd == "Set" and arg.startswith("TypingSpeed"):
            st["speed"] = parse_time(arg.split()[1])
        elif cmd in ("Hide", "Show"):
            st["shown"] = cmd == "Show"
        elif cmd == "Sleep" and st["shown"]:
            st["t"] += parse_time(arg)
        elif cmd.startswith("Type"):
            speed = parse_time(cmd[5:]) if cmd.startswith("Type@") else st["speed"]
            text = json.loads(arg) if arg.startswith('"') else arg.strip("`'")
            for ch in text:
                if st["shown"]:
                    st["keys"].append((st["t"], "space" if ch == " " else "key"))
                    st["t"] += speed
        elif m := KEY.match(line):
            name, at, count = m.groups()
            for _ in range(int(count or 1)):
                if st["shown"]:
                    kind = {"Enter": "enter", "Tab": "space", "Space": "space"}.get(name, "key")
                    st["keys"].append((st["t"], kind))
                    st["t"] += parse_time(at) if at else st["speed"]
    return st["keys"], st["t"]


def screen_changes(clip):
    """Times of the frames where the picture changes."""
    w, h = 258, 280
    raw = subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-i", clip, "-vf",
         f"scale={w}:{h}:flags=area,format=gray", "-f", "rawvideo", "-"],
        capture_output=True, check=True,
    ).stdout
    rate = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v", "-show_entries",
         "stream=r_frame_rate", "-of", "csv=p=0", clip],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    num, _, den = rate.partition("/")
    fps = float(num) / float(den or 1)
    frames = np.frombuffer(raw, np.uint8).reshape(-1, h, w).astype(np.int16)
    changed = (np.abs(np.diff(frames, axis=0)) > 12).mean(axis=(1, 2))
    return [(i + 1) / fps for i in np.nonzero(changed > 0.0003)[0]]


def align(script, changes, scale, reach=0.6):
    """Match scripted key times to screen changes, in order. A match costs how
    far its gap from the previous match strays from the script's gap, relative
    to that gap (a long wait drifts more than a burst of typing); leaving a key
    unmatched (one that changes nothing on screen, like the tmux prefix) costs
    1; leaving a change unmatched (a log scrolling) is free. Returns
    {key index: change index}."""

    def stray(actual, planned):
        return ((actual - planned) / (0.05 + 0.15 * abs(planned))) ** 2

    best = {}
    for i, s in enumerate(script):
        for j, c in enumerate(changes):
            if abs(c - scale * s) > reach:
                continue
            cost, prev = i + 0.5 * stray(c, scale * s), None
            for (pi, pj), (pc, _) in best.items():
                if pi < i and pj < j:
                    step = pc + (i - pi - 1) + stray(c - changes[pj], scale * (s - script[pi]))
                    if step < cost:
                        cost, prev = step, (pi, pj)
            best[(i, j)] = (cost, prev)
    if not best:
        return {}
    end = min(best, key=lambda k: best[k][0] + len(script) - 1 - k[0])
    pairs = {}
    while end:
        pairs[end[0]] = end[1]
        end = best[end][1]
    return pairs


def synced_keys(tape, clip):
    """The tape's keys, each on the frame where the screen shows it; keys that
    show nothing keep their scripted gap to their neighbours."""
    keys, scripted = tape_keys(tape)
    changes = screen_changes(clip)
    scale = duration(clip) / scripted if scripted else 1.0
    script = [t for t, _ in keys]
    pairs = align(script, changes, scale)
    times = []
    for i, s in enumerate(script):
        if i in pairs:
            times.append(changes[pairs[i]] - 0.03)  # the press shows a frame or so later
            continue
        before = [k for k in pairs if k < i]
        after = [k for k in pairs if k > i]
        if before:
            k = max(before)
            t = changes[pairs[k]] - 0.03 + scale * (s - script[k])
        elif after:
            k = min(after)
            t = changes[pairs[k]] - 0.03 - scale * (script[k] - s)
        else:
            t = scale * s
        times.append(t)
    return [(t, kind) for t, (_, kind) in zip(times, keys)]


# --- dsp ---------------------------------------------------------------------


def spectral(x, fn, pad=4096):
    """Zero-phase filter: multiply the spectrum by fn(freqs)."""
    n = len(x)
    y = np.fft.rfft(np.concatenate([np.zeros(pad), x, np.zeros(pad)]))
    f = np.fft.rfftfreq(n + 2 * pad, 1 / SR)
    return np.fft.irfft(y * fn(f), n + 2 * pad)[pad:pad + n]


def lp(fc, order=2):
    return lambda f: 1 / np.sqrt(1 + (f / fc) ** (2 * order))


def hp(fc, order=2):
    return lambda f: 1 / np.sqrt(1 + (fc / np.maximum(f, 1e-3)) ** (2 * order))


def bp(lo, hi, order=2):
    return lambda f: lp(hi, order)(f) * hp(lo, order)(f)


def tv_filter(x, gain):
    """Time-varying filter (STFT): gain(seconds, freqs) -> magnitudes."""
    win_n, hop = 2048, 512
    win = np.hanning(win_n + 1)[:-1]
    padded = np.concatenate([np.zeros(win_n), x, np.zeros(win_n)])
    out = np.zeros_like(padded)
    norm = np.zeros_like(padded)
    f = np.fft.rfftfreq(win_n, 1 / SR)
    for s in range(0, len(padded) - win_n, hop):
        g = gain((s + win_n / 2 - win_n) / SR, f)
        out[s:s + win_n] += np.fft.irfft(np.fft.rfft(padded[s:s + win_n] * win) * g) * win
        norm[s:s + win_n] += win**2
    return (out / np.maximum(norm, 1e-6))[win_n:win_n + len(x)]


def sweep(x, center_at, width=1.2):
    """Band-pass whose center moves: center_at(seconds) -> Hz."""
    return tv_filter(x, lambda s, f: np.exp(-0.5 * (np.log2(np.maximum(f, 1) / center_at(s)) / width) ** 2))


def tt(seconds):
    return np.arange(int(seconds * SR)) / SR


def mtof(m):
    return 440.0 * 2 ** ((m - 69) / 12)


def fade(x, attack=0.004, release=0.03):
    n = len(x)
    env = np.ones(n)
    a, r = min(int(attack * SR), n), min(int(release * SR), n)
    env[:a] = np.linspace(0, 1, a)
    if r:
        env[n - r:] *= np.linspace(1, 0, r)
    return x * env


class Bus:
    def __init__(self, n):
        self.x = np.zeros((2, n))

    def add(self, sig, t, gain=1.0, pan=0.0):
        i = int(round(t * SR))
        if i >= self.x.shape[1]:
            return
        sig = np.atleast_2d(sig)
        if sig.shape[0] == 1:
            a = (pan + 1) * math.pi / 4
            sig = np.vstack([sig[0] * math.cos(a), sig[0] * math.sin(a)]) * math.sqrt(2)
        if i < 0:
            sig, i = sig[:, -i:], 0
        m = min(sig.shape[1], self.x.shape[1] - i)
        self.x[:, i:i + m] += gain * sig[:, :m]


# --- instruments -------------------------------------------------------------


def kick():
    t = tt(0.5)
    freq = 46 + 115 * np.exp(-t / 0.032)
    body = np.sin(2 * np.pi * np.cumsum(freq) / SR) * np.exp(-t / 0.12)
    click = spectral(rng.standard_normal(len(t)) * np.exp(-t / 0.003), bp(1500, 7000)) * 0.35
    return fade(np.tanh(1.8 * (body + click)), 0.0005, 0.05)


def clap():
    t = tt(0.4)
    env = sum(np.exp(-np.maximum(t - d, 0) / 0.006) * (t >= d) for d in (0, 0.011, 0.023))
    env = env + 0.7 * np.exp(-np.maximum(t - 0.03, 0) / 0.11) * (t >= 0.03)
    noise = spectral(rng.standard_normal(len(t)) * env, bp(900, 5000))
    body = np.sin(2 * np.pi * 205 * t) * np.exp(-t / 0.045) * 0.35
    return fade(noise / np.abs(noise).max() + body)


def metallic(seconds):
    t = tt(seconds)
    sq = sum(np.sign(np.sin(2 * np.pi * f * 1.6 * t)) for f in (205.3, 304.4, 369.6, 522.7, 540, 800))
    return sq / 6 + 0.8 * rng.standard_normal(len(t))


def hat(open_=False):
    decay, n = (0.16, 0.45) if open_ else (0.028, 0.12)
    t = tt(n)
    x = spectral(metallic(n) * np.exp(-t / decay), hp(7000, 3))
    return fade(x / np.abs(x).max(), 0.001, 0.02)


def crash():
    t = tt(2.6)
    x = spectral(metallic(2.6) * np.exp(-t / 0.75), hp(4200, 2))
    return fade(x / np.abs(x).max(), 0.002, 0.3)


def bass(m, beats):
    t = tt(beats * BEAT + 0.04)
    f = mtof(m)
    x = np.sin(2 * np.pi * f * t) + 0.3 * np.sin(4 * np.pi * f * t)
    x = np.tanh(2.4 * x) / np.tanh(2.4)  # harmonics, so phones can hear it
    return fade(spectral(x, lp(1100)), 0.005, 0.04)


def ep(m, beats, vel=1.0):
    """FM electric piano."""
    t = tt(beats * BEAT + 1.3)
    f = mtof(m)
    index = (1.4 * vel + 0.3) * np.exp(-t / 0.3) + 0.12
    x = np.sin(2 * np.pi * f * t + index * np.sin(2 * np.pi * f * t))
    x += 0.15 * np.sin(2 * np.pi * 7 * f * t) * np.exp(-t / 0.05) * vel  # tine
    held = beats * BEAT
    env = np.exp(-t / 1.6) * np.where(t > held, np.exp(-(t - held) / 0.22), 1)
    return fade(x * env * vel, 0.004, 0.05)


def pad(notes, beats):
    """Detuned, soft saw stack, built from its harmonics (no aliasing)."""
    dur = beats * BEAT
    t = tt(dur + 0.9)
    out = np.zeros((2, len(t)))
    for m in notes:
        for ch, cents in ((0, -7), (1, 7), (0, 2), (1, -2)):
            f = mtof(m) * 2 ** (cents / 1200)
            ks = np.arange(1, int(2600 / f) + 1)
            amp = lp(1500)(ks * f) / ks
            phase = rng.uniform(0, 2 * np.pi, len(ks))
            out[ch] += (amp[:, None] * np.sin(2 * np.pi * f * ks[:, None] * t + phase[:, None])).sum(0)
    rise = np.minimum(t / 0.45, 1) ** 2
    fall = np.where(t > dur, np.exp(-(t - dur) / 0.25), 1)
    return out * rise * fall / (len(notes) * 2)


def pluck(m, seconds=1.1, bright=4500):
    """Karplus-Strong string, computed a period at a time."""
    f = mtof(m)
    p = max(2, int(round(SR / f)))
    n = int(seconds * SR)
    blocks = n // p + 2
    damp = 0.001 ** (1 / (1.3 * f))  # about -60 dB after 1.3 s
    y = np.zeros((blocks + 1) * p + 1)
    y[1:p + 1] = spectral(rng.uniform(-1, 1, p), lp(bright))
    for b in range(1, blocks):
        s = 1 + b * p
        y[s:s + p] = damp * 0.5 * (y[s - p:s] + y[s - p - 1:s - 1])
    x = y[1:n + 1]
    return fade(x / (np.abs(x).max() + 1e-9), 0.001, 0.06)


def bell(m):
    t = tt(2.4)
    f = mtof(m)
    x = sum(a * np.sin(2 * np.pi * f * r * t) * np.exp(-t / d)
            for r, a, d in ((1, 1, 1.4), (2.756, 0.45, 0.6), (5.404, 0.22, 0.3), (8.933, 0.12, 0.15)))
    return fade(x, 0.002, 0.2)


def keyclick(kind):
    t = tt(0.11)
    low = {"enter": 150, "space": 170}.get(kind, rng.uniform(210, 270))
    tick = spectral(rng.standard_normal(len(t)) * np.exp(-t / 0.0022), bp(2500, 9000))
    thock = np.sin(2 * np.pi * low * t) * np.exp(-t / 0.016)
    knock = spectral(rng.standard_normal(len(t)) * np.exp(-t / 0.01), bp(400, 1600))
    x = 0.55 * tick / np.abs(tick).max() + 0.5 * thock + 0.35 * knock / np.abs(knock).max()
    up = int(rng.uniform(0.055, 0.075) * SR)  # the key coming back up
    x[up:] += 0.25 * (tick / np.abs(tick).max())[: len(x) - up] * 0.6
    return fade(x, 0.0003, 0.02) * {"enter": 1.25, "space": 1.1}.get(kind, rng.uniform(0.8, 1.0))


def whoosh(seconds=0.55, peak=0.42):
    t = tt(seconds)
    noise = rng.standard_normal(len(t))
    x = sweep(noise, lambda s: 400 * 2 ** (4 * min(max(s, 0), peak) / peak), width=0.7)
    env = np.where(t < peak, np.sin(np.pi / 2 * t / peak) ** 3, np.exp(-(t - peak) / 0.06))
    x = x * env
    return np.vstack([x * np.linspace(1.2, 0.6, len(t)), x * np.linspace(0.6, 1.2, len(t))])


def riser(seconds):
    t = tt(seconds)
    x = sweep(rng.standard_normal(len(t)), lambda s: 300 * 2 ** (5 * max(s, 0) / seconds), width=0.9)
    return x * (t / seconds) ** 2


def reverb(x, seconds=2.6, rt60=2.0):
    t = tt(seconds)
    decay = 10 ** (-3 * t / rt60)
    out = np.zeros((2, x.shape[1] + len(t) - 1))
    size = 1 << (out.shape[1] - 1).bit_length()
    for ch in range(2):
        ir = spectral(rng.standard_normal(len(t)) * decay, bp(200, 5500))
        ir[: int(0.018 * SR)] = 0  # pre-delay
        ir /= np.sqrt((ir**2).sum())
        out[ch] = np.fft.irfft(np.fft.rfft(x[ch], size) * np.fft.rfft(ir, size), size)[: out.shape[1]]
    return out[:, : x.shape[1]]


def ping_pong(x, delay=0.75 * BEAT, taps=4, feedback=0.42):
    out = x.copy()
    d = int(delay * SR)
    for k in range(1, taps + 1):
        src = spectral(x[0] + x[1], lp(3500 / k)) * feedback**k / 2
        ch = (k + 1) % 2
        out[ch, d * k:] += src[: x.shape[1] - d * k]
    return out


# --- score -------------------------------------------------------------------

# (bass, chord voicing, arpeggio notes); key of C.
CHORDS = {
    "Fmaj7": (41, [53, 57, 60, 64], [65, 69, 72, 76]),
    "G6": (43, [55, 59, 62, 64], [67, 71, 74, 76]),
    "Em7": (40, [52, 55, 59, 62], [64, 67, 71, 74]),
    "Am7": (45, [55, 57, 60, 64], [69, 72, 76, 79]),
    "G7sus": (43, [53, 55, 60, 62], [67, 72, 74, 77]),
    "Cmaj9": (36, [48, 52, 55, 59, 62], [72, 76, 79, 83, 86]),
}
INTRO = ["Am7", "G6"]
LOOP = ["Fmaj7", "G6", "Em7", "Am7"]
ARP = [0, 1, 2, 3, 2, 1, 2, 3]  # eighth notes over a bar


def score(scene_beats, keys):
    """keys: [(seconds, kind)] over the whole video."""
    drop, end = scene_beats[0], sum(scene_beats)
    total = end + OUTRO_BEATS - XFADE_BEATS
    n = int(total * BEAT * SR)
    music, sends, intro, fx = Bus(n), Bus(n), Bus(n), Bus(n)
    kicks = []
    b = lambda beat: beat * BEAT  # noqa: E731

    def chord_at(bar):
        start = bar * 4
        if start < drop:
            return INTRO[bar % len(INTRO)]
        if start >= end:
            return "Cmaj9"
        if start == end - 4:
            return "G7sus"
        return LOOP[(start - drop) // 4 % len(LOOP)]

    k, c, ch, co, cr = kick(), clap(), hat(), hat(True), crash()
    for bar in range(end // 4):
        root, voicing, arp = CHORDS[chord_at(bar)]
        s = bar * 4
        if s < drop:  # intro: soft keys and plucks, filter opening up
            intro.add(pad(voicing, 4), b(s), 0.7)
            for m in voicing:
                intro.add(ep(m, 3.5, 0.8), b(s), 0.22, rng.uniform(-0.3, 0.3))
            for i, step in enumerate(ARP):
                if bar or i >= 4:
                    intro.add(pluck(arp[step] + 12 * (i % 4 == 3)), b(s + i / 2), 0.3, (-0.4, 0.4)[i % 2])
                if bar:
                    intro.add(ch, b(s + i / 2 + (0.035 if i % 2 else 0)), 0.16 if i % 2 else 0.1, 0.3)
            continue
        lift = s >= drop + 16  # second phrase onwards: busier hats
        chord = pad(voicing, 4)
        music.add(chord, b(s), 0.55)
        sends.add(chord, b(s), 0.3)
        for at, beats, vel in ((0, 1.4, 1.0), (1.5, 0.6, 0.7), (3, 0.7, 0.6)):
            for m in voicing:
                note = ep(m, beats, vel)
                music.add(note, b(s + at), 0.16, rng.uniform(-0.35, 0.35))
                sends.add(note, b(s + at), 0.06)
        for at, beats, m in ((0, 0.8, root), (2, 0.45, root), (2.5, 0.8, root), (3.5, 0.4, root + 7)):
            music.add(bass(m, beats), b(s + at), 0.3)
        for i, step in enumerate(ARP):
            note = pluck(arp[step] + (12 if lift and i % 4 == 3 else 0))
            music.add(note, b(s + i / 2), 0.25, (-0.45, 0.45)[i % 2])
            sends.add(note, b(s + i / 2), 0.1)
        last = s == end - 4
        for at in (0, 2) + ((2.5,) if bar % 2 else ()):
            kicks.append(b(s + at))
            music.add(k, b(s + at), 0.7)
        for at in (1, 3):
            music.add(c, b(s + at), 0.5, 0.05)
            sends.add(c, b(s + at), 0.18)
        for i in range(8):
            at = i / 2 + (0.035 if i % 2 else 0)  # a little swing
            music.add(ch, b(s + at), 0.2 if i % 2 else 0.12, 0.3)
            if lift:
                music.add(ch, b(s + at + 0.25), 0.07, -0.3)
        if lift and not last:
            music.add(co, b(s + 3.5), 0.14, -0.2)
        if last:  # a clap roll into the closing card
            for i in range(4):
                music.add(c, b(s + 3 + i / 4), 0.12 + 0.07 * i, 0.05)

    # riser into the drop, crash on it
    fx.add(riser(b(2)), b(drop - 2), 0.22)
    music.add(cr, b(drop), 0.32)

    # closing card: the chord, a bell arpeggio, one last hit
    root, voicing, arp = CHORDS["Cmaj9"]
    fx.add(riser(b(1.6)), b(end - 1.6), 0.2)
    music.add(k, b(end), 0.7)
    music.add(cr, b(end), 0.32)
    kicks.append(b(end))
    music.add(bass(root, 3), b(end), 0.32)
    for m in voicing:
        note = ep(m, OUTRO_BEATS - 2, 1.0)
        music.add(note, b(end), 0.15, rng.uniform(-0.4, 0.4))
        sends.add(note, b(end), 0.1)
    chord = pad(voicing, OUTRO_BEATS - 2)
    music.add(chord, b(end), 0.6)
    sends.add(chord, b(end), 0.4)
    for i, m in enumerate([84, 88, 91, 95, 98]):
        note = bell(m)
        music.add(note, b(end + 0.5 * i), 0.09, (-0.5, 0.5)[i % 2])
        sends.add(note, b(end + 0.5 * i), 0.12)

    # a whoosh into each cut
    cut = 0
    for beats in scene_beats[:-1]:
        cut += beats
        w = whoosh()
        fx.add(w, b(cut) - 0.42, 0.4)
        sends.add(w, b(cut) - 0.42, 0.05)

    # key clicks
    for t, kind in keys:
        fx.add(keyclick(kind), t, 0.7, rng.uniform(-0.15, 0.15))

    # intro filter: muffled, opening up into the drop
    def intro_cut(s):
        return 900 * 2 ** (3.2 * min(max(s / b(drop), 0), 1) ** 2)

    for chn in range(2):
        intro.x[chn] = tv_filter(intro.x[chn], lambda s, f: lp(intro_cut(s))(f))
    music.x += intro.x

    # sidechain: everything but the drums ducks under the kick
    t = np.arange(n) / SR
    duck = np.ones(n)
    for kt in kicks:
        i, j = int(kt * SR), min(n, int((kt + 0.8) * SR))
        duck[i:j] = np.minimum(duck[i:j], 1 - 0.38 * np.exp(-(t[i:j] - kt) / 0.14))
    music.x = music.x * duck
    wet = reverb(ping_pong(sends.x))
    mix = music.x + 0.55 * wet + fx.x
    for chn in range(2):
        mix[chn] = spectral(mix[chn], lambda f: hp(28)(f) * (1 + 0.5 / (1 + (4000 / np.maximum(f, 1)) ** 2)))
    tail = int(1.6 * SR)
    mix[:, -tail:] *= np.linspace(1, 0, tail) ** 1.5
    mix = np.tanh(1.1 * mix / np.abs(mix).max()) * 0.9
    return mix, {"music": music.x, "reverb": 0.55 * wet, "fx": fx.x}


def write_wav(path, x):
    pcm = (np.clip(x.T, -1, 1) * 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def loudnorm(src, dst):
    target = f"I={LOUDNESS}:TP=-1.5:LRA=11"
    probe = subprocess.run(
        ["ffmpeg", "-hide_banner", "-i", src, "-af", f"loudnorm={target}:print_format=json",
         "-f", "null", "-"], capture_output=True, text=True, check=True,
    ).stderr
    m = json.loads(probe[probe.rindex("{"):])
    measured = (f"measured_I={m['input_i']}:measured_TP={m['input_tp']}:measured_LRA={m['input_lra']}"
                f":measured_thresh={m['input_thresh']}:offset={m['target_offset']}:linear=true")
    subprocess.run(
        ["ffmpeg", "-y", "-loglevel", "error", "-i", src, "-af", f"loudnorm={target}:{measured}",
         "-ar", str(SR), dst], check=True,
    )


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("plan", "render"):
        sys.exit(__doc__)
    if sys.argv[1] == "plan":
        beats = plan([duration(c) for c in sys.argv[2:]])
        print(*[f"{x * BEAT:.4f}" for x in beats + [OUTRO_BEATS, XFADE_BEATS]])
        return
    out, clips = sys.argv[2], sys.argv[3:]
    beats = plan([duration(c) for c in clips])
    keys, start = [], 0.0
    for clip, scene in zip(clips, beats):
        tape = REPO / "demo/short" / (Path(clip).stem + ".tape")
        length = scene * BEAT
        keys += [(start + t, kind) for t, kind in synced_keys(tape, clip) if t < length - 0.05]
        start += length
    mix, _ = score(beats, keys)
    with tempfile.TemporaryDirectory() as tmp:
        raw = Path(tmp) / "raw.wav"
        write_wav(raw, mix)
        loudnorm(str(raw), out)
    print(f"{out}: {mix.shape[1] / SR:.2f}s, {len(keys)} key clicks")


if __name__ == "__main__":
    main()
