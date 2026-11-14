// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Final integrated-pass proof cases for (Agent, business, end user) isolation.
use axum::body::{to_bytes,Body};
use axum::http::{Request,StatusCode};
use serde_json::{json,Value};
use std::sync::Arc;
use tower::ServiceExt;
use xiar_memory_api::{app,init,AppState,Config};

fn setup(root: &std::path::Path)->(axum::Router,Value) {
    // the real stop state is root-owned and fails closed when absent; these tests prove the API, not the stop (tests/autonomy_barrier.rs does)
    xiar_memory_api::force_stop_for_tests(false);
    let cfg:Config=toml::from_str(r#"
[server]
bind="127.0.0.1:0"
max_body_bytes=65536
[store]
root="store"
language="english"
user="owner"
master_key_file="secrets/master"
[auth]
tokens_file="secrets/tokens.json"
[[clients]]
name="xiar"
agent_id="xiar"
end_user_id="owner"
businesses=["*"]
read_all_agents=true
[[clients]]
name="alpha-alice"
agent_id="alpha"
end_user_id="alice"
businesses=["tenant-a"]
[[clients]]
name="alpha-bob"
agent_id="alpha"
end_user_id="bob"
businesses=["tenant-a"]
[[clients]]
name="bravo-alice"
agent_id="bravo"
end_user_id="alice"
businesses=["tenant-a","tenant-b"]
read_all_agents=true
"#).unwrap();
    init(&cfg,root).unwrap();
    let tokens=serde_json::from_slice(&std::fs::read(root.join("secrets/tokens.json")).unwrap()).unwrap();
    (app(Arc::new(AppState::from_config(&cfg,root).unwrap()),65536),tokens)
}
async fn call(app:&axum::Router,tokens:&Value,client:&str,path:&str,business:&str,extra:&[(&str,&str)],body:Value)->(StatusCode,Value) {
    let mut req=Request::post(path).header("content-type","application/json")
        .header("authorization",format!("Bearer {}",tokens[client].as_str().unwrap()))
        .header("x-business-id",business);
    for (k,v) in extra {req=req.header(*k,*v);}
    let res=app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let code=res.status();let bytes=to_bytes(res.into_body(),1<<20).await.unwrap();
    (code,serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn agent_and_private_user_isolation_and_xiar_read_only_grant() {
    let d=tempfile::tempdir().unwrap();let (app,t)=setup(d.path());
    let (code,row)=call(&app,&t,"alpha-alice","/v1/semantic/insert","tenant-a",&[],json!({"name":"scope probe","summary":"private sensor probe"})).await;
    assert_eq!(code,StatusCode::CREATED);
    let id=row["id"].as_str().unwrap();
    for client in ["alpha-bob","bravo-alice"] {
        let (code,body)=call(&app,&t,client,"/v1/semantic/search","tenant-a",&[],json!({"query":"private sensor probe"})).await;
        assert_eq!(code,StatusCode::OK);assert!(body["hits"].as_array().unwrap().is_empty());
        let (code,_)=call(&app,&t,client,"/v1/semantic/get","tenant-a",&[],json!({"id":id})).await;
        assert_eq!(code,StatusCode::NOT_FOUND);
    }
    let target=[("x-read-agent-id","alpha"),("x-read-user-id","alice")];
    let (code,body)=call(&app,&t,"xiar","/v1/semantic/get","tenant-a",&target,json!({"id":id})).await;
    assert_eq!(code,StatusCode::OK);assert_eq!(body["agent_id"],"alpha");assert_eq!(body["end_user_id"],"alice");
    for client in ["bravo-alice","alpha-bob"] {
        let (code,_)=call(&app,&t,client,"/v1/semantic/search","tenant-a",&target,json!({"query":"private"})).await;
        assert_eq!(code,StatusCode::FORBIDDEN);
    }
    let (code,_)=call(&app,&t,"xiar","/v1/semantic/insert","tenant-a",&target,json!({"name":"bad","summary":"cross write"})).await;
    assert_eq!(code,StatusCode::FORBIDDEN);
    for (key,value) in [("x-agent-id","bravo"),("x-end-user-id","bob")] {
        let (code,_)=call(&app,&t,"alpha-alice","/v1/semantic/search","tenant-a",&[(key,value)],json!({"query":"private"})).await;
        assert_eq!(code,StatusCode::FORBIDDEN);
    }
    let (code,_)=call(&app,&t,"alpha-alice","/v1/semantic/search","tenant-b",&[],json!({"query":"private"})).await;
    assert_eq!(code,StatusCode::FORBIDDEN);
    for (agent,user) in [("alpha","alice"),("alpha","bob"),("bravo","alice")] {
        assert!(d.path().join(format!("store/agents/{agent}/users/{user}/tenants/tenant-a/.xiar-scope")).exists());
    }
}
#[tokio::test]
async fn tampered_scope_marker_is_refused_even_after_handle_cached() {
    let d=tempfile::tempdir().unwrap();let (app,t)=setup(d.path());
    let (code,_)=call(&app,&t,"alpha-alice","/v1/semantic/insert","tenant-a",&[],json!({"name":"marker probe","summary":"private"})).await;
    assert_eq!(code,StatusCode::CREATED);
    std::fs::write(d.path().join("store/agents/alpha/users/alice/tenants/tenant-a/.xiar-scope"),b"{}").unwrap();
    let (code,_)=call(&app,&t,"alpha-alice","/v1/semantic/search","tenant-a",&[],json!({"query":"private"})).await;
    assert_eq!(code,StatusCode::BAD_REQUEST);
}