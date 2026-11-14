// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Requests to a program on THIS computer that wants a bearer key (the Hermes agent server). Same rules as the rest of the crate: plain
//! `http://127.0.0.1`, no TLS, no redirects. The key is given by the caller and is never logged or returned.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

const MAX_BODY: u64 = 16 * 1024 * 1024;
const METHODS: [&str; 5] = ["GET", "POST", "PATCH", "PUT", "DELETE"];

fn check(port: u16, method: &str, path: &str) -> Result<String, String> {
    if port < 1024 {
        return Err("the port must be 1024 or higher".into());
    }
    if !METHODS.contains(&method) {
        return Err(format!("method {method} is not allowed"));
    }
    if !path.starts_with('/') || path.starts_with("//") || path.contains(['\r', '\n', ' ', '\\']) || path.contains("..") {
        return Err("the path must start with a single / and hold no spaces, line breaks, backslashes or ..".into());
    }
    Ok(format!("http://127.0.0.1:{port}{path}"))
}

fn send(agent: &ureq::Agent, method: &str, url: &str, bearer: &str, body: Option<&Value>) -> Result<ureq::http::Response<ureq::Body>, String> {
    let auth = format!("Bearer {bearer}");
    let result = match (method, body) {
        ("GET", _) => agent.get(url).header("Authorization", &auth).call(),
        ("DELETE", _) => agent.delete(url).header("Authorization", &auth).call(),
        (m, b) => {
            let payload = b.map(Value::to_string).unwrap_or_else(|| "{}".into());
            let req = match m {
                "POST" => agent.post(url),
                "PATCH" => agent.patch(url),
                _ => agent.put(url),
            };
            req.header("Authorization", &auth).header("Content-Type", "application/json").send(payload.as_bytes())
        }
    };
    result.map_err(|e| format!("nothing answered: {e}"))
}

/// One request with a bearer key. Returns the status and the answer text (cut at 16 MB).
pub fn bearer_request(port: u16, method: &str, path: &str, bearer: &str, body: Option<&Value>, timeout: Duration) -> Result<(u16, String), String> {
    let url = check(port, method, path)?;
    let agent = crate::agent(timeout);
    let mut resp = send(&agent, method, &url, bearer, body)?;
    let status = resp.status().as_u16();
    let mut text = String::new();
    resp.body_mut().as_reader().take(MAX_BODY).read_to_string(&mut text).map_err(|e| format!("could not read the answer: {e}"))?;
    Ok((status, text))
}

/// A request whose answer is a stream of lines (server-sent events). `on_line` gets each line without its line break and returns `false` to
/// stop reading. Returns the status once the stream ends. `idle` is how long a silent stream may stay open.
pub fn bearer_stream(port: u16, method: &str, path: &str, bearer: &str, body: Option<&Value>, idle: Duration, mut on_line: impl FnMut(&str) -> bool) -> Result<u16, String> {
    let url = check(port, method, path)?;
    let agent = crate::agent(idle);
    let mut resp = send(&agent, method, &url, bearer, body)?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let mut text = String::new();
        let _ = resp.body_mut().as_reader().take(64 * 1024).read_to_string(&mut text);
        return Err(format!("HTTP {status}: {}", text.chars().take(300).collect::<String>()));
    }
    let mut lines = BufReader::new(resp.body_mut().as_reader());
    let mut line = String::new();
    loop {
        line.clear();
        match lines.by_ref().take(1024 * 1024).read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if !on_line(line.trim_end_matches(['\r', '\n'])) {
                    break;
                }
            }
            Err(e) => return Err(format!("the stream broke: {e}")),
        }
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_methods_are_checked_before_anything_is_sent() {
        assert!(check(18742, "GET", "/health").is_ok());
        assert!(check(80, "GET", "/health").is_err());
        assert!(check(18742, "TRACE", "/health").is_err());
        assert!(check(18742, "GET", "health").is_err());
        assert!(check(18742, "GET", "//evil.example/x").is_err());
        assert!(check(18742, "GET", "/a/../b").is_err());
        assert!(check(18742, "GET", "/a b").is_err());
        assert!(check(18742, "GET", "/a\r\nHost: x").is_err());
    }

    #[test]
    fn nothing_listening_is_an_error_not_a_hang() {
        let err = bearer_request(1999, "GET", "/health", "k", None, Duration::from_secs(2)).unwrap_err();
        assert!(err.contains("nothing answered"));
    }
}
