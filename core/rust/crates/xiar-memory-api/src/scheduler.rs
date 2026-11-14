// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Privileged scheduler operational lifecycle: real scoped Episodic + the canonical encrypted audit ledger.
//! No caller task text, business facts, provider keys, lesson data or neural actions accepted.
use super::*;
use sha2::{Digest,Sha256};
use std::sync::Mutex;
static SERIAL:Mutex<()>=Mutex::new(());
#[derive(Deserialize,serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Lifecycle {
    run_id:String,agent:String,user:String,business:String,state:String,template:String,archive:bool,
}
pub(crate) fn authenticate(h:&HeaderMap)->Result<(),ApiErr>{
    let token=h.get("authorization").and_then(|v|v.to_str().ok()).and_then(|v|v.strip_prefix("Bearer ")).unwrap_or("");
    let path=Path::new("/etc/xiar/scheduler-memory.sha256");
    let info=std::fs::symlink_metadata(path).map_err(|_|err(StatusCode::UNAUTHORIZED,"scheduler capability required"))?;
    if !info.is_file()||info.file_type().is_symlink(){return Err(err(StatusCode::UNAUTHORIZED,"scheduler verifier invalid"));}
    #[cfg(unix)]{use std::os::unix::fs::MetadataExt;if !super::autonomy::root_owned(info.uid())||info.mode()&0o022!=0{return Err(err(StatusCode::UNAUTHORIZED,"scheduler verifier invalid"));}}
    let expected=std::fs::read_to_string(path).map_err(|_|err(StatusCode::UNAUTHORIZED,"scheduler verifier unavailable"))?;
    if token.len()!=64||expected.trim().len()!=64||!ct_eq(format!("{:x}",Sha256::digest(token.as_bytes())).as_bytes(),expected.trim().as_bytes()){
        return Err(err(StatusCode::UNAUTHORIZED,"scheduler capability refused"));
    }Ok(())
}
pub(crate) async fn lifecycle(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<Lifecycle>)->Result<Json<Value>,ApiErr>{
    authenticate(&h)?;
    if b.run_id.len()!=32||!b.run_id.bytes().all(|c|c.is_ascii_hexdigit()&&!c.is_ascii_uppercase())||!["queued","admitting","launching","running","cancelling","completed","failed","cancelled","killed_budget"].contains(&b.state.as_str()){
        return Err(err(StatusCode::BAD_REQUEST,"invalid scheduler lifecycle"));
    }
    for value in [&b.agent,&b.user]{if !matches!(xiar_memory::tenant::BusinessId::parse(value),Ok(ref id) if id.slug()==value){return Err(err(StatusCode::BAD_REQUEST,"normalized scheduler identity required"));}}
    if !b.business.is_empty()&&!matches!(xiar_memory::tenant::BusinessId::parse(&b.business),Ok(ref id) if id.slug()==b.business){return Err(err(StatusCode::BAD_REQUEST,"normalized business required"));}
    if b.template.len()>128||!b.template.bytes().all(|v|v.is_ascii_alphanumeric()||b"-_.".contains(&v)){return Err(err(StatusCode::BAD_REQUEST,"invalid template identifier"));}
    tokio::task::spawn_blocking(move||{
        let _serial=SERIAL.lock().map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scheduler archive lock"))?;
        let session=format!("scheduler:{}",b.run_id);let kind=format!("scheduler_{}",b.state);
        let entries=st.ledger.session_entries(&session).map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scheduler audit unavailable"))?;
        let existing=entries.iter().find(|m|m.kind==kind);
        if let Some(m)=existing{
            let previous:Value=serde_json::from_slice(&st.ledger.read_payload(m.seq).map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scheduler audit sealed payload unavailable"))?).map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scheduler audit invalid"))?;
            if previous["agent"]!=b.agent||previous["user"]!=b.user||previous["business"]!=b.business{return Err(err(StatusCode::CONFLICT,"scheduler run identity changed"));}
        }
        let seq=if let Some(m)=existing{m.seq}else{
            let payload=serde_json::to_vec(&b).map_err(|_|err(StatusCode::BAD_REQUEST,"scheduler metadata invalid"))?;
            st.ledger.append(&xiar_audit::NewEntry{session:&session,user:&b.user,bot:&b.agent,kind:&kind,tags:&["scheduler","operational"],payload:&payload,ts_ms:xiar_audit::now_ms()}).map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scheduler canonical audit failed"))?.seq
        };
        let terminal=["completed","failed","cancelled","killed_budget"].contains(&b.state.as_str());let mut archived=false;
        if terminal&&b.archive&&!b.business.is_empty(){
            let scope=Scope{agent:b.agent.clone(),user:b.user.clone(),business:b.business.clone()};
            let lock=st.tenants.operation_lock(&scope).map_err(|_|err(StatusCode::BAD_REQUEST,"scheduler memory scope unavailable"))?;
            let _write=lock.write().map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scheduler scope lock"))?;
            let manager=st.tenants.episodic(&scope).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"episodic store unavailable"))?;
            let id=format!("scheduler-{}",b.run_id);let scopes=vec![format!("business:{}",b.business)];
            if manager.get(&b.user,&id,Some(&scopes)).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"episodic lookup unavailable"))?.is_none(){
                let summary=if b.state=="killed_budget"{"Subagent killed: budget".to_string()}else{format!("Subagent {}",b.state)};
                let details=json!({"run_id":b.run_id,"template":b.template,"state":b.state,"audit_seq":seq}).to_string();
                let mut item=xiar_memory::types::EpisodicEvent::new(&b.user,"system_notification",&summary,&details,"assistant",xiar_memory::types::now_ms());
                item.common.id=id;item.common.agent_id=Some(b.agent.clone());item.common.client_id=Some("scheduler".into());
                item.common.filter_tags=Some(serde_json::Map::from_iter([("scope".into(),json!(scopes[0])),("tags".into(),json!(["scheduler","operational",scopes[0]]))]));
                manager.insert(item,std::collections::BTreeMap::new()).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"episodic archive failed"))?;
            }archived=true;
        }
        Ok(Json(json!({"audited":true,"audit_seq":seq,"archived":archived,"archive_deferred":terminal&&!archived,"reason":if terminal&&!archived{"Memory disabled or no explicit business binding; canonical audit only"}else{"Canonical scheduler lifecycle recorded"}})))
    }).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scheduler lifecycle unavailable"))?
}

