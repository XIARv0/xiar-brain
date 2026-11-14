// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! `filter_tags`: free-form JSON tags attached to every memory item for filtering and access control.
//!
//! Semantics ported from MIRIX `database/filter_tags_query.py`:
//! * plain scalar value: exact match, compared **as strings** (`3` matches `"3"`);
//! * `{"$contains": v}`: the stored value is an array that contains `v`;
//! * `{"$exists": true|false}`: the key is present (or absent);
//! * `{"$in": [..]}`: the stored scalar is one of the listed values (empty list matches nothing);
//! * exactly one operator per key; unknown operators are errors;
//! * the keys `scope` and `read_scopes` inside a query are ignored: scope authorization is a separate parameter,
//!   `scopes`, and filters on `tags["scope"]`. `Some(&[])` (authorized for nothing) matches nothing; `None` = no
//!   scope check.

use serde_json::{Map, Value};

pub type Tags = Map<String, Value>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FilterError {
    #[error("filter_tags value for {key:?} has no operator key (expected one of $contains, $exists, $in)")]
    NoOperator { key: String },
    #[error("filter_tags value for {key:?} has multiple operators; only one per key is supported")]
    MultipleOperators { key: String },
    #[error("unknown filter_tags operator {op:?} (supported: $contains, $exists, $in)")]
    UnknownOperator { op: String },
    #[error("$exists for {key:?} must be a boolean")]
    ExistsNotBool { key: String },
    #[error("$in for {key:?} must be an array")]
    InNotArray { key: String },
}

const IGNORED_KEYS: &[&str] = &["scope", "read_scopes"];

fn as_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn is_operator_dict(v: &Value) -> bool {
    matches!(v, Value::Object(m) if m.keys().any(|k| k.starts_with('$')))
}

/// Validate a query without evaluating it (so callers fail fast before touching storage).
pub fn validate(query: &Tags) -> Result<(), FilterError> {
    for (key, val) in query {
        if IGNORED_KEYS.contains(&key.as_str()) || !is_operator_dict(val) {
            continue;
        }
        let Value::Object(m) = val else { unreachable!() };
        let ops: Vec<&String> = m.keys().filter(|k| k.starts_with('$')).collect();
        if ops.len() > 1 {
            return Err(FilterError::MultipleOperators { key: key.clone() });
        }
        match ops[0].as_str() {
            "$contains" => {}
            "$exists" => {
                if !m["$exists"].is_boolean() {
                    return Err(FilterError::ExistsNotBool { key: key.clone() });
                }
            }
            "$in" => {
                if !m["$in"].is_array() {
                    return Err(FilterError::InNotArray { key: key.clone() });
                }
            }
            op => return Err(FilterError::UnknownOperator { op: op.to_string() }),
        }
    }
    Ok(())
}

