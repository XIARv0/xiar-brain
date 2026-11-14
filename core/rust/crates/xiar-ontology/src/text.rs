// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Name handling shared by the agents: the exact similarity measure of the legacy engineer (Python `difflib`
//! `SequenceMatcher.ratio`, so duplicate decisions at the 0.75 threshold come out the same) and the naming rules.

/// Longest block common to `a[alo..ahi]` and `b[blo..bhi]`, as (start in a, start in b, length). Ties go to the block that
/// starts earliest in `a`, then earliest in `b`, exactly as `SequenceMatcher.find_longest_match` does without junk.
fn longest_match(a: &[char], b: &[char], alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
    let (mut best_i, mut best_j, mut best_k) = (alo, blo, 0);
    let mut prev = vec![0usize; bhi - blo + 1];
    for (i, &ai) in a.iter().enumerate().take(ahi).skip(alo) {
        let mut cur = vec![0usize; bhi - blo + 1];
        for j in blo..bhi {
            if ai == b[j] {
                let k = prev[j - blo] + 1;
                cur[j - blo + 1] = k;
                if k > best_k {
                    best_i = i + 1 - k;
                    best_j = j + 1 - k;
                    best_k = k;
                }
            }
        }
        prev = cur;
    }
    (best_i, best_j, best_k)
}

/// `SequenceMatcher(None, a, b).ratio()` for strings shorter than 200 characters (no "popular element" heuristic needed).
pub fn sequence_ratio(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    let mut matched = 0usize;
    let mut todo = vec![(0, a.len(), 0, b.len())];
    while let Some((alo, ahi, blo, bhi)) = todo.pop() {
        if alo >= ahi || blo >= bhi {
            continue;
        }
        let (i, j, k) = longest_match(&a, &b, alo, ahi, blo, bhi);
        if k > 0 {
            matched += k;
            todo.push((alo, i, blo, j));
            todo.push((i + k, ahi, j + k, bhi));
        }
    }
    2.0 * matched as f64 / total as f64
}

fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Python `str.capitalize`: first character upper, the rest lower.
fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase(),
        None => String::new(),
    }
}

/// `Some Thing` -> `SomeThing` (CamelCase for classes).
pub fn class_name(name: &str) -> String {
    words(name).iter().map(|w| capitalize(w)).collect()
}

/// `is part of` -> `isPartOf` (camelCase for properties); nothing usable gives `relatesTo`.
pub fn property_name(name: &str) -> String {
    let w = words(name);
    match w.split_first() {
        None => "relatesTo".into(),
        Some((first, rest)) => first.to_ascii_lowercase() + &rest.iter().map(|x| capitalize(x)).collect::<String>(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ratio_matches_python_difflib() {
        // reference values computed with difflib.SequenceMatcher(None, a, b).ratio()
        for (a, b, want) in [
            ("agent", "agents", 10.0 / 11.0),
            ("task", "tasks", 8.0 / 9.0),
            ("abcd", "bcde", 0.75),
            ("", "", 1.0),
            ("a", "", 0.0),
            ("customer", "client", 2.0 / 7.0),
            ("workflow", "work", 2.0 / 3.0),
            ("personalassistant", "personalagent", 0.7333333333333333),
            ("invoice", "invoices", 14.0 / 15.0),
            ("order", "border", 10.0 / 11.0),
            ("xyz", "abc", 0.0),
            ("abxcd", "abycd", 0.8),
        ] {
            assert!((sequence_ratio(a, b) - want).abs() < 1e-9, "{a} vs {b}: {} != {want}", sequence_ratio(a, b));
        }
    }

    #[test]
    fn naming_rules_match_the_legacy_engineer() {
        assert_eq!(class_name("customer order"), "CustomerOrder");
        assert_eq!(class_name("API_key-store"), "ApiKeyStore");
        assert_eq!(class_name("  "), "");
        assert_eq!(property_name("Is Part Of"), "isPartOf");
        assert_eq!(property_name("USES"), "uses");
        assert_eq!(property_name("!!"), "relatesTo");
    }
}
