// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Prepared for the final integrated pass; no owner brain or real credentials.
use std::{io::{BufRead,BufReader,Read,Write},net::TcpListener,path::Path,sync::{Arc,atomic::{AtomicBool,AtomicUsize,Ordering}},thread,time::Duration};
use axum::{body::{Body,to_bytes},http::{Request,StatusCode},Router};
use serde_json::{json,Value};
use tower::ServiceExt;
use xiar_memory_api::{app,init,AppState,Config};
struct Embeddings{base:String,revision:Arc<AtomicUsize>,stop:Arc<AtomicBool>,thread:Option<thread::JoinHandle<()>>}
impl Embeddings{
 fn new()->Self{
  let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();let base=format!("http://{}",listener.local_addr().unwrap());
  let revision=Arc::new(AtomicUsize::new(1));let stop=Arc::new(AtomicBool::new(false));let (r,s)=(revision.clone(),stop.clone());
  let worker=thread::spawn(move||while !s.load(Ordering::SeqCst){
   let (mut stream,_)=match listener.accept(){Ok(c)=>c,Err(e)if e.kind()==std::io::ErrorKind::WouldBlock=>{thread::sleep(Duration::from_millis(10));continue},Err(e)=>panic!("fixture server: {e}")};
   stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
   let mut reader=BufReader::new(stream.try_clone().unwrap());let mut first=String::new();reader.read_line(&mut first).unwrap();let mut length=0;
   loop{let mut line=String::new();reader.read_line(&mut line).unwrap();if line=="\r\n"||line.is_empty(){break;}if let Some((key,value))=line.split_once(':'){if key.eq_ignore_ascii_case("content-length"){length=value.trim().parse::<usize>().unwrap();}}}
   let mut body=vec![0;length];reader.read_exact(&mut body).unwrap();
   let response=if first.starts_with("GET "){json!({"data":[{"id":"fixture-embed","embedding_dimension":3,"fingerprint":format!("fixture-v{}",r.load(Ordering::SeqCst))}]})}
   else{let req:Value=serde_json::from_slice(&body).unwrap();let rows=req["input"].as_array().unwrap();let vectors=rows.iter().enumerate().map(|(i,text)|{let t=text.as_str().unwrap().to_lowercase();let v=if t.contains("sensor"){vec![1.0,0.0,0.0]}else if t.contains("logistics"){vec![0.0,0.0,1.0]}else{vec![0.0,1.0,0.0]};json!({"index":i,"embedding":v})}).collect::<Vec<_>>();assert!(["query","document"].contains(&req["input_type"].as_str().unwrap()));json!({"data":vectors})};
   let encoded=response.to_string();write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",encoded.len(),encoded).unwrap();
  });Self{base,revision,stop,thread:Some(worker)}
 }
}
impl Drop for Embeddings{fn drop(&mut self){self.stop.store(true,Ordering::SeqCst);self.thread.take().unwrap().join().unwrap();}}
fn selection(root:&Path,enabled:bool){std::fs::write(root.join("config/settings_panel.toml"),format!("[memory]\nembedding_model=\"{}\"\nretrieval_method=\"hybrid\"\nretrieval_ontology=true\n",if enabled{"fixture-embed"}else{""})).unwrap();}
fn setup(root:&Path,embed:&Embeddings)->(Router,Value){
    // the real stop state is root-owned and fails closed when absent; these tests prove the API, not the stop (tests/autonomy_barrier.rs does)
    xiar_memory_api::force_stop_for_tests(false);
 std::fs::create_dir_all(root.join("config")).unwrap();selection(root,true);
 std::fs::write(root.join("config/llm_runtime.toml"),format!("[embedding]\nserved_name=\"fixture-embed\"\nbase_url=\"{}\"\n",embed.base)).unwrap();
 let cfg:Config=toml::from_str(r#"
[server]
bind="127.0.0.1:0"
max_body_bytes=65536
[store]
root="m"
language="english"
user="owner"
master_key_file="keys/master"
[auth]
tokens_file="keys/tokens"
[[clients]]
name="alpha"
agent_id="alpha"
end_user_id="alice"
businesses=["*"]
[[clients]]
name="xiar"
agent_id="xiar"
end_user_id="owner"
businesses=["*"]
read_all_agents=true
"#).unwrap();init(&cfg,root).unwrap();let tokens=serde_json::from_slice(&std::fs::read(root.join("keys/tokens")).unwrap()).unwrap();(app(Arc::new(AppState::from_config(&cfg,root).unwrap()),65536),tokens)
}
async fn call(app:&Router,token:&str,business:&str,path:&str,body:Value,read:bool)->(StatusCode,Value){
 let mut req=Request::builder().method("POST").uri(path).header("authorization",format!("Bearer {token}")).header("X-Business-Id",business).header("content-type","application/json");
 if read{req=req.header("X-Read-Agent-Id","alpha").header("X-Read-User-Id","alice");}
 let response=app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();let status=response.status();let value=serde_json::from_slice(&to_bytes(response.into_body(),1<<20).await.unwrap()).unwrap();(status,value)
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn hybrid_scope_provenance_and_xiar_read_only(){
 let embed=Embeddings::new();let dir=tempfile::tempdir().unwrap();let (app,t)=setup(dir.path(),&embed);let alpha=t["alpha"].as_str().unwrap();let xiar=t["xiar"].as_str().unwrap();
 let (status,saved)=call(&app,alpha,"tenant-a","/v1/semantic/insert",json!({"name":"Sensor supplier","summary":"sensor vendor","details":"sensor contract"}),false).await;
 assert_eq!(status,StatusCode::CREATED);assert_eq!(saved["embedding_indexed"],true);
 let (status,found)=call(&app,alpha,"tenant-a","/v1/semantic/search",json!({"query":"sensor"}),false).await;
 assert_eq!(status,StatusCode::OK);assert_eq!(found["method"],"hybrid");assert_eq!(found["hits"].as_array().unwrap().len(),1);
 let (status,_)=call(&app,alpha,"tenant-b","/v1/semantic/get",json!({"id":saved["id"]}),false).await;assert_eq!(status,StatusCode::NOT_FOUND);
 let (status,found)=call(&app,xiar,"tenant-a","/v1/semantic/search",json!({"query":"sensor"}),true).await;assert_eq!(status,StatusCode::OK);assert_eq!(found["agent_id"],"alpha");
 let (status,_)=call(&app,xiar,"absent","/v1/semantic/search",json!({"query":"sensor"}),true).await;assert_eq!(status,StatusCode::NOT_FOUND);
 assert!(!dir.path().join("m/agents/alpha/users/alice/tenants/absent").exists());
 let (status,_)=call(&app,xiar,"tenant-a","/v1/semantic/reindex",json!({}),true).await;assert_eq!(status,StatusCode::FORBIDDEN);
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn missing_or_stale_model_index_never_returns_old_vectors(){
 let embed=Embeddings::new();let dir=tempfile::tempdir().unwrap();let (app,t)=setup(dir.path(),&embed);let alpha=t["alpha"].as_str().unwrap();
 assert_eq!(call(&app,alpha,"tenant-a","/v1/semantic/insert",json!({"name":"A","summary":"sensor"}),false).await.0,StatusCode::CREATED);
 selection(dir.path(),false);call(&app,alpha,"tenant-a","/v1/semantic/insert",json!({"name":"B","summary":"logistics"}),false).await;selection(dir.path(),true);
 assert_eq!(call(&app,alpha,"tenant-a","/v1/semantic/search",json!({"query":"sensor"}),false).await.0,StatusCode::CONFLICT);
 assert_eq!(call(&app,alpha,"tenant-a","/v1/semantic/reindex",json!({}),false).await.1["indexed"],2);
 embed.revision.store(2,Ordering::SeqCst);
 assert_eq!(call(&app,alpha,"tenant-a","/v1/semantic/search",json!({"query":"sensor"}),false).await.0,StatusCode::CONFLICT);
 assert_eq!(call(&app,alpha,"tenant-a","/v1/semantic/reindex",json!({}),false).await.0,StatusCode::OK);
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn deletion_removes_all_model_versions(){
 let embed=Embeddings::new();let dir=tempfile::tempdir().unwrap();let (app,t)=setup(dir.path(),&embed);let alpha=t["alpha"].as_str().unwrap();
 let saved=call(&app,alpha,"tenant-a","/v1/semantic/insert",json!({"name":"A","summary":"sensor"}),false).await.1;
 embed.revision.store(2,Ordering::SeqCst);assert_eq!(call(&app,alpha,"tenant-a","/v1/semantic/reindex",json!({}),false).await.0,StatusCode::OK);
 assert_eq!(call(&app,alpha,"tenant-a","/v1/semantic/delete",json!({"id":saved["id"]}),false).await.0,StatusCode::OK);
 for version in [1,2]{embed.revision.store(version,Ordering::SeqCst);let (status,found)=call(&app,alpha,"tenant-a","/v1/semantic/search",json!({"query":"sensor","method":"embedding"}),false).await;assert_eq!(status,StatusCode::OK);assert!(found["hits"].as_array().unwrap().is_empty());}
}
