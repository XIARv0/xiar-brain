// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! What the monitor must never look at, and what must never be stored.
//!
//! * [`sensitive_window`]: windows whose title or application suggests passwords, payments, banking, health or
//!   private browsing are skipped before any pixel is analysed.
//! * [`redact`]: text derived from the screen (descriptions, transcripts) is scrubbed of card numbers, API keys,
//!   tokens, and "password: ..." style values before it is stored anywhere.
//!
//! Both are conservative on purpose: a false positive loses one observation, a false negative stores a secret.

const SENSITIVE_APPS: &[&str] = &["1password", "bitwarden", "keepass", "lastpass", "dashlane", "enpass", "proton pass", "keychain access", "credential manager", "authenticator", "authy"];
const SENSITIVE_TITLE_WORDS: &[&str] = &[
    "password", "passcode", "passphrase", "sign in", "sign-in", "log in", "login", "2fa", "two-factor", "verification code", "one-time", "otp",
    "bank", "banking", "checkout", "payment", "pay now", "credit card", "card number", "paypal", "wallet", "seed phrase", "recovery phrase", "private key",
    "incognito", "inprivate", "private browsing", "private window",
    "medical", "patient", "health record", "diagnosis", "prescription", "mychart", "therapy",
    "tax return", "social security",
];

/// True if the window must not be captured. `extra_excluded` are the user's own excluded app names (case-insensitive
/// substring match on the application name).
pub fn sensitive_window(app: &str, title: &str, extra_excluded: &[String]) -> Option<&'static str> {
    let a = app.to_lowercase();
    let t = title.to_lowercase();
    if extra_excluded.iter().any(|x| !x.trim().is_empty() && a.contains(&x.trim().to_lowercase())) {
        return Some("excluded app");
    }
    if SENSITIVE_APPS.iter().any(|s| a.contains(s) || t.contains(s)) {
        return Some("password manager or authenticator");
    }
    if SENSITIVE_TITLE_WORDS.iter().any(|w| t.contains(w)) {
        return Some("sensitive page");
    }
    None
}

fn luhn_ok(digits: &[u8]) -> bool {
    let mut sum = 0u32;
    for (i, d) in digits.iter().rev().enumerate() {
        let mut v = u32::from(*d);
        if i % 2 == 1 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    sum.is_multiple_of(10)
}

const MARK: &str = "[redacted]";

/// Scrub secrets from text. Order: labelled secrets, secrets named in a sentence, token shapes, then card numbers.
pub fn redact(text: &str) -> String {
    let labelled = redact_labelled(text);
    let described = redact_described(&labelled);
    let tokens = redact_tokens(&described);
    redact_cards(&tokens)
}

fn redact_labelled(text: &str) -> String {
    const LABELS: &[&str] = &["password", "passwd", "passcode", "pin", "secret", "api key", "api_key", "apikey", "token", "cvv", "cvc", "security code", "ssn"];
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let lower = line.to_lowercase();
        let mut cut: Option<usize> = None;
        for l in LABELS {
            let mut from = 0;
            while let Some(i) = lower[from..].find(l) {
                let at = from + i;
                let after = at + l.len();
                let word_start = at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric();
                let rest = &lower[after..];
                let sep = rest.trim_start_matches([' ', '\t']);
                if word_start && (sep.starts_with(':') || sep.starts_with('=') || sep.starts_with(" is ") || rest.starts_with(" is ")) {
                    cut = Some(cut.map_or(after, |c| c.min(after)));
                }
                from = after;
            }
        }
        match cut {
            Some(c) => {
                // keep the label, drop the value up to the end of the line
                let trailing_nl = if line.ends_with('\n') { "\n" } else { "" };
                out.push_str(line[..c].trim_end());
                out.push_str(": ");
                out.push_str(MARK);
                out.push_str(trailing_nl);
            }
            None => out.push_str(line),
        }
    }
    out
}

/// Words after which a secret may be named in running text ("the password "hunter2" is visible", "PIN number 4821").
const DESCRIBED_LABELS: &[&str] = &["password", "passwords", "passwd", "passcode", "passphrase", "pin", "secret", "token", "apikey", "cvv", "cvc", "ssn"];
/// of those, the ones whose value is often only digits
const DIGIT_LABELS: &[&str] = &["pin", "cvv", "cvc", "ssn"];
/// words that may stand between the label and its value
const FILLERS: &[&str] = &[
    "is", "was", "are", "were", "be", "of", "the", "a", "an", "to", "as", "being", "set", "reads", "read", "says", "said", "shows", "show", "shown", "showing", "displays", "displayed", "display", "visible",
    "contains", "contain", "field", "box", "entered", "typed", "text", "value", "number", "code", "appears", "written", "labelled", "labeled", "entry", "input", "it", "and", "or", "also", "here", "below", "above", "listed", "given",
];

fn bare(w: &str) -> &str {
    w.trim_matches(|c: char| !c.is_alphanumeric())
}

