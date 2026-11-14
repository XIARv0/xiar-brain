// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Versioned MIRIX Semantic dense indexes. The original BM25 store is never
//! re-created with a conflicting vector schema. Each model has a scope-bound
//! derivative index; a complete primary snapshot hash is required before search.
use std::{collections::{BTreeMap,HashMap,HashSet},fs,path::{Path,PathBuf},sync::{Arc,Mutex}};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use xiar_engine_http::{HttpEmbedEngine,HttpEngineConfig};
use xiar_llm::EmbedEngine;
use xiar_memory::{manager::{MemoryManager,Scored,Sealer},query_expand::{QueryExpander,merge_query},search::{SearchMethod,SearchRequest},tenant::TenantSemantic,types::{SemanticItem,EmbeddingMeta}};
use xiar_store::vector::{EdgeVectorStore,VectorStore};
use crate::scope::Scope;
type Result<T>=std::result::Result<T,String>;
fn hash(value:&impl Serialize)->Result<String>{Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(value).map_err(|_|"index serialization failed")?)))}
fn config(path:&Path)->Result<Value>{
    if !path.exists(){return Ok(json!({}));}
    let value:toml::Value=toml::from_str(&fs::read_to_string(path).map_err(|_|"settings unavailable")?).map_err(|_|"invalid settings TOML")?;
    serde_json::to_value(value).map_err(|_|"invalid settings".into())
}
#[derive(Clone,Debug)]
pub struct Policy {pub method:String,pub ontology:bool,pub model:String,pub field:String,pub rrf_k:f32,pub overfetch:usize}
#[derive(Clone,Serialize,Deserialize,PartialEq)]
struct Model {base:String,model:String,dim:u32,revision:String}
#[derive(Serialize,Deserialize,PartialEq)]
struct Binding {agent:String,user:String,business:String,model:Model}
struct Index {mgr:MemoryManager<SemanticItem>,store:Arc<EdgeVectorStore>,dir:PathBuf}
impl Index {
    fn snapshot(&self)->Option<String>{fs::read_to_string(self.dir.join(".snapshot")).ok()}
    fn complete(&self,snapshot:&str)->Result<()>{
        self.store.flush("semantic").map_err(|_|"index flush failed")?;
        let temp=self.dir.join(".snapshot.pending");fs::write(&temp,snapshot).map_err(|_|"index snapshot write failed")?;
        fs::rename(temp,self.dir.join(".snapshot")).map_err(|_|"index snapshot activation failed".into())
    }
}
pub struct DenseService {root:PathBuf,language:String,sealer:Arc<dyn Sealer>,indexes:Mutex<HashMap<PathBuf,Arc<Index>>>}
impl DenseService {
    pub fn new(root:&Path,language:&str,sealer:Arc<dyn Sealer>)->Self{Self{root:root.into(),language:language.into(),sealer,indexes:Mutex::new(HashMap::new())}}
    pub fn policy(&self)->Result<Policy>{
        let panel=config(&self.root.join("config/settings_panel.toml"))?;let m=&panel["memory"];
        let method=m["retrieval_method"].as_str().unwrap_or("bm25");
        if !["bm25","embedding","hybrid"].contains(&method){return Err("invalid retrieval method".into());}
        let field=m["embedding_search_field"].as_str().unwrap_or("summary");
        if !["name","summary","details"].contains(&field){return Err("invalid embedded field".into());}
        let rrf_k=m["rrf_k"].as_f64().unwrap_or(60.0)as f32;
        if !rrf_k.is_finite()||rrf_k<=0.0{return Err("invalid RRF constant".into());}
        let overfetch=m["dense_overfetch"].as_u64().unwrap_or(3);if !(1..=1000).contains(&overfetch){return Err("invalid dense overfetch".into());}
        Ok(Policy{overfetch:overfetch as usize,method:method.into(),ontology:m["retrieval_ontology"].as_bool().unwrap_or(true),model:m["embedding_model"].as_str().unwrap_or("").into(),field:field.into(),rrf_k})
    }
    fn engine(&self)->Result<(Model,HttpEmbedEngine)>{
        let policy=self.policy()?;if policy.model.is_empty(){return Err("Choose an embedding connection in Memory settings".into());}
        let runtime=config(&self.root.join("config/llm_runtime.toml"))?;
        let c=if runtime["embedding"]["served_name"]==policy.model{&runtime["embedding"]}else{&runtime["connections"][&policy.model]};
        if c.is_null()||c.get("kind").is_some_and(|v|v!="embedding"){return Err("Selected connection is not an embedding model".into());}
        let model=c["served_name"].as_str().or(c["model"].as_str()).unwrap_or(&policy.model).to_string();
        let remote=c["type"].as_str().is_some_and(|t|t!="local");
        if remote&&c["revision"].as_str().unwrap_or("").is_empty(){return Err("served/cloud embeddings require an explicit model revision before indexing".into());}
        let base=if remote{format!("http://127.0.0.1:{}",c["proxy_port"].as_u64().ok_or("Start a configured provider proxy before memory retrieval")?)}else{c["base_url"].as_str().map(str::to_owned).unwrap_or_else(||format!("http://{}:{}",c["host"].as_str().unwrap_or("127.0.0.1"),c["port"].as_u64().unwrap_or(0)))};
        if base.contains('@')||base.contains('?')||base.contains('#'){return Err("Credential-bearing embedding URLs refused".into());}
        let metadata=HttpEmbedEngine::metadata(&base).map_err(|_|"Embedding model metadata is unavailable")?;
        let row=metadata["data"].as_array().and_then(|rows|rows.iter().find(|row|row["id"]==model)).ok_or("Configured model is not served by this endpoint")?;
        let dim=row["embedding_dimension"].as_u64().or(c["dim"].as_u64().filter(|d|*d>0)).ok_or("Embedding dimension must be advertised or explicitly configured")?;
        if !(1..=65536).contains(&dim){return Err("Invalid embedding dimension".into());}
        let revision=row["fingerprint"].as_str().or(c["revision"].as_str()).filter(|s|!s.is_empty()).ok_or("This endpoint needs an explicit model revision to prevent mixed-model indexes")?.to_string();
        let spec=Model{base,model,dim:dim as u32,revision};
        let mut cfg=HttpEngineConfig::new(&spec.base,&spec.model,1);cfg.embedding_dim=Some(spec.dim);
        let engine=HttpEmbedEngine::new(cfg).map_err(|_|"Invalid embedding endpoint")?;
        Ok((spec,engine))
    }
    fn open(&self,dir:&Path,scope:&Scope,model:&Model,create:bool)->Result<Arc<Index>>{
        let binding=Binding{agent:scope.agent.clone(),user:scope.user.clone(),business:scope.business.clone(),model:model.clone()};
        let location=dir.join("d").join(&hash(model)?[..16]);let marker=location.join(".xiar-embedding");
        if marker.exists(){let got:Binding=serde_json::from_slice(&fs::read(&marker).map_err(|_|"index binding unreadable")?).map_err(|_|"invalid index binding")?;if got!=binding{return Err("Embedding index scope/model binding mismatch".into());}}
        else if create{
            if location.exists()&&fs::read_dir(&location).map_err(|_|"index directory unreadable")?.next().is_some(){return Err("Unbound embedding index refused".into());}
            fs::create_dir_all(&location).map_err(|_|"index directory creation failed")?;
            use std::io::Write;
            let mut f=fs::OpenOptions::new().write(true).create_new(true).open(&marker).map_err(|_|"index binding creation failed")?;
            f.write_all(&serde_json::to_vec(&binding).map_err(|_|"index binding serialization failed")?).map_err(|_|"index binding write failed")?;
        }else{return Err("Embedding index does not exist; rebuild this scope's index".into());}
        let mut cache=self.indexes.lock().map_err(|_|"index cache lock poisoned")?;
        if let Some(index)=cache.get(&location){return Ok(index.clone());}
        let store=Arc::new(EdgeVectorStore::open(&location,&self.language).map_err(|_|"index store open failed")?);
        let dims=[("name",model.dim),("summary",model.dim),("details",model.dim)];
        let mgr=MemoryManager::new(store.clone(),self.sealer.clone(),&dims).map_err(|_|"index schema initialization failed")?;
        let index=Arc::new(Index{mgr,store,dir:location.clone()});cache.insert(location,index.clone());Ok(index)
    }
    pub fn fingerprint(items:&[SemanticItem])->Result<String>{let mut sorted=items.to_vec();sorted.sort_by(|a,b|a.common.id.cmp(&b.common.id));hash(&sorted)}
    fn vectors(engine:&HttpEmbedEngine,item:&SemanticItem)->Result<BTreeMap<String,Vec<f32>>>{
        let rows=engine.embed(&[item.name.clone(),item.summary.clone(),item.details.clone()]).map_err(|_|"Embedding backend failed; primary Semantic memory remains intact")?;
        Ok(["name","summary","details"].into_iter().zip(rows).map(|(k,v)|(k.to_string(),v)).collect())
    }
    fn indexed_item(item:&SemanticItem,model:&Model)->SemanticItem{let mut item=item.clone();item.embedding=Some(EmbeddingMeta{plugin:"http-local".into(),model:model.model.clone(),dim:model.dim});item}
    pub fn record(&self,dir:&Path,scope:&Scope,before:&[SemanticItem],after:&[SemanticItem],item:&SemanticItem)->Result<bool>{
        if self.policy()?.model.is_empty(){return Ok(false);}
        let (model,engine)=self.engine()?;let index=self.open(dir,scope,&model,true)?;
        let empty_index=before.is_empty()&&index.mgr.list_recent(&scope.user,0,Some(&[format!("business:{}",scope.business)]),None).map_err(|_|"index snapshot scan failed")?.is_empty();
        let complete=empty_index||index.snapshot().as_deref()==Some(Self::fingerprint(before)?.as_str());
        index.mgr.insert(Self::indexed_item(item,&model),Self::vectors(&engine,item)?).map_err(|_|"dense item update failed")?;
        if complete{index.complete(&Self::fingerprint(after)?)?;}Ok(complete)
    }
    pub fn reindex(&self,dir:&Path,scope:&Scope,items:&[SemanticItem])->Result<Value>{
        let (model,engine)=self.engine()?;let index=self.open(dir,scope,&model,true)?;
        // Invalidate first: a partial/restarted reindex must never claim readiness.
        if index.dir.join(".snapshot").exists(){fs::remove_file(index.dir.join(".snapshot")).map_err(|_|"index invalidation failed")?;}
        let ids:HashSet<_>=items.iter().map(|i|i.common.id.as_str()).collect();
        // The index was invalidated above, so stopping between items is safe: an aborted reindex never claims readiness (the next one starts over). Every item
        // check also stops the calls to the embedding provider; a call already in flight finishes, no new one is made.
        for old in index.mgr.list_recent(&scope.user,0,Some(&[format!("business:{}",scope.business)]),None).map_err(|_|"index scan failed")?{
            if crate::autonomy::stopped(){return Err("autonomy_paused".into());}
            if !ids.contains(old.common.id.as_str()){index.mgr.delete(&scope.user,&old.common.id).map_err(|_|"stale index entry removal failed")?;}
        }
        for item in items{
            if crate::autonomy::stopped(){return Err("autonomy_paused".into());}
            index.mgr.insert(Self::indexed_item(item,&model),Self::vectors(&engine,item)?).map_err(|_|"dense reindex write failed")?;
        }
        if crate::autonomy::stopped(){return Err("autonomy_paused".into());}
        index.complete(&Self::fingerprint(items)?)?;
        Ok(json!({"indexed":items.len(),"ready":true,"model":model.model,"dimension":model.dim}))
    }
    pub fn erase(&self,dir:&Path,scope:&Scope,id:&str,before:&[SemanticItem],after:&[SemanticItem])->Result<()>{
        let parent=dir.join("d");if !parent.exists(){return Ok(());}
        for child in fs::read_dir(parent).map_err(|_|"derived index enumeration failed")?{
            let path=child.map_err(|_|"derived index enumeration failed")?.path();if !path.is_dir(){continue;}
            let binding:Binding=serde_json::from_slice(&fs::read(path.join(".xiar-embedding")).map_err(|_|"derived index binding missing")?).map_err(|_|"invalid derived index binding")?;
            let index=self.open(dir,scope,&binding.model,false)?;
            if index.dir!=path{return Err("derived index location mismatch".into());}
            let was_ready=index.snapshot().as_deref()==Some(Self::fingerprint(before)?.as_str());
            index.mgr.delete(&scope.user,id).map_err(|_|"derived index deletion failed")?;
            index.store.flush("semantic").map_err(|_|"derived index flush failed")?;
            if was_ready{index.complete(&Self::fingerprint(after)?)?;}
        }Ok(())
    }
    pub fn search(&self,dir:&Path,scope:&Scope,tenant:&TenantSemantic,query:&str,method:&str,limit:usize,exp:Option<&dyn QueryExpander>)->Result<Vec<Scored<SemanticItem>>>{
        if method=="bm25"{return tenant.search(query,limit,exp).map_err(|_|"Keyword/ontology search failed".into());}
        if !["embedding","hybrid"].contains(&method){return Err("Unknown retrieval method".into());}
        let policy=self.policy()?;let (model,engine)=self.engine()?;let index=self.open(dir,scope,&model,false)?;
        let items=tenant.list_recent(0).map_err(|_|"primary snapshot unavailable")?;
        if index.snapshot().as_deref()!=Some(Self::fingerprint(&items)?.as_str()){return Err("Embedding index is incomplete or stale; rebuild this scope's index".into());}
        let expanded=if method=="embedding"{exp.map(|e|e.expand(query)).unwrap_or_default()}else{vec![]};
        let q=if expanded.is_empty(){query.to_string()}else{merge_query(query,&expanded)};
        let mut vectors=engine.embed_query(&[q]).map_err(|_|"Query embedding failed")?;
        let mut req=SearchRequest::new(&scope.user,query);req.method=SearchMethod::Embedding;req.embedding=Some(vectors.remove(0));req.field=policy.field;
        req.limit=limit.saturating_mul(policy.overfetch);req.scopes=Some(vec![format!("business:{}",scope.business)]);
        let mut dense=index.mgr.search(&req).map_err(|_|"Dense Semantic search failed")?;
        let primary:BTreeMap<_,_>=items.iter().map(|item|(item.common.id.as_str(),item)).collect();
        for hit in &mut dense{
            let current=primary.get(hit.item.common.id.as_str()).ok_or("Derived index contains a removed item; rebuild this scope's index")?;
            if hit.item.name!=current.name||hit.item.summary!=current.summary||hit.item.details!=current.details{return Err("Derived index contains outdated text; rebuild this scope's index".into());}
            hit.item=(*current).clone();
        }
        if method=="embedding"{return Ok(dense.into_iter().take(limit).collect());}
        let sparse=tenant.search(query,req.limit,exp).map_err(|_|"Keyword/ontology search failed")?;
        let mut ranked:BTreeMap<String,(SemanticItem,f32)>=BTreeMap::new();
        for list in [dense,sparse]{for(rank,hit)in list.into_iter().enumerate(){let score=1.0/(policy.rrf_k+rank as f32+1.0);ranked.entry(hit.item.common.id.clone()).and_modify(|v|v.1+=score).or_insert((hit.item,score));}}
        let mut out:Vec<_>=ranked.into_values().map(|(item,score)|Scored{item,score}).collect();
        out.sort_by(|a,b|b.score.total_cmp(&a.score).then_with(||a.item.common.id.cmp(&b.item.common.id)));out.truncate(limit);Ok(out)
    }
}
