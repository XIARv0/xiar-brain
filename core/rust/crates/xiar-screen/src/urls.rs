// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Web addresses found in text that OCR read off a screenshot. Port of the desktop-agent's `ocr_url_extractor.py`
//! (desktop-agent@f7e75fc3), minus the OCR call itself: that is an OCR plug-in's job ([`crate::pipeline::Ocr`]); what is done
//! with its text is here, and is replayed against the original's results (`tests/golden/ocr_urls.json`).
//!
//! Only addresses are kept; the OCR text is dropped, so nothing the user was reading is stored because of this.

use regex::Regex;
use std::sync::LazyLock;

// `[^\s<>"{}|\^`\[\]]` as upstream writes it
static FULL_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)https?://[^\s<>"{}|\^`\[\]]+"#).unwrap());
static DOMAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,}(?:/[^\s<>"{}|\^`\[\]]*)?"#).unwrap());

const NOT_SITES: &[&str] = &["etc.", "e.g.", "i.e.", "vs.", "Dr.", "Mr.", "Mrs.", "Ms.", "Inc.", "Ltd.", "Co."];

/// Common false positives in what the domain pattern finds ("e.g.", "file.txt" is NOT filtered, as upstream).
pub fn is_likely_url(domain: &str) -> bool {
    if domain.chars().count() < 4 || !domain.contains('.') {
        return false;
    }
    let lower = domain.to_lowercase();
    if NOT_SITES.iter().any(|fp| lower.starts_with(&fp.to_lowercase())) {
        return false;
    }
    let parts: Vec<&str> = domain.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    let tld = parts[parts.len() - 1].split('/').next().unwrap_or("");
    if tld.chars().count() < 2 || !tld.chars().all(char::is_alphabetic) {
        return false;
    }
    domain.chars().any(char::is_alphabetic)
}

/// Every address in the text, in order of appearance, duplicates removed; a bare domain gets `https://`.
pub fn extract_urls(text: &str) -> Vec<String> {
    let full: Vec<&str> = FULL_URL.find_iter(text).map(|m| m.as_str()).collect();
    let mut out: Vec<String> = full.iter().map(|s| s.to_string()).collect();
    for m in DOMAIN.find_iter(text) {
        let domain = m.as_str();
        if full.iter().any(|u| u.contains(domain)) {
            continue;
        }
        if is_likely_url(domain) {
            out.push(format!("https://{domain}"));
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|u| seen.insert(u.clone()));
    out
}

/// An address as it will be used: the punctuation of the sentence it was printed in is not part of it (upstream keeps it).
/// A closing bracket stays when the address opened one ("https://en.wikipedia.org/wiki/Rust_(language)").
pub fn tidy(url: &str) -> String {
    let mut u = url;
    while let Some(last) = u.chars().last() {
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => true,
            ')' => u.matches(')').count() > u.matches('(').count(),
            ']' => u.matches(']').count() > u.matches('[').count(),
            '}' => true,
            _ => false,
        };
        if !drop {
            break;
        }
        u = &u[..u.len() - last.len_utf8()];
    }
    u.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn golden() -> Value {
        serde_json::from_str(include_str!("../tests/golden/ocr_urls.json")).unwrap()
    }

    #[test]
    fn extraction_equals_the_pinned_upstream_on_every_recorded_text() {
        let g = golden();
        let mut wrong = Vec::new();
        for c in g["cases"].as_array().unwrap() {
            let text = c["text"].as_str().unwrap();
            let want: Vec<String> = c["urls"].as_array().unwrap().iter().map(|u| u.as_str().unwrap().to_string()).collect();
            let got = extract_urls(text);
            if got != want {
                wrong.push(format!("{text:?}\n   upstream {want:?}\n   port     {got:?}"));
            }
        }
        assert!(wrong.is_empty(), "differences from upstream:\n{}", wrong.join("\n"));
    }

    #[test]
    fn tidy_removes_the_sentence_punctuation_only() {
        assert_eq!(tidy("https://example.com/x),"), "https://example.com/x");
        assert_eq!(tidy("https://en.wikipedia.org/wiki/Rust_(language)"), "https://en.wikipedia.org/wiki/Rust_(language)");
        assert_eq!(tidy("https://example.com/x)."), "https://example.com/x");
        assert_eq!(tidy("https://example.com/page.html"), "https://example.com/page.html");
        assert_eq!(tidy("https://example.com."), "https://example.com");
    }

    #[test]
    fn the_false_positive_filter_equals_upstream() {
        let g = golden();
        for c in g["likely"].as_array().unwrap() {
            let d = c["domain"].as_str().unwrap();
            assert_eq!(is_likely_url(d), c["likely"].as_bool().unwrap(), "{d:?}");
        }
    }
}