fn quote_open(c: char) -> bool {
    matches!(c, '"' | '\'' | '\u{201c}' | '\u{2018}' | '`')
}

fn quote_close(c: char) -> bool {
    matches!(c, '"' | '\'' | '\u{201d}' | '\u{2019}' | '`')
}

/// A bare word that looks like a secret value: letters mixed with digits or symbols, six or more characters
/// (or any run of three to ten digits after a label that is usually a number).
fn secret_like(w: &str, digits_ok: bool) -> bool {
    let w = w.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | '(' | ']' | '[' | '}' | '{'));
    let n = w.chars().count();
    if digits_ok && (3..=10).contains(&n) && w.chars().all(|c| c.is_ascii_digit() || c == '-') && w.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    n >= 6 && w.chars().any(|c| c.is_ascii_digit()) && w.chars().any(|c| c.is_alphabetic() || matches!(c, '!' | '@' | '#' | '$' | '%' | '^' | '&' | '*'))
}

/// A secret named in a sentence rather than written as `label: value`: the label word, up to four filler words, then a quoted
/// value or a word that looks like a secret. Only that value is removed. "The password manager opened" is just a sentence.
fn redact_described(text: &str) -> String {
    // split into words and the whitespace after each, so the text can be put back together exactly
    let mut words: Vec<(String, String)> = Vec::new();
    let mut cur = String::new();
    let mut gap = String::new();
    for c in text.chars() {
        if c.is_whitespace() {
            gap.push(c);
        } else {
            if !gap.is_empty() {
                words.push((std::mem::take(&mut cur), std::mem::take(&mut gap)));
            }
            cur.push(c);
        }
    }
    words.push((cur, gap));
    let mut i = 0;
    while i < words.len() {
        let label = bare(&words[i].0).to_lowercase();
        let api_key = label == "key" && i > 0 && bare(&words[i - 1].0).eq_ignore_ascii_case("api");
        if !(DESCRIBED_LABELS.contains(&label.as_str()) || api_key) {
            i += 1;
            continue;
        }
        let digits_ok = DIGIT_LABELS.contains(&label.as_str());
        let mut j = i + 1;
        let mut hit = None;
        while j < words.len() && j <= i + 5 {
            let w = &words[j].0;
            if w.chars().next().is_some_and(quote_open) {
                // a quoted value, up to six words long
                let mut k = j;
                while k < words.len() && k < j + 6 {
                    let t = words[k].0.trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
                    if (k > j || t.chars().count() > 1) && t.chars().last().is_some_and(quote_close) {
                        hit = Some((j, k));
                        break;
                    }
                    k += 1;
                }
                break;
            }
            if secret_like(w, digits_ok) {
                hit = Some((j, j));
                break;
            }
            if !FILLERS.contains(&bare(w).to_lowercase().as_str()) {
                break;
            }
            j += 1;
        }
        match hit {
            Some((from, to)) => {
                // keep the punctuation that closed the sentence or clause after the value
                let tail: String = words[to].0.chars().rev().take_while(|c| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')')).collect::<Vec<_>>().into_iter().rev().collect();
                let gap = words[to].1.clone();
                words[from] = (format!("{MARK}{tail}"), gap);
                words.drain(from + 1..=to);
                i = from + 1;
            }
            None => i += 1,
        }
    }
    words.into_iter().map(|(w, g)| w + &g).collect()
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '+' | '=')
}