/// Does an item with `stored` tags satisfy `query` and the caller's authorized `scopes`?
pub fn matches(stored: Option<&Tags>, query: Option<&Tags>, scopes: Option<&[String]>) -> Result<bool, FilterError> {
    if let Some(q) = query {
        validate(q)?;
    }
    if let Some(scopes) = scopes {
        let item_scope = stored.and_then(|t| t.get("scope")).map(as_string);
        match item_scope {
            Some(s) if scopes.contains(&s) => {}
            _ => return Ok(false), // includes the empty-scopes "authorized for nothing" case
        }
    }
    let Some(query) = query else { return Ok(true) };
    for (key, val) in query {
        if IGNORED_KEYS.contains(&key.as_str()) {
            continue;
        }
        let have = stored.and_then(|t| t.get(key));
        let ok = if is_operator_dict(val) {
            let Value::Object(m) = val else { unreachable!() };
            let (op, arg) = m.iter().find(|(k, _)| k.starts_with('$')).expect("validated");
            match op.as_str() {
                "$contains" => matches!(have, Some(Value::Array(a)) if a.iter().any(|x| as_string(x) == as_string(arg))),
                "$exists" => have.is_some() == arg.as_bool().unwrap_or(true),
                "$in" => {
                    let list = arg.as_array().expect("validated");
                    match have {
                        Some(h) if !matches!(h, Value::Array(_) | Value::Object(_)) => list.iter().any(|x| as_string(x) == as_string(h)),
                        _ => false,
                    }
                }
                _ => unreachable!("validated"),
            }
        } else {
            matches!(have, Some(h) if !matches!(h, Value::Array(_) | Value::Object(_)) && as_string(h) == as_string(val))
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(v: Value) -> Tags {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn plain_scalars_compare_as_strings() {
        let stored = t(json!({"project": "alpha", "priority": 3, "done": false}));
        assert!(matches(Some(&stored), Some(&t(json!({"project": "alpha"}))), None).unwrap());
        assert!(matches(Some(&stored), Some(&t(json!({"priority": "3"}))), None).unwrap(), "3 == \"3\"");
        assert!(matches(Some(&stored), Some(&t(json!({"priority": 3}))), None).unwrap());
        assert!(matches(Some(&stored), Some(&t(json!({"done": "false"}))), None).unwrap());
        assert!(!matches(Some(&stored), Some(&t(json!({"project": "beta"}))), None).unwrap());
        assert!(!matches(Some(&stored), Some(&t(json!({"missing": "x"}))), None).unwrap());
        assert!(!matches(None, Some(&t(json!({"project": "alpha"}))), None).unwrap());
        assert!(matches(None, None, None).unwrap());
    }

    #[test]
    fn contains_exists_in_operators() {
        let stored = t(json!({"tags": ["important", "work"], "owner": "ann"}));
        assert!(matches(Some(&stored), Some(&t(json!({"tags": {"$contains": "work"}}))), None).unwrap());
        assert!(!matches(Some(&stored), Some(&t(json!({"tags": {"$contains": "home"}}))), None).unwrap());
        assert!(!matches(Some(&stored), Some(&t(json!({"owner": {"$contains": "ann"}}))), None).unwrap(), "$contains needs an array");
        assert!(matches(Some(&stored), Some(&t(json!({"owner": {"$exists": true}}))), None).unwrap());
        assert!(matches(Some(&stored), Some(&t(json!({"nope": {"$exists": false}}))), None).unwrap());
        assert!(!matches(Some(&stored), Some(&t(json!({"owner": {"$exists": false}}))), None).unwrap());
        assert!(matches(Some(&stored), Some(&t(json!({"owner": {"$in": ["bob", "ann"]}}))), None).unwrap());
        assert!(!matches(Some(&stored), Some(&t(json!({"owner": {"$in": []}}))), None).unwrap(), "empty $in matches nothing");
        assert!(!matches(Some(&stored), Some(&t(json!({"tags": {"$in": ["work"]}}))), None).unwrap(), "$in is for scalars");
    }

    #[test]
    fn operator_errors_are_reported_before_evaluation() {
        let stored = t(json!({"a": 1}));
        let bad = |q: Value| matches(Some(&stored), Some(&t(q)), None);
        assert_eq!(bad(json!({"a": {"$gt": 1}})), Err(FilterError::UnknownOperator { op: "$gt".into() }));
        assert_eq!(bad(json!({"a": {"$in": [1], "$exists": true}})), Err(FilterError::MultipleOperators { key: "a".into() }));
        assert_eq!(bad(json!({"a": {"$exists": "yes"}})), Err(FilterError::ExistsNotBool { key: "a".into() }));
        assert_eq!(bad(json!({"a": {"$in": 5}})), Err(FilterError::InNotArray { key: "a".into() }));
    }

    #[test]
    fn scope_authorization_is_separate_and_deny_by_default() {
        let work = t(json!({"scope": "work", "k": "v"}));
        let home = t(json!({"scope": "home"}));
        let none = t(json!({"k": "v"}));
        let auth = |s: &[&str]| -> Vec<String> { s.iter().map(|x| x.to_string()).collect() };
        assert!(matches(Some(&work), None, Some(&auth(&["work", "home"]))).unwrap());
        assert!(!matches(Some(&home), None, Some(&auth(&["work"]))).unwrap());
        assert!(!matches(Some(&none), None, Some(&auth(&["work"]))).unwrap(), "unscoped item is not visible to a scoped caller");
        assert!(!matches(Some(&work), None, Some(&auth(&[]))).unwrap(), "authorized for nothing sees nothing");
        assert!(matches(Some(&none), None, None).unwrap(), "no scope check when scopes is None");
        // a query cannot widen or bypass scope by naming it
        assert!(!matches(Some(&home), Some(&t(json!({"scope": "home", "read_scopes": ["home"]}))), Some(&auth(&["work"]))).unwrap());
        // scope/read_scopes keys inside the query are ignored (not matched against stored data)
        assert!(matches(Some(&work), Some(&t(json!({"scope": "anything"}))), None).unwrap());
    }
}