#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub(crate) struct EventGet {id:String}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub(crate) struct EventList {limit:Option<usize>}
pub(crate) async fn get(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<EventGet>)->Result<Json<Value>,ApiErr>{
    let scope=authorize(&st,&h,true)?;
    if b.id.len()>200{return Err(err(StatusCode::BAD_REQUEST,"episodic id exceeds bound"));}
    tokio::task::spawn_blocking(move||{
        let manager=st.tenants.episodic(&scope).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"episodic store unavailable"))?;
        let scopes=vec![format!("business:{}",scope.business)];
        let item=manager.get(&scope.user,&b.id,Some(&scopes)).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"episodic read unavailable"))?;
        Ok(Json(json!({"item":item})))
    }).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"episodic read failed"))?
}
pub(crate) async fn list(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<EventList>)->Result<Json<Value>,ApiErr>{
    let scope=authorize(&st,&h,true)?;let limit=b.limit.unwrap_or(st.search.default_limit).clamp(1,st.search.max_limit.max(1));
    tokio::task::spawn_blocking(move||{
        let manager=st.tenants.episodic(&scope).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"episodic store unavailable"))?;
        let scopes=vec![format!("business:{}",scope.business)];
        let items=manager.list_recent(&scope.user,limit,Some(&scopes),None).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"episodic list unavailable"))?;
        Ok(Json(json!({"items":items})))
    }).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"episodic list failed"))?
}

#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub(crate) struct CoreRead {agent:String,user:String}
pub(crate) async fn core(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<CoreRead>)->Result<Json<Value>,ApiErr>{
    authenticate(&h)?;
    for value in [&b.agent,&b.user]{if !matches!(xiar_memory::tenant::BusinessId::parse(value),Ok(ref id) if id.slug()==value){return Err(err(StatusCode::BAD_REQUEST,"normalized Core identity required"));}}
    tokio::task::spawn_blocking(move||{
        let mut blocks=Vec::new();
        // Shared human preferences are user-bound; personas remain Agent-bound.
        // Read only these allowed Core labels. Never enumerate other memories,
        // other Agents' personas, Vault values or trainer lesson storage.
        for (scope,label) in [(String::new(),"human"),(format!("agent:{}",b.agent),"persona")]{
            if let Some(block)=st.core.get(&b.user,&scope,label).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"Core read unavailable"))?{blocks.push(json!({"label":label,"value":block.value}));}
        }
        Ok(Json(json!({"blocks":blocks})))
    }).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"Core read unavailable"))?
}
