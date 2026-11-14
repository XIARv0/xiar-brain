// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Rust tier enforcement shared by Hermes/native dispatch through the privileged broker.
//! Unknown/opaque tools never inherit Free permissions; no prompt/argument content in audit.
use super::*;
use sha2::{Digest,Sha256};
const DEFAULT_POLICY:&str="/etc/xiar/autonomy.toml";
const DEFAULT_STATE:&str="/etc/xiar/autonomy-state.json";
/// Policy file: $XIAR_AUTONOMY_CONFIG, else /etc/xiar/autonomy.toml.
pub(crate) fn policy_path()->PathBuf{
    std::env::var_os("XIAR_AUTONOMY_CONFIG").map(PathBuf::from).unwrap_or_else(||PathBuf::from(DEFAULT_POLICY))
}
/// State file: $XIAR_AUTONOMY_STATE, else control.state_file in the policy, else /etc/xiar/autonomy-state.json.
pub(crate) fn state_path()->PathBuf{
    if let Some(p)=std::env::var_os("XIAR_AUTONOMY_STATE"){return resolve_cfg_path(PathBuf::from(p));}
    if let Ok(txt)=std::fs::read_to_string(policy_path()){
        #[derive(Deserialize)]struct Control{#[serde(default)]state_file:Option<PathBuf>}
        #[derive(Deserialize)]struct File{#[serde(default)]control:Option<Control>}
        if let Ok(v)=toml::from_str::<File>(&txt){if let Some(p)=v.control.and_then(|c|c.state_file){return resolve_cfg_path(p);}}
    }
    PathBuf::from(DEFAULT_STATE)
}
pub(crate) fn resolve_cfg_path(p:PathBuf)->PathBuf{if p.is_absolute(){p}else{crate::xiar_root().join(p)}}
/// Explicit local-dev only, and never when the policy path is still the production /etc/xiar file.
pub(crate) fn dev_mode()->bool{
    match std::env::var("XIAR_AUTONOMY_DEV"){
        Ok(v) if matches!(v.to_ascii_lowercase().as_str(),"1"|"true"|"yes")=>policy_path()!=Path::new(DEFAULT_POLICY),
        _=>false,
    }
}
fn process_uid()->u32{
    #[cfg(unix)]{use std::os::unix::fs::MetadataExt;std::fs::metadata("/proc/self").map(|m|m.uid()).unwrap_or(u32::MAX)}
    #[cfg(not(unix))]{u32::MAX}
}
fn trusted_owner(uid:u32)->bool{if dev_mode(){uid==process_uid()}else{root_owned(uid)}}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub(crate) struct DecisionRequest {
    pub agent:String,pub user:String,pub surface:String,pub tool:String,pub args:Value,
    pub stopped:bool,pub approved:bool,pub worker:bool,
}
#[derive(Deserialize)]struct FileConfig {policy:Policy}
#[derive(Deserialize)]struct Policy {
    max_args_bytes:usize,read_tools:Vec<String>,write_tools:Vec<String>,free_read_tools:Vec<String>,free_write_tools:Vec<String>,
    read_native:Vec<String>,free_write_native:Vec<String>,ask_native:Vec<String>,write_native:Vec<String>,space_roots:Vec<PathBuf>,maintained_roots:Vec<PathBuf>,security_roots:Vec<PathBuf>,secret_roots:Vec<PathBuf>,secret_fragments:Vec<String>,never_read_fragments:Vec<String>,
}
fn config()->Result<Policy,ApiErr>{
    let p=policy_path();let m=std::fs::symlink_metadata(&p).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"Autonomy policy unavailable"))?;
    #[cfg(unix)]{use std::os::unix::fs::MetadataExt;if !m.is_file()||m.file_type().is_symlink()||!trusted_owner(m.uid())||m.mode()&0o022!=0{return Err(err(StatusCode::SERVICE_UNAVAILABLE,"Autonomy policy ownership invalid"));}}
    toml::from_str::<FileConfig>(&std::fs::read_to_string(p).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"Autonomy policy unreadable"))?).map(|v|v.policy).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"Autonomy policy invalid"))
}
/// Root ownership as this process can see it. Inside a user-service sandbox (PrivateTmp and friends) real root is an unmapped user and shows as the kernel's overflow
/// user, so both 0 and the overflow uid count as root; production files in /etc/xiar must also not be group/other writable. Local dev (XIAR_AUTONOMY_DEV) accepts the current user instead of root, never on the production policy path.
pub(crate) fn root_owned(uid:u32)->bool{
    let overflow=std::fs::read_to_string("/proc/sys/kernel/overflowuid").ok().and_then(|v|v.trim().parse::<u32>().ok()).unwrap_or(65534);
    uid==0||uid==overflow
}
fn contains(v:&[String],key:&str)->bool{v.iter().any(|s|s==key)}
fn path(args:&Value)->Option<PathBuf>{
    let raw=["path","file_path","filename","file","target"].iter().find_map(|k|args.get(k).and_then(Value::as_str))?;
    let p=PathBuf::from(raw);if !p.is_absolute()||p.components().any(|c|matches!(c,std::path::Component::ParentDir)){return None;}
    // Resolve existing ancestors for a not-yet-created target, never grant lexical symlink escapes.
    let mut base=p.as_path();let mut tail=Vec::new();
    while !base.exists(){tail.push(base.file_name()?.to_os_string());base=base.parent()?;}
    let mut real=std::fs::canonicalize(base).ok()?;for part in tail.into_iter().rev(){real.push(part);}Some(real)
}
fn under(path:&Path,roots:&[PathBuf])->bool{roots.iter().any(|r|path.starts_with(r))}
pub(crate) async fn decide(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<DecisionRequest>)->Result<Json<Value>,ApiErr>{
    super::scheduler::authenticate(&h)?;
    for value in [&b.agent,&b.user]{if !matches!(xiar_memory::tenant::BusinessId::parse(value),Ok(ref id) if id.slug()==value){return Err(err(StatusCode::BAD_REQUEST,"normalized Autonomy identity required"));}}
    if !["hermes","native","internal"].contains(&b.surface.as_str())||b.tool.is_empty()||b.tool.len()>128||!b.tool.bytes().all(|c|c.is_ascii_alphanumeric()||b"_-.:".contains(&c)){return Err(err(StatusCode::BAD_REQUEST,"invalid Autonomy operation"));}
    let policy=config()?;let raw=serde_json::to_vec(&b.args).map_err(|_|err(StatusCode::BAD_REQUEST,"invalid tool arguments"))?;
    if raw.len()>policy.max_args_bytes{return Err(err(StatusCode::PAYLOAD_TOO_LARGE,"Autonomy arguments exceed configured bound"));}
    let mut tier="ask_first";let mut reason="Unclassified or opaque operation requires explicit owner approval";let mut mutating=true;let mut forbidden=false;
    if b.tool=="xiar_emergency_stop"||b.tool=="xiar_drive_end" {tier="free";mutating=false;reason="Safety stop remains available; coordinator separately verifies standing Xiar identity";}
    else if b.surface=="native" {
        if contains(&policy.read_native,&b.tool){tier="free";mutating=false;reason="Known native read-only operation";}
        else if contains(&policy.free_write_native,&b.tool){tier="free";reason="Native business memory change stays inside backend-enforced tenant grants";}
        else if contains(&policy.ask_native,&b.tool){reason="Agent, business or group permission boundaries require explicit authenticated owner action";}
        else if contains(&policy.write_native,&b.tool){tier="logged_undoable";reason="Native managed change requires audit and pre-change snapshot";}
    }else if contains(&policy.free_read_tools,&b.tool){tier="free";mutating=false;reason="Known scoped read-only tool";}
    else if contains(&policy.free_write_tools,&b.tool){tier="free";reason="Scoped product-space operation; dispatch still enforces scope and resources";}
    else if contains(&policy.read_tools,&b.tool)||contains(&policy.write_tools,&b.tool){
        mutating=contains(&policy.write_tools,&b.tool);
        if let Some(p)=path(&b.args){
            let text=p.to_string_lossy();
            if !mutating&&policy.never_read_fragments.iter().any(|s|text.contains(s)){forbidden=true;reason="Trainer-only lesson material is never a Hermes-readable memory source";}
            else if policy.secret_fragments.iter().any(|s|text.contains(s))||under(&p,&policy.secret_roots)||mutating&&under(&p,&policy.security_roots){reason="Credentials, security settings or policy boundaries require explicit owner approval";}
            else if !mutating{tier="free";reason="Local non-secret read under the existing OS grants";}
            else if under(&p,&policy.space_roots)||b.worker&&p.starts_with("/var/lib/xiar-agent/workspace"){tier="free";reason="Write inside the scoped product workspace";}
            else if under(&p,&policy.maintained_roots){tier="logged_undoable";reason="Owner-marked maintained source requires snapshot and audit";}
            else{reason="Write outside granted product space requires explicit owner approval";}
        }else{reason="File target must be explicit, absolute and safely resolved; no relative-path grant guessed";}
    }
    let allowed=!forbidden&&!(b.stopped&&mutating)&&(tier!="ask_first"||b.approved);
    if b.stopped&&mutating{reason="Emergency stop is persistent: XIAR remains read-only until owner resumes";}
    let digest=format!("{:x}",Sha256::digest(&raw));let payload=json!({"surface":b.surface,"tool":b.tool,"tier":tier,"allowed":allowed,"mutating":mutating,"reason":reason,"args_digest":digest,"approved":b.approved});
    let session=format!("autonomy:{}",b.agent);let user=b.user;let agent=b.agent;let copy=payload.clone();
    let seq=tokio::task::spawn_blocking(move||{
        st.ledger.append(&xiar_audit::NewEntry{session:&session,user:&user,bot:&agent,kind:if allowed{"autonomy_allowed"}else{"autonomy_blocked"},tags:&["autonomy","tool_gate"],payload:copy.to_string().as_bytes(),ts_ms:xiar_audit::now_ms()}).map(|m|m.seq).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"Canonical Autonomy audit unavailable"))
    }).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"Autonomy audit task failed"))??;
    Ok(Json(json!({"allowed":allowed,"tier":tier,"mutating":mutating,"reason":reason,"forbidden":forbidden,"audit_seq":seq})))
}
static EVENT_SERIAL:std::sync::Mutex<()>=std::sync::Mutex::new(());
#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub(crate) struct Event {event_id:String,agent:String,user:String,event:String,request_id:String,snapshot:Option<u64>,linked_audit:Option<u64>}
pub(crate) async fn event(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<Event>)->Result<Json<Value>,ApiErr>{
    super::scheduler::authenticate(&h)?;
    if b.event_id.len()!=32||!b.event_id.bytes().all(|v|v.is_ascii_hexdigit()){return Err(err(StatusCode::BAD_REQUEST,"invalid event identity"));}
    if !["emergency_stop","owner_resume","owner_approve","owner_deny","tool_completed","tool_failed","undo","undo_completed"].contains(&b.event.as_str())||!b.request_id.is_empty()&&(b.request_id.len()!=32||!b.request_id.bytes().all(|v|v.is_ascii_hexdigit())){return Err(err(StatusCode::BAD_REQUEST,"invalid Autonomy event"));}
    for value in [&b.agent,&b.user]{if !matches!(xiar_memory::tenant::BusinessId::parse(value),Ok(ref id) if id.slug()==value){return Err(err(StatusCode::BAD_REQUEST,"normalized Autonomy identity required"));}}
    let seq=tokio::task::spawn_blocking(move||{let session=format!("autonomy-event:{}",b.event_id);let kind=format!("autonomy_{}",b.event);let mut payload=json!({"request_id":b.request_id,"snapshot":b.snapshot,"linked_audit":b.linked_audit});if b.event=="undo_completed"{payload["undo_of"]=json!(b.request_id);payload["undone_by"]=json!(b.event_id);}let _serial=EVENT_SERIAL.lock().map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"Autonomy event lock unavailable"))?;let entries=st.ledger.session_entries(&session).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"Autonomy event lookup failed"))?;if let Some(old)=entries.first(){if old.user!=b.user||old.bot!=b.agent||old.kind!=kind||st.ledger.read_payload(old.seq).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"Autonomy event payload unavailable"))?!=payload.to_string().as_bytes(){return Err(err(StatusCode::CONFLICT,"Autonomy event identity changed"));}return Ok(old.seq);}st.ledger.append(&xiar_audit::NewEntry{session:&session,user:&b.user,bot:&b.agent,kind:&kind,tags:&["autonomy","lifecycle"],payload:payload.to_string().as_bytes(),ts_ms:xiar_audit::now_ms()}).map(|m|m.seq).map_err(|_|err(StatusCode::SERVICE_UNAVAILABLE,"Canonical Autonomy event unavailable"))}).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"Autonomy event task failed"))??;
    Ok(Json(json!({"audit_seq":seq})))
}