fn looks_like_secret_token(w: &str) -> bool {
    let w = w.trim_matches(|c: char| !is_token_char(c));
    let len = w.chars().count();
    let known_prefix = ["sk-", "sk_", "pk_", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-", "AKIA", "ASIA", "AIza", "eyJ", "glpat-", "hf_"].iter().any(|p| w.starts_with(p));
    if known_prefix && len >= 16 {
        return true;
    }
    // long mixed token: letters and digits, no spaces, at least 32 chars (hashes, keys)
    len >= 32 && w.chars().any(|c| c.is_ascii_digit()) && w.chars().any(|c| c.is_ascii_alphabetic()) && w.chars().all(is_token_char) && !w.contains('/') && !w.contains('.')
}

fn redact_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            if looks_like_secret_token(word) {
                out.push_str(MARK);
            } else {
                out.push_str(word);
            }
            word.clear();
        }
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn redact_cards(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            // a run of digits possibly grouped by single spaces or dashes
            let mut j = i;
            let mut digits: Vec<u8> = Vec::new();
            let mut last_digit_end = i;
            while j < chars.len() {
                if chars[j].is_ascii_digit() {
                    digits.push(chars[j] as u8 - b'0');
                    j += 1;
                    last_digit_end = j;
                } else if (chars[j] == ' ' || chars[j] == '-') && j + 1 < chars.len() && chars[j + 1].is_ascii_digit() && !digits.is_empty() {
                    j += 1;
                } else {
                    break;
                }
            }
            if (13..=19).contains(&digits.len()) && luhn_ok(&digits) {
                out.push_str(MARK);
            } else {
                out.extend(&chars[i..last_digit_end]);
            }
            i = last_digit_end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_managers_banks_and_private_windows_are_never_captured() {
        for (app, title) in [("1Password", "Vault"), ("chrome", "Sign in to your account - Google"), ("firefox", "My Bank - Online Banking"), ("edge", "New InPrivate window"), ("chrome", "Checkout - Shop"), ("Notepad", "seed phrase.txt"), ("chrome", "MyChart - Test Results")] {
            assert!(sensitive_window(app, title, &[]).is_some(), "{app} / {title}");
        }
        for (app, title) in [("Code", "main.rs - xiar"), ("chrome", "Rust documentation"), ("Slack", "general")] {
            assert!(sensitive_window(app, title, &[]).is_none(), "{app} / {title}");
        }
    }

    #[test]
    fn the_users_excluded_apps_are_honoured_case_insensitively() {
        assert_eq!(sensitive_window("Discord.exe", "chat", &["discord".into()]), Some("excluded app"));
        assert!(sensitive_window("Discord.exe", "chat", &["  ".into(), "".into()]).is_none(), "blank entries exclude nothing");
    }

    #[test]
    fn labelled_secrets_lose_their_value_but_keep_the_label() {
        assert_eq!(redact("Password: hunter2"), "Password: [redacted]");
        assert_eq!(redact("line one\napi_key = abc123\nline three"), "line one\napi_key: [redacted]\nline three");
        assert_eq!(redact("my pin is 4821 ok"), "my pin: [redacted]");
        assert_eq!(redact("The password manager opened"), "The password manager opened", "a label without a value separator is just a word");
        assert_eq!(redact("spinning up"), "spinning up", "labels inside words do not trigger");
    }

    #[test]
    fn a_secret_named_in_a_sentence_loses_its_value() {
        // example of what a vision model might write about a screen
        assert_eq!(redact("The password \"hunter2hunter2\" is also visible."), "The password [redacted] is also visible.");
        assert_eq!(redact("A form whose password field shows Tr0ub4dor&3 and a Sign in button"), "A form whose password field shows [redacted] and a Sign in button");
        assert_eq!(redact("Their PIN number 4821 was typed"), "Their PIN number [redacted] was typed");
        assert_eq!(redact("the api key sk_live_FAKEEXAMPLE0 appears"), "the api key [redacted] appears");
        assert_eq!(redact("passphrase 'correct horse battery' on a sticky note"), "passphrase [redacted] on a sticky note");
        assert_eq!(redact("the card CVV 123 was visible"), "the card CVV [redacted] was visible");
        assert_eq!(redact("and the token abc12345def too"), "and the token [redacted] too");
        // ordinary sentences about passwords are not touched
        for t in [
            "The password manager opened",
            "Password reset email sent to the user",
            "Enter your password to continue",
            "A password field with dots in it",
            "Pin the tab and open settings",
            "The secret garden chapter, page 40",
            "Token ring networks were common in 1990",
            "She changed the passwords last week",
            "The key points are listed below",
        ] {
            assert_eq!(redact(t), t, "{t}");
        }
        // and it stays idempotent
        let once = redact("The password \"hunter2hunter2\" is also visible.");
        assert_eq!(redact(&once), once);
    }

    #[test]
    fn token_shapes_are_removed() {
        for t in ["sk-abcdefghijklmnop1234", "ghp_abcdefghijklmnopqrstuv", "AKIAIOSFODNN7EXAMPLE", "hf_abcdefghijklmnopqrst", "0123456789abcdef0123456789abcdef0123456789"] {
            assert_eq!(redact(&format!("key {t} end")), "key [redacted] end", "{t}");
        }
        assert_eq!(redact("see https://example.com/some/long/path/with-words-in-it-for-reading now"), "see https://example.com/some/long/path/with-words-in-it-for-reading now");
        assert_eq!(redact("antidisestablishmentarianism-pneumonoultramicroscopic"), "antidisestablishmentarianism-pneumonoultramicroscopic", "long words without digits are text");
    }

    #[test]
    fn card_numbers_are_removed_only_when_they_pass_the_luhn_check() {
        assert_eq!(redact("pay with 4111 1111 1111 1111 today"), "pay with [redacted] today");
        assert_eq!(redact("4111-1111-1111-1111"), "[redacted]");
        assert_eq!(redact("order 4111 1111 1111 1112 shipped"), "order 4111 1111 1111 1112 shipped", "fails Luhn: an order number");
        assert_eq!(redact("call 555 0100 now"), "call 555 0100 now");
        assert_eq!(redact("year 2026, total 1234"), "year 2026, total 1234");
    }

    #[test]
    fn ordinary_text_is_untouched_and_redaction_is_idempotent() {
        let t = "Reading the Rust book, chapter 4: ownership. Meeting at 3pm.";
        assert_eq!(redact(t), t);
        let once = redact("Password: x and 4111111111111111");
        assert_eq!(redact(&once), once);
    }
}
