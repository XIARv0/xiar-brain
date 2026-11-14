// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use xiar_memory_api::{app, init, AppState, Config};

fn setup(dir: &std::path::Path, ofn: Option<&str>) -> (axum::Router, String, String) {
    // the real stop state is root-owned and fails closed when absent; these tests prove the API, not the stop (tests/autonomy_barrier.rs does)
    xiar_memory_api::force_stop_for_tests(false);
    let cfg: Config = toml::from_str(&format!(r#"
[server]
bind = "127.0.0.1:0"
max_body_bytes = 65536
[store]
root = "store"
language = "english"
user = "owner"
master_key_file = "secrets/master.key"
[ontology]
enabled = {}
ofn = '{}'
[auth]
tokens_file = "secrets/tokens.json"
[[clients]]
name = "hermes"
businesses = ["*"]
[[clients]]
name = "tenant-a-only"
businesses = ["tenant-a"]
"#, ofn.is_some(), ofn.unwrap_or(""))).unwrap();
    init(&cfg, dir).unwrap();
    let toks: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("secrets/tokens.json")).unwrap()).unwrap();
    let st = Arc::new(AppState::from_config(&cfg, dir).unwrap());
    (app(st, 65536), toks["hermes"].as_str().unwrap().into(), toks["tenant-a-only"].as_str().unwrap().into())
}

async fn call(app: &axum::Router, path: &str, tok: &str, biz: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut r = Request::post(path).header("content-type", "application/json").header("authorization", format!("Bearer {tok}"));
    if let Some(b) = biz { r = r.header("x-business-id", b); }
    let res = app.clone().oneshot(r.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let code = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (code, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn business_id_required_and_cross_business_denied() {
    let d = tempfile::tempdir().unwrap();
    let (app, hermes, tenant_a_only) = setup(d.path(), None);
    let (c, _) = call(&app, "/v1/semantic/insert", &hermes, Some("tenant-a"), json!({"name": "Sensor Supply Co", "summary": "tenant-a sensor supplier"})).await;
    assert_eq!(c, StatusCode::CREATED);
    let (c, _) = call(&app, "/v1/semantic/insert", &hermes, Some("tenant-b"), json!({"name": "Contoso", "summary": "tenant-b freight supplier"})).await;
    assert_eq!(c, StatusCode::CREATED);
    let (c, _) = call(&app, "/v1/semantic/search", &hermes, None, json!({"query": "supplier"})).await;
    assert_eq!(c, StatusCode::BAD_REQUEST, "missing X-Business-Id");
    let (c, b) = call(&app, "/v1/semantic/search", &hermes, Some("tenant-a"), json!({"query": "supplier"})).await;
    assert_eq!(c, StatusCode::OK);
    let names: Vec<_> = b["hits"].as_array().unwrap().iter().map(|h| h["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, vec!["Sensor Supply Co"]);
    let (c, _) = call(&app, "/v1/semantic/search", &tenant_a_only, Some("tenant-b"), json!({"query": "supplier"})).await;
    assert_eq!(c, StatusCode::FORBIDDEN);
    let (c, _) = call(&app, "/v1/semantic/insert", &tenant_a_only, Some("tenant-b"), json!({"name": "x", "summary": "y"})).await;
    assert_eq!(c, StatusCode::FORBIDDEN);
    let (c, _) = call(&app, "/v1/semantic/search", "wrong", Some("tenant-a"), json!({"query": "supplier"})).await;
    assert_eq!(c, StatusCode::UNAUTHORIZED);
    let (c, _) = call(&app, "/v1/semantic/search", &hermes, Some("../tenant-a"), json!({"query": "supplier"})).await;
    assert_eq!(c, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ontology_expansion_in_live_search() {
    let ofn = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ontologies-example/example-org_v3_promoted.ofn");
    assert!(std::path::Path::new(ofn).exists(), "promoted ontology missing: {ofn}");
    let d = tempfile::tempdir().unwrap();
    let (app, hermes, _) = setup(d.path(), Some(ofn));
    // Type claims are checked against the SCOPE'S OWN promoted ontology (tenant isolation), so each tenant is created first and given its ontology file.
    for (biz, name) in [("tenant-a", "Seed A"), ("tenant-b", "Seed B")] {
        let (c, _) = call(&app, "/v1/semantic/insert", &hermes, Some(biz), json!({"name": name, "summary": "seed"})).await;
        assert_eq!(c, StatusCode::CREATED);
        let agents = d.path().join("store/agents");
        let agent = std::fs::read_dir(&agents).unwrap().next().unwrap().unwrap().path();
        std::fs::copy(ofn, agent.join("users/owner/tenants").join(biz).join("ontology-promoted.ofn")).unwrap();
    }
    let ins = |biz: &'static str, name: &'static str, summary: &'static str| {
        let app = app.clone();
        let hermes = hermes.clone();
        async move {
            let (c, b) = call(&app, "/v1/semantic/insert", &hermes, Some(biz), json!({"name": name, "summary": summary, "entity_types": ["Vendor"]})).await;
            assert_eq!(c, StatusCode::CREATED, "{b}");
        }
    };
    ins("tenant-a", "Sensor Supply Co", "makes the temperature sensors").await;
    ins("tenant-b", "Freight Partners", "moves the pallets").await;
    let (c, b) = call(&app, "/v1/semantic/search", &hermes, Some("tenant-a"), json!({"query": "Which vendors are mentioned?"})).await;
    assert_eq!(c, StatusCode::OK);
    assert_eq!(b["ontology"], json!(true));
    let names: Vec<_> = b["hits"].as_array().unwrap().iter().map(|h| h["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, vec!["Sensor Supply Co"], "expanded search finds the typed fact and stays inside the tenant");
}