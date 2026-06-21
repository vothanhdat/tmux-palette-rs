//! Fuzzy matching and ranking — port of `src/fuzzy.ts`.

use crate::text::auto_alias;
use crate::types::Item;

fn is_boundary(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '_' | '·' | '.' | '/' | ':')
}

fn find_subslice(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

fn exact_match_score(hs: &[char], nd: &[char]) -> i64 {
    match find_subslice(hs, nd) {
        None => 0,
        Some(idx) => {
            let at_boundary = idx == 0 || is_boundary(hs[idx - 1]);
            10000 + if at_boundary { 5000 } else { 0 } - idx as i64
        }
    }
}

fn char_bonus(at_boundary: bool, consecutive: bool) -> i64 {
    if at_boundary {
        50
    } else if consecutive {
        20
    } else {
        5
    }
}

fn subsequence_score(hs: &[char], nd: &[char]) -> i64 {
    let mut score = 0i64;
    let mut h = 0usize;
    let mut prev: i64 = -2;
    for &target in nd {
        while h < hs.len() && hs[h] != target {
            h += 1;
        }
        if h >= hs.len() {
            return 0;
        }
        let at_boundary = h == 0 || is_boundary(hs[h - 1]);
        let consecutive = h as i64 == prev + 1;
        score += char_bonus(at_boundary, consecutive);
        prev = h as i64;
        h += 1;
    }
    score.max(1)
}

fn fuzzy_score(haystack: &str, needle: &str) -> i64 {
    if needle.is_empty() {
        return 1;
    }
    let hs: Vec<char> = haystack.to_lowercase().chars().collect();
    let nd: Vec<char> = needle.to_lowercase().chars().collect();
    let exact = exact_match_score(&hs, &nd);
    if exact > 0 {
        return exact;
    }
    subsequence_score(&hs, &nd)
}

/// Every query part must match; scores sum. Returns 0 if any part fails.
pub fn multi_fuzzy_score(haystack: &str, parts: &[String]) -> i64 {
    let mut total = 0;
    for p in parts {
        let s = fuzzy_score(haystack, p);
        if s == 0 {
            return 0;
        }
        total += s;
    }
    total
}

fn build_item_haystack(item: &Item) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if !item.title.is_empty() {
        parts.push(&item.title);
    }
    if let Some(d) = &item.description {
        if !d.is_empty() {
            parts.push(d);
        }
    }
    if let Some(c) = &item.category {
        if !c.is_empty() {
            parts.push(c);
        }
    }
    if let Some(s) = &item.shortcut {
        if !s.is_empty() {
            parts.push(s);
        }
    }
    if let Some(al) = &item.aliases {
        for a in al {
            if !a.is_empty() {
                parts.push(a);
            }
        }
    }
    let auto = auto_alias(&item.title);
    let mut joined = parts.join(" ");
    if let Some(a) = &auto {
        if !joined.is_empty() {
            joined.push(' ');
        }
        joined.push_str(a);
    }
    joined
}

/// Boost when the query exactly matches an item alias (auto-initials or
/// user-defined). Must outrank any fuzzy score the haystack can produce.
const ALIAS_EXACT_BOOST: i64 = 100000;

fn alias_exact_boost(item: &Item, parts: &[String]) -> i64 {
    if parts.len() != 1 {
        return 0;
    }
    let q = parts[0].to_lowercase();
    if let Some(auto) = auto_alias(&item.title) {
        if auto == q {
            return ALIAS_EXACT_BOOST;
        }
    }
    if let Some(al) = &item.aliases {
        for a in al {
            if a.to_lowercase() == q {
                return ALIAS_EXACT_BOOST;
            }
        }
    }
    0
}

pub fn default_filter(items: &[Item], needle: &str) -> Vec<Item> {
    let parts: Vec<String> = needle.split_whitespace().map(|s| s.to_string()).collect();
    let mut scored: Vec<(&Item, i64)> = items
        .iter()
        .map(|c| {
            (
                c,
                multi_fuzzy_score(&build_item_haystack(c), &parts) + alias_exact_boost(c, &parts),
            )
        })
        .filter(|x| x.1 > 0)
        .collect();
    // Stable sort keeps original order for equal scores (matches JS Array.sort).
    scored.sort_by_key(|x| std::cmp::Reverse(x.1));
    scored.into_iter().map(|x| x.0.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Action, Item};

    fn item(title: &str, category: Option<&str>, aliases: Option<Vec<&str>>) -> Item {
        Item {
            title: title.to_string(),
            category: category.map(|s| s.to_string()),
            aliases: aliases.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            action: Action::Shell(":".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn requires_every_query_part_to_match() {
        let p1 = vec!["split".to_string(), "pane".to_string()];
        assert!(multi_fuzzy_score("split horizontal pane", &p1) > 0);
        let p2 = vec!["split".to_string(), "window".to_string()];
        assert_eq!(multi_fuzzy_score("split horizontal pane", &p2), 0);
    }

    #[test]
    fn matches_title_initials_through_auto_aliases() {
        let items = vec![
            item("Split Horizontal", Some("Panes"), None),
            item("New Window", Some("Windows"), None),
            item("Choose Session", None, Some(vec!["sessions"])),
        ];
        let got: Vec<String> = default_filter(&items, "sh")
            .iter()
            .map(|i| i.title.clone())
            .collect();
        assert_eq!(got, vec!["Split Horizontal".to_string()]);
    }

    #[test]
    fn matches_explicit_aliases() {
        let items = vec![
            item("Split Horizontal", Some("Panes"), None),
            item("New Window", Some("Windows"), None),
            item("Choose Session", None, Some(vec!["sessions"])),
        ];
        let got: Vec<String> = default_filter(&items, "sessions")
            .iter()
            .map(|i| i.title.clone())
            .collect();
        assert_eq!(got, vec!["Choose Session".to_string()]);
    }

    #[test]
    fn auto_alias_outranks_substring_matches() {
        let items = vec![
            item("Detach", Some("Sessions"), None),
            item("New Session", Some("Sessions"), None),
            item("Next Session", Some("Sessions"), None),
        ];
        let ranked: Vec<String> = default_filter(&items, "ns")
            .iter()
            .map(|i| i.title.clone())
            .collect();
        assert_eq!(ranked[0], "New Session");
        assert_eq!(ranked[1], "Next Session");
    }
}
