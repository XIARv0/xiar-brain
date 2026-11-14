// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Phase 8.2 item 3: the emergency-stop barriers of the memory API. One test function on purpose: the forced stop is process-wide.
use axum::body::{to_bytes,Body};
use axum::http::{Request,StatusCode};
use serde_json::{json,Value};
use std::sync::Arc;
use tower::ServiceExt;
use xiar_memory_api::{app,force_stop_for_tests,init,mutations_in_flight,AppState,Config};

fn setup(root:&std::path::Path)->(axum::Router,Value){
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
name="alpha"
agent_id="alpha"
end_user_id="alice"
businesses=["tenant-a"]
"#).unwrap();
    init(&cfg,root).unwrap();
    let tokens=serde_json::from_slice(&std::fs::read(root.join("secrets/tokens.json")).unwrap()).unwrap();
    (app(Arc::new(AppState::from_config(&cfg,root).unwrap()),65536),tokens)
}
async fn call(app:&axum::Router,t:&Value,path:&str,body:Value)->(StatusCode,Value){
    let req=Request::post(path).header("content-type","application/json").header("authorization",format!("Bearer {}",t["alpha"].as_str().unwrap())).header("x-business-id","tenant-a").body(Body::from(body.to_string())).unwrap();
    let res=app.clone().oneshot(req).await.unwrap();let code=res.status();let bytes=to_bytes(res.into_body(),1<<20).await.unwrap();
    (code,serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
async fn count(app:&axum::Router,t:&Value)->usize{
    let (code,v)=call(app,t,"/v1/semantic/list",json!({"query":"","limit":1})).await;assert_eq!(code,StatusCode::OK);v["total"].as_u64().unwrap() as usize
}
#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn stop_refuses_every_mutation_and_never_commits_after_it() {
    let d=tempfile::tempdir().unwrap();let (app,t)=setup(d.path());
    force_stop_for_tests(false);
    let (code,row)=call(&app,&t,"/v1/semantic/insert",json!({"name":"keep","summary":"kept before the stop"})).await;assert_eq!(code,StatusCode::CREATED);
    let id=row["id"].as_str().unwrap().to_string();
    // 1. while stopped, every one of the six mutation paths answers 423 and changes nothing
    force_stop_for_tests(true);
    for (path,body) in [("/v1/semantic/insert",json!({"name":"x","summary":"x"})),("/v1/semantic/delete",json!({"id":id})),("/v1/semantic/reindex",json!({})),("/v1/jobs/enqueue",json!({"kind":"noop"})),("/v1/jobs/run",json!({})),("/v1/knowledge/remove",json!({"document":"d"}))] {
        let (code,_)=call(&app,&t,path,body).await;assert_eq!(code,StatusCode::LOCKED,"{path} must be refused while stopped");
    }
    // reads stay available
    let (code,_)=call(&app,&t,"/v1/semantic/list",json!({"query":"","limit":1})).await;assert_eq!(code,StatusCode::OK);
    force_stop_for_tests(false);
    assert_eq!(count(&app,&t).await,1,"nothing changed while stopped");
    assert_eq!(mutations_in_flight(),0);
    // 2. the stop lands in the middle of a burst of concurrent inserts: each answer is 201 or 423, and the store holds exactly the 201s (nothing refused was committed)
    let mut tasks=Vec::new();
    for n in 0..64 {let a=app.clone();let tk=t.clone();tasks.push(tokio::spawn(async move{call(&a,&tk,"/v1/semantic/insert",json!({"name":format!("burst {n}"),"summary":format!("burst item number {n}")})).await.0}));}
    tokio::task::yield_now().await;force_stop_for_tests(true);
    let mut created=0;for h in tasks{match h.await.unwrap(){StatusCode::CREATED=>created+=1,StatusCode::LOCKED=>{},other=>panic!("unexpected {other}")}}
    force_stop_for_tests(false);
    assert_eq!(count(&app,&t).await,1+created,"stored items must equal the accepted inserts");
    assert_eq!(mutations_in_flight(),0,"every admitted mutation has finished");
}