#[cfg(unix)]fn authoritative_stop()->Option<bool>{
    use std::io::{Read,Write};use std::os::unix::{fs::MetadataExt,net::UnixStream};
    #[derive(Deserialize)]struct Control {broker_socket:PathBuf,state_probe_timeout_s:f64}
    #[derive(Deserialize)]struct Config {control:Control}
    // config() validates the same root-owned policy before this secondary decode.
    config().ok()?;let cfg:Config=toml::from_str(&std::fs::read_to_string(policy_path()).ok()?).ok()?;
    if !cfg.control.state_probe_timeout_s.is_finite()||cfg.control.state_probe_timeout_s<=0.0{return None;}
    let sock=resolve_cfg_path(cfg.control.broker_socket);
    let meta=std::fs::symlink_metadata(&sock).ok()?;
    if !trusted_owner(meta.uid())||meta.file_type().is_symlink()||meta.mode()&0o002!=0{return None;}
    let mut stream=UnixStream::connect(sock).ok()?;let timeout=Some(std::time::Duration::try_from_secs_f64(cfg.control.state_probe_timeout_s).ok()?);stream.set_read_timeout(timeout).ok()?;stream.set_write_timeout(timeout).ok()?;
    stream.write_all(b"POST /internal/emergency HTTP/1.0\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}").ok()?;
    let mut raw=Vec::new();stream.take(65537).read_to_end(&mut raw).ok()?;if raw.len()>65536{return None;}
    let split=raw.windows(4).position(|v|v==b"\r\n\r\n")?;let headers=std::str::from_utf8(&raw[..split]).ok()?;
    if headers.lines().next()?.split_whitespace().nth(1)?!="200"{return None;}
    serde_json::from_slice::<Value>(&raw[split+4..]).ok()?["stopped"].as_bool()
}
/// Test-only override of the stop state (0 = real state, 1 = stopped, 2 = running). It exists ONLY when the crate is built with the `test-hooks` feature, which no
/// release or production build enables: the shipped binary cannot be told to ignore the root-owned stop state.
#[cfg(feature="test-hooks")]
static OVERRIDE:std::sync::atomic::AtomicU8=std::sync::atomic::AtomicU8::new(0);
#[cfg(feature="test-hooks")]
pub fn force_stop_for_tests(stopped:bool){OVERRIDE.store(if stopped{1}else{2},std::sync::atomic::Ordering::SeqCst);}
static IN_FLIGHT:std::sync::atomic::AtomicUsize=std::sync::atomic::AtomicUsize::new(0);
/// Held by every memory mutation from the moment it is admitted until it has fully finished (committed or refused); the broker reads the count to know when the
/// stop has really quiesced the memory.
pub(crate) struct InFlight;
impl InFlight{pub(crate) fn enter()->Self{IN_FLIGHT.fetch_add(1,std::sync::atomic::Ordering::SeqCst);InFlight}}
impl Drop for InFlight{fn drop(&mut self){IN_FLIGHT.fetch_sub(1,std::sync::atomic::Ordering::SeqCst);}}
pub fn mutations_in_flight()->usize{IN_FLIGHT.load(std::sync::atomic::Ordering::SeqCst)}
/// The commit barrier: called right before a mutation's write, AFTER any wait for the scope lock (a request that queued behind a long operation before the stop
/// must not commit after it). A mutation that is already writing finishes; one that has not started its write is refused.
pub(crate) fn commit_check()->Result<(),ApiErr>{
    if stopped(){Err(err(StatusCode::LOCKED,"Emergency stop: memory mutation refused at its commit point"))}else{Ok(())}
}
pub(crate) async fn quiesce(State(_st):State<Arc<AppState>>,h:HeaderMap)->Result<Json<Value>,ApiErr>{
    super::scheduler::authenticate(&h)?;
    Ok(Json(json!({"in_flight":mutations_in_flight(),"stopped":stopped()})))
}
const MUTATING:&[&str]=&["/v1/knowledge/remove","/v1/jobs/enqueue","/v1/jobs/run","/v1/semantic/delete","/v1/semantic/insert","/v1/semantic/reindex"];
pub(crate) fn stopped()->bool{
    #[cfg(feature="test-hooks")]{match OVERRIDE.load(std::sync::atomic::Ordering::SeqCst){1=>return true,2=>return false,_=>{}}}
    let p=state_path();
    let Ok(meta)=std::fs::symlink_metadata(&p) else{return true;};
    #[cfg(unix)]{use std::os::unix::fs::MetadataExt;if !meta.is_file()||meta.file_type().is_symlink()||!trusted_owner(meta.uid())||meta.mode()&0o022!=0{return true;}}
    let mirror=std::fs::read_to_string(&p).ok().and_then(|v|serde_json::from_str::<Value>(&v).ok()).and_then(|v|v["stopped"].as_bool()).unwrap_or(true);
    if mirror{return true;}
    // A failed public-mirror write must not override the broker's durable state or stop latch.
    #[cfg(unix)]{authoritative_stop().unwrap_or(true)}
    #[cfg(not(unix))]{true}
}
pub(crate) async fn memory_stop_guard(req:axum::extract::Request,next:axum::middleware::Next)->axum::response::Response{
    use axum::response::IntoResponse;
    let path=req.uri().path();
    let mutating=MUTATING.contains(&path);
    if mutating&&stopped(){
        return (StatusCode::LOCKED,Json(json!({"error":"Emergency stop: memory mutations paused until owner resumes"}))).into_response();
    }
    let _admitted=if mutating{Some(InFlight::enter())}else{None};
    next.run(req).await
}
