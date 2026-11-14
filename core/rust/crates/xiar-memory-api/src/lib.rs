// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! xiar-memory-api: HTTP front for tenant-scoped MIRIX Semantic memory.
//!
//! * `POST /v1/semantic/search` `{query, limit?, ontology?}` and `POST /v1/semantic/insert`
//!   `{name, summary, details?, source?, entity_types?}`; both need `Authorization: Bearer <client token>`
//!   and `X-Business-Id: <slug>`. Missing id -> 400, business not granted to the client -> 403.
//! * Storage is `xiar_memory::tenant::TenantRegistry` (one physically separate store per business).
//! * Ontology expansion uses the promoted ontology through `OntologyService::expand_query` (OntologyExpander).
//! * Config: `config/memory_api.toml` (paths relative to `$XIAR_ROOT`); tokens live in a 0600 secrets file.

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xiar_memory::manager::Sealer;
use xiar_memory::ontology_retrieve::FnExpander;
use xiar_memory::query_expand::QueryExpander;
mod scope;
mod scheduler;
mod autonomy;
/// Test hooks only ever make the service stop MORE: forcing the stopped answer, and counting admitted mutations.
pub use autonomy::mutations_in_flight;
#[cfg(feature = "test-hooks")]
pub use autonomy::force_stop_for_tests;
mod dense;
mod jobs;
use scope::{Scope, ScopedStores};
use xiar_memory::types::SemanticItem;
use xiar_ontology::critic::Critic;
use xiar_ontology::models::Status;
use xiar_ontology::registry::Registry;
use xiar_ontology::service::OntologyService;
use xiar_prompts::PromptStore;
use xiar_store::keyring::{Keyring, MasterKey};
use xiar_store::relational::RedbStore;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerCfg,
    pub store: StoreCfg,
    #[serde(default)]
    pub ontology: OntologyCfg,
    pub auth: AuthCfg,
    #[serde(default)]
    pub clients: Vec<ClientCfg>,
    #[serde(default)]
    pub search: SearchCfg,
}
#[derive(Debug, Clone, Deserialize)]
pub struct ServerCfg {
    pub bind: String,
    pub max_body_bytes: usize,
}
#[derive(Debug, Clone, Deserialize)]
pub struct StoreCfg {
    pub root: String,
    pub language: String,
    pub user: String,
    pub master_key_file: String,
}
#[derive(Debug, Clone, Deserialize, Default)]
pub struct OntologyCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub ofn: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct AuthCfg {
    pub tokens_file: String,
    #[serde(default = "default_dynamic_grants")]
    pub dynamic_grants_file: String,
}
fn default_dynamic_grants() -> String { "data/agents/memory-grants.json".into() }

#[derive(Debug, Clone, Deserialize)]
pub struct ClientCfg {
    pub name: String,
    /// business slugs this client may use; `"*"` = any
    pub businesses: Vec<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub end_user_id: Option<String>,
    #[serde(default)]
    pub read_all_agents: bool,
}
#[derive(Debug, Clone, Deserialize)]
pub struct SearchCfg {
    pub default_limit: usize,
    pub max_limit: usize,
}
impl Default for SearchCfg {
    fn default() -> Self {
        Self { default_limit: 8, max_limit: 50 }
    }
}

pub fn xiar_root() -> PathBuf {
    std::env::var_os("XIAR_ROOT").map(PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

pub fn resolve(root: &Path, p: &str) -> PathBuf {
    let p = if let Some(rest) = p.strip_prefix("~/") {
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest)
    } else {
        PathBuf::from(p)
    };
    if p.is_absolute() { p } else { root.join(p) }
}

pub fn load_config(path: &Path) -> Result<Config, String> {
    let txt = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&txt).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, data).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Create the master key and one token per configured client if missing. Never prints secrets.
pub fn init(cfg: &Config, root: &Path) -> Result<Vec<String>, String> {
    use base64::Engine;
    let mut made = vec![];
    let mk = resolve(root, &cfg.store.master_key_file);
    if !mk.exists() {
        let k = MasterKey::generate();
        write_private(&mk, base64::engine::general_purpose::STANDARD.encode(k.as_bytes()).as_bytes())?;
        made.push(format!("master key {}", mk.display()));
    }
    let tf = resolve(root, &cfg.auth.tokens_file);
    let mut toks: BTreeMap<String, String> = std::fs::read_to_string(&tf).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut changed = false;
    for c in &cfg.clients {
        if !toks.contains_key(&c.name) {
            toks.insert(c.name.clone(), format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple()));
            made.push(format!("token for client {:?}", c.name));
            changed = true;
        }
    }
    if changed {
        write_private(&tf, serde_json::to_string_pretty(&toks).map_err(|e| e.to_string())?.as_bytes())?;
    }
    Ok(made)
}

fn load_master(path: &Path) -> Result<MasterKey, String> {
    use base64::Engine;
    let raw = std::fs::read_to_string(path).map_err(|e| format!("master key {}: {e} (run `xiar-memory-api init`)", path.display()))?;
    let b = base64::engine::general_purpose::STANDARD.decode(raw.trim()).map_err(|e| e.to_string())?;
    let arr: [u8; 32] = b.try_into().map_err(|_| "master key must be 32 bytes".to_string())?;
    Ok(MasterKey::from_bytes(arr))
}

pub fn ontology_expander(ofn: &Path) -> Result<FnExpander, String> {
    let text = std::fs::read_to_string(ofn).map_err(|e| format!("{}: {e}", ofn.display()))?;
    let reg = Arc::new(Registry::new(Arc::new(RedbStore::in_memory().map_err(|e| e.to_string())?)));
    reg.register_text(&text, "memory-api", false, Some("memory-api live search".into()), Status::Promoted).map_err(|e| e.to_string())?;
    let svc = Arc::new(OntologyService::new(reg, Critic::new(None, Arc::new(PromptStore::new(None, xiar_ontology::BUNDLED)))));
    Ok(FnExpander(Arc::new(move |q: &str| svc.expand_query(q).map(|e| e.expanded_terms).unwrap_or_default())))
}

#[derive(Clone)]
pub struct ClientGrant {
    pub token: String,
    /// true: `token` holds the SHA-256 (hex) of the capability, not the capability itself (private-Agent grants, registry version 2)
    pub hashed: bool,
    pub agent: String,
    pub user: String,
    pub businesses: Vec<String>,
    pub read_all: bool,
}

pub struct AppState {
    pub dense: dense::DenseService,
    pub jobs: jobs::Jobs,
    pub ledger: Arc<xiar_audit::Ledger>,
    pub core: Arc<xiar_memory::core::CoreMemory>,
    pub tenants: ScopedStores,
    pub expander: Option<Box<dyn QueryExpander>>,
    /// token -> allowed businesses
    pub tokens: Vec<ClientGrant>,
    pub dynamic_grants_file: PathBuf,
    pub search: SearchCfg,
}

#[derive(Deserialize)]
struct DynamicGrants { version: u32, grants: Vec<DynamicGrant> }
#[derive(Deserialize)]
struct DynamicGrant { #[serde(default)] token: String, #[serde(default)] token_sha256: String, agent: String, user: String, businesses: Vec<String> }

impl AppState {
    /// Reload on EACH authorization and queue execution: disabling a scope must
    /// revoke its capability immediately, without restarting or widening owner keys.
    pub fn grants(&self) -> Result<Vec<ClientGrant>, String> {
        let mut grants = self.tokens.clone();
        let metadata = match std::fs::symlink_metadata(&self.dynamic_grants_file) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(grants),
            // The registry folder is root-only while no private Agent is connected, so this service cannot even look. Nothing private can be granted
            // then: only the static owner/service grants apply and every private token is refused (fails closed). Warn once. A registry that exists
            // but is invalid, or has the wrong mode, is still an error.
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) { eprintln!("private grant registry not readable: no private Agent grants are active"); }
                return Ok(grants);
            }
            Err(_) => return Err("private grant registry unavailable".into()),
        };
        if !metadata.is_file() || metadata.len() > 4 * 1024 * 1024 { return Err("private grant registry invalid".into()); }
        let data: DynamicGrants = serde_json::from_slice(&std::fs::read(&self.dynamic_grants_file).map_err(|_|"private grant registry unavailable")?)
            .map_err(|_|"private grant registry invalid")?;
        if !matches!(data.version, 1 | 2) || data.grants.len() > 8192 { return Err("unknown grant registry format".into()); }
        #[cfg(unix)] {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let mode = metadata.permissions().mode();
            if data.version == 1 {
                // plaintext capabilities: only the owner may even read the file
                if mode & 0o077 != 0 { return Err("private grant registry permissions invalid".into()); }
            } else {
                // hashed capabilities: readable by this service, but written only by a DIFFERENT user (the root broker), so nothing running as this
                // service's user can add or change a grant. Ownership is compared with this process's own user, not with uid 0: inside a user-service
                // sandbox (PrivateTmp and friends) real root appears as the overflow user. The directory must likewise belong to another user.
                let me = std::fs::metadata("/proc/self").map(|m| m.uid()).map_err(|_| "cannot determine the service user")?;
                let parent_ok = self.dynamic_grants_file.parent().and_then(|p| std::fs::symlink_metadata(p).ok())
                    .is_some_and(|p| p.is_dir() && p.uid() != me && p.permissions().mode() & 0o022 == 0);
                if metadata.uid() == me || mode & 0o022 != 0 || !parent_ok {
                    return Err("private grant registry must belong to another user, not be writable by others, in a directory likewise".into());
                }
            }
        }
        for g in data.grants {
            let (value, hashed) = if data.version == 2 {
                if !g.token.is_empty() { return Err("plaintext capability in a hashed registry".into()); }
                (g.token_sha256.clone(), true)
            } else { (g.token.clone(), false) };
            if value.len() != 64 || !value.bytes().all(|b|b.is_ascii_hexdigit()) || grants.iter().any(|a|a.hashed==hashed && ct_eq(a.token.as_bytes(),value.as_bytes())) {
                return Err("invalid or duplicate private capability".into());
            }
            for identity in [&g.agent,&g.user] {
                if xiar_memory::tenant::BusinessId::parse(identity).map_err(|_|"invalid private identity")?.slug() != identity.as_str() {return Err("identity must be normalized".into());}
            }
            if g.businesses.is_empty() || g.businesses.len() > 128 {return Err("invalid private business grant".into());}
            for b in &g.businesses {
                if xiar_memory::tenant::BusinessId::parse(b).map_err(|_|"invalid private business grant")?.slug()!=b.as_str() {return Err("business must be normalized".into());}
            }
            let read_all=g.agent=="xiar";
            grants.push(ClientGrant {token:value.to_ascii_lowercase(),hashed,agent:g.agent,user:g.user,businesses:g.businesses,read_all});
        }
        Ok(grants)
    }
    fn scoped_ontology(&self,scope:&Scope)->Result<Option<OntologyService>,String>{
        let path=self.tenants.ontology_path(scope)?;if !path.is_file(){return Ok(None);}
        let text=std::fs::read_to_string(path).map_err(|_|"scoped ontology unavailable")?;
        let reg=Arc::new(Registry::new(Arc::new(RedbStore::in_memory().map_err(|_|"ontology registry failed")?)));
        reg.register_text(&text,"memory-api",false,None,Status::Promoted).map_err(|_|"promoted ontology invalid")?;
        Ok(Some(OntologyService::new(reg,Critic::new(None,Arc::new(PromptStore::new(None,xiar_ontology::BUNDLED))))))
    }
    pub fn validated_types(&self,scope:&Scope,name:&str,types:Vec<String>)->Result<Vec<String>,String>{
        if types.is_empty(){return Ok(types);}
        let service=self.scoped_ontology(scope)?.ok_or("type claims require this scope's promoted ontology")?;
        let known=service.class_names().map_err(|_|"ontology class list failed")?;
        let asserted=types.into_iter().filter(|t|known.contains(t)).collect::<Vec<_>>();
        service.entity_types(&[(name.into(),asserted.clone())]).map_err(|_|"entity type claims contradict the promoted ontology")?;Ok(asserted)
    }
    pub fn from_config(cfg: &Config, root: &Path) -> Result<Self, String> {
        let sroot = resolve(root, &cfg.store.root);
        std::fs::create_dir_all(&sroot).map_err(|e| e.to_string())?;
        let rel = Arc::new(RedbStore::open(&sroot.join("keyring.redb")).map_err(|e| e.to_string())?);
        let keyring = Arc::new(Keyring::new(rel.clone(), load_master(&resolve(root, &cfg.store.master_key_file))?));
        let core=Arc::new(xiar_memory::core::CoreMemory::new(rel.clone(),keyring.clone()));
        let ledger = Arc::new(xiar_audit::Ledger::new(rel, keyring.clone()));
        let ring: Arc<dyn Sealer> = keyring;
        let dense=dense::DenseService::new(root,&cfg.store.language,ring.clone());
        let jobs=jobs::Jobs::new(root,ring.clone())?;
        let tenants = ScopedStores::new(sroot, cfg.store.language.clone(), ring);
        let expander: Option<Box<dyn QueryExpander>> = if cfg.ontology.enabled && !cfg.ontology.ofn.is_empty() {
            Some(Box::new(ontology_expander(&resolve(root, &cfg.ontology.ofn))?))
        } else {
            None
        };
        let tf = resolve(root, &cfg.auth.tokens_file);
        let map: HashMap<String, String> = serde_json::from_str(&std::fs::read_to_string(&tf).map_err(|e| format!("{}: {e} (run init)", tf.display()))?)
            .map_err(|e| e.to_string())?;
        let tokens = cfg
            .clients
            .iter()
            .filter_map(|c| map.get(&c.name).map(|t| ClientGrant {
                token: t.clone(), hashed: false, agent: c.agent_id.clone().unwrap_or_else(|| c.name.clone()),
                user: c.end_user_id.clone().unwrap_or_else(|| cfg.store.user.clone()),
                businesses: c.businesses.iter().map(|b| b.trim().to_lowercase()).collect(),
                read_all: c.read_all_agents && c.agent_id.as_deref() == Some("xiar"),
            }))
            .collect();
        Ok(Self { core, jobs, dense, tenants, expander, tokens, dynamic_grants_file: resolve(root, &cfg.auth.dynamic_grants_file), ledger, search: cfg.search.clone() })
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

type ApiErr = (StatusCode, Json<Value>);
fn err(code: StatusCode, msg: impl Into<String>) -> ApiErr {
    (code, Json(json!({ "error": msg.into() })))
}

/// Authenticate + authorise one business. Returns the normalised slug.
fn authorize(st: &AppState, h: &HeaderMap, read: bool) -> Result<Scope, ApiErr> {
    let tok = h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    let grants = st.grants().map_err(|e|{ eprintln!("private grant registry refused: {e}"); err(StatusCode::SERVICE_UNAVAILABLE,"private grant registry unavailable or invalid") })?;
    let presented_hash = format!("{:x}", Sha256::digest(tok.as_bytes()));
    let Some(grant) = grants.iter().find(|g| !tok.is_empty() && if g.hashed { ct_eq(g.token.as_bytes(), presented_hash.as_bytes()) } else { ct_eq(g.token.as_bytes(), tok.as_bytes()) }) else {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid token"));
    };
    let header = |name: &str| h.get(name).and_then(|v|v.to_str().ok());
    if header("x-agent-id").is_some_and(|v|v != grant.agent) || header("x-end-user-id").is_some_and(|v|v != grant.user) {
        return Err(err(StatusCode::FORBIDDEN, "caller identity does not match token"));
    }
    let b = xiar_memory::tenant::BusinessId::require(header("x-business-id")).map_err(|e|err(StatusCode::BAD_REQUEST,e.to_string()))?;
    if !grant.businesses.iter().any(|a| a == "*" || a == b.slug()) {
        return Err(err(StatusCode::FORBIDDEN, "business not granted"));
    }
    let agent = header("x-read-agent-id").unwrap_or(&grant.agent);
    let user = header("x-read-user-id").unwrap_or(&grant.user);
    if agent != grant.agent || user != grant.user {
        if !read || !grant.read_all || grant.agent != "xiar" {
            return Err(err(StatusCode::FORBIDDEN, "cross-scope access refused"));
        }
    }
    for v in [agent,user] {
        let slug=xiar_memory::tenant::BusinessId::parse(v).map_err(|e|err(StatusCode::BAD_REQUEST,e.to_string()))?;
        if slug.slug()!=v { return Err(err(StatusCode::BAD_REQUEST,"identity must be normalized")); }
    }
    let scope=Scope {agent:agent.into(),user:user.into(),business:b.slug().into()};
    if (agent!=grant.agent||user!=grant.user)&&!st.tenants.is_bound(&scope){
        return Err(err(StatusCode::NOT_FOUND,"target memory scope does not exist; read-only access cannot create it"));
    }
    Ok(scope)
}

#[derive(Deserialize)]
struct SearchBody {
    query: String,
    limit: Option<usize>,
    ontology: Option<bool>,
    method: Option<String>,
}

#[derive(Deserialize)]
struct InsertBody {
    name: String,
    summary: String,
    #[serde(default)]
    details: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    entity_types: Vec<String>,
}

async fn search(State(st): State<Arc<AppState>>, h: HeaderMap, Json(b): Json<SearchBody>) -> Result<Json<Value>, ApiErr> {
    let scope = authorize(&st, &h, true)?;
    let biz = scope.business.clone();
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        let lock=st2.tenants.operation_lock(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let _read=lock.read().map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scope lock poisoned"))?;
        let dir=st2.tenants.directory(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let policy=st2.dense.policy().map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let use_ontology=policy.ontology&&b.ontology.unwrap_or(true);
        let method=b.method.as_deref().unwrap_or(&policy.method);
        if !["bm25","embedding","hybrid"].contains(&method){return Err(err(StatusCode::BAD_REQUEST,"unknown retrieval method"));}
        let t = st2.tenants.open(&scope).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
        let limit = b.limit.unwrap_or(st2.search.default_limit).clamp(1, st2.search.max_limit);
        let path=st2.tenants.ontology_path(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let scoped=if use_ontology&&path.exists(){Some(ontology_expander(&path).map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e))?)}else{None};
        let exp:Option<&dyn QueryExpander>=if use_ontology{scoped.as_ref().map(|x|x as &dyn QueryExpander)}else{None};
        let hits = st2.dense.search(&dir,&scope,&t,&b.query,method,limit,exp).map_err(|e| err(StatusCode::CONFLICT, e))?;
        let hits: Vec<Value> = hits
            .into_iter()
            .map(|s| json!({"id": s.item.common.id, "kind":"semantic", "name": s.item.name, "summary": s.item.summary,
                            "entity_types": s.item.entity_types, "source": s.item.source, "score": s.score}))
            .collect();
        Ok(Json(json!({"agent_id": scope.agent, "end_user_id": scope.user, "business_id": biz, "ontology": exp.is_some(), "method":method, "method_source":if b.method.is_some(){"request"}else{"memory_settings"}, "ontology_reason":if !policy.ontology{"disabled_in_memory_settings"}else if !b.ontology.unwrap_or(true){"disabled_by_caller"}else if exp.is_some(){"scoped_promoted_ontology"}else{"no_scoped_promoted_ontology"}, "hits": hits})))
    })
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
}

async fn insert(State(st): State<Arc<AppState>>, h: HeaderMap, Json(b): Json<InsertBody>) -> Result<(StatusCode, Json<Value>), ApiErr> {
    let scope = authorize(&st, &h, false)?;
    let biz = scope.business.clone();
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || {
        let lock=st2.tenants.operation_lock(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let _write=lock.write().map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scope lock poisoned"))?;
        let dir=st2.tenants.directory(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let t = st2.tenants.open(&scope).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
        let before=t.list_recent(0).map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?;
        let mut item = SemanticItem::new(&scope.user, &b.name, &b.summary, &b.details, &b.source);
        item.entity_types = st2.validated_types(&scope,&item.name,b.entity_types).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let intent=json!({"agent_id":scope.agent,"business_id":scope.business,"end_user_id":scope.user,"id":item.common.id}).to_string();
        st2.ledger.append(&xiar_audit::NewEntry{session:"memory-api",user:&scope.user,bot:&scope.agent,kind:"semantic_insert_requested",tags:&["memory","semantic"],payload:intent.as_bytes(),ts_ms:xiar_audit::now_ms()}).map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"Insert audit failed; no Semantic item written"))?;
        autonomy::commit_check()?;
        let saved = t.insert(item, BTreeMap::new()).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
        let mut after=before.clone();after.push(saved.clone());
        let (indexed,index_reason)=match st2.dense.record(&dir,&scope,&before,&after,&saved){
            Ok(true)=>(true,"indexed".to_string()),
            Ok(false)=>(false,if st2.dense.policy().is_ok_and(|p|p.model.is_empty()){"embedding_not_selected"}else{"reindex_required"}.to_string()),
            Err(reason)=>(false,reason),
        };
        let audit=json!({"agent_id":scope.agent,"business_id":scope.business,"end_user_id":scope.user,"id":saved.common.id,"indexed":indexed,"reason":index_reason}).to_string();
        let audited=st2.ledger.append(&xiar_audit::NewEntry{session:"memory-api",user:&scope.user,bot:&scope.agent,kind:"semantic_embedding_decision",tags:&["memory","embedding"],payload:audit.as_bytes(),ts_ms:xiar_audit::now_ms()}).is_ok();
        if !audited{eprintln!("Semantic saved under recorded insert intent; completion audit requires reconciliation");}
        Ok((StatusCode::CREATED, Json(json!({"business_id": biz, "id": saved.common.id,"embedding_indexed":indexed,"embedding_reason":index_reason,"embedding_decision_audited":audited}))))
    })
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
}

#[derive(Deserialize)]
struct GetBody { id: String }
async fn get_item(State(st): State<Arc<AppState>>, h: HeaderMap, Json(b): Json<GetBody>) -> Result<Json<Value>, ApiErr> {
    let scope = authorize(&st, &h, true)?;
    tokio::task::spawn_blocking(move || {
        let t=st.tenants.open(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let item=t.get(&b.id).map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?
            .ok_or_else(||err(StatusCode::NOT_FOUND,"item not found in caller scope"))?;
        Ok(Json(json!({"agent_id":scope.agent,"business_id":scope.business,"end_user_id":scope.user,"item":item})))
    }).await.map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?
}

#[derive(Deserialize)]
#[serde(rename_all="snake_case")]
enum OntologyAuditAction { Started, Completed }
#[derive(Deserialize)]
struct OntologyAuditBody { action:OntologyAuditAction, version:Option<String>, promoted:Option<bool> }
async fn ontology_audit(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<OntologyAuditBody>)->Result<Json<Value>,ApiErr>{
    let scope=authorize(&st,&h,false)?;
    if b.version.as_ref().is_some_and(|v|v.len()<2||v.len()>64||!v.starts_with('v')||!v[1..].bytes().all(|x|x.is_ascii_digit())){return Err(err(StatusCode::BAD_REQUEST,"invalid ontology version"));}
    tokio::task::spawn_blocking(move||{
        st.tenants.open(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let action=match b.action{OntologyAuditAction::Started=>"ontology_build_started",OntologyAuditAction::Completed=>"ontology_build_completed"};
        let payload=json!({"agent_id":scope.agent,"end_user_id":scope.user,"business_id":scope.business,"version":b.version,"promoted":b.promoted}).to_string();
        st.ledger.append(&xiar_audit::NewEntry{session:"settings",user:&scope.user,bot:&scope.agent,kind:action,tags:&["memory","ontology"],payload:payload.as_bytes(),ts_ms:xiar_audit::now_ms()}).map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"ontology audit append failed"))?;
        Ok(Json(json!({"recorded":true})))
    }).await.map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?
}

#[derive(Deserialize)]
struct ListBody { #[serde(default)] query: String, limit: Option<usize> }
async fn list_items(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<ListBody>)->Result<Json<Value>,ApiErr>{
    let scope=authorize(&st,&h,true)?;
    tokio::task::spawn_blocking(move||{
        let lock=st.tenants.operation_lock(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let _read=lock.read().map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scope lock poisoned"))?;
        let dir=st.tenants.directory(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let policy=st.dense.policy().map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let t=st.tenants.open(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let limit=b.limit.unwrap_or(st.search.default_limit).clamp(1,st.search.max_limit);
        let path=st.tenants.ontology_path(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let scoped=if policy.ontology&&path.exists(){Some(ontology_expander(&path).map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e))?)}else{None};
        let exp:Option<&dyn QueryExpander>=scoped.as_ref().map(|x|x as &dyn QueryExpander);
        let items=if b.query.trim().is_empty(){t.list_recent(limit).map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?}
            else{st.dense.search(&dir,&scope,&t,&b.query,&policy.method,limit,exp).map_err(|e|err(StatusCode::CONFLICT,e))?.into_iter().map(|h|h.item).collect()};
        Ok(Json(json!({"agent_id":scope.agent,"end_user_id":scope.user,"business_id":scope.business,"items":items,"total":t.count().map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?})))
    }).await.map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?
}
async fn delete_item(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<GetBody>)->Result<Json<Value>,ApiErr>{
    let scope=authorize(&st,&h,false)?;
    tokio::task::spawn_blocking(move||{
        let lock=st.tenants.operation_lock(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let _write=lock.write().map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scope lock poisoned"))?;
        let dir=st.tenants.directory(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let t=st.tenants.open(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        if t.get(&b.id).map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?.is_none(){return Err(err(StatusCode::NOT_FOUND,"item not found in caller scope"));}
        autonomy::commit_check()?;
        let audit=json!({"agent_id":scope.agent,"business_id":scope.business,"end_user_id":scope.user,"id":b.id,"action":"delete_requested"}).to_string();
        st.ledger.append(&xiar_audit::NewEntry{session:"settings",user:&scope.user,bot:&scope.agent,kind:"memory_delete_requested",tags:&["memory","semantic"],payload:audit.as_bytes(),ts_ms:xiar_audit::now_ms()})
            .map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"audit append failed; item not deleted"))?;
        let before=t.list_recent(0).map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?;
        let removed=t.delete(&b.id).map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?;
        let after=before.iter().filter(|item|item.common.id!=b.id).cloned().collect::<Vec<_>>();
        st.dense.erase(&dir,&scope,&b.id,&before,&after).map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,format!("Primary deleted but derived index cleanup failed: {e}")))?;
        st.ledger.append(&xiar_audit::NewEntry{session:"settings",user:&scope.user,bot:&scope.agent,kind:"memory_deleted",tags:&["memory","semantic"],payload:audit.as_bytes(),ts_ms:xiar_audit::now_ms()})
            .map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"deletion succeeded but completion audit append failed"))?;
        Ok(Json(json!({"deleted":removed})))
    }).await.map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?
}

async fn reindex(State(st):State<Arc<AppState>>,h:HeaderMap)->Result<Json<Value>,ApiErr>{
    let scope=authorize(&st,&h,false)?;
    tokio::task::spawn_blocking(move||{
        let lock=st.tenants.operation_lock(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let _write=lock.write().map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"scope lock poisoned"))?;
        let dir=st.tenants.directory(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        let t=st.tenants.open(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
        autonomy::commit_check()?;
        let payload=json!({"agent_id":scope.agent,"end_user_id":scope.user,"business_id":scope.business}).to_string();
        st.ledger.append(&xiar_audit::NewEntry{session:"settings",user:&scope.user,bot:&scope.agent,kind:"semantic_reindex_started",tags:&["memory","embedding"],payload:payload.as_bytes(),ts_ms:xiar_audit::now_ms()}).map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"index audit failed; no reindex started"))?;
        let items=t.list_recent(0).map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?;
        let result=st.dense.reindex(&dir,&scope,&items).map_err(|e|err(if e=="autonomy_paused"{StatusCode::LOCKED}else{StatusCode::SERVICE_UNAVAILABLE},e))?;
        st.ledger.append(&xiar_audit::NewEntry{session:"settings",user:&scope.user,bot:&scope.agent,kind:"semantic_reindex_completed",tags:&["memory","embedding"],payload:payload.as_bytes(),ts_ms:xiar_audit::now_ms()}).map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"index built but completion audit failed"))?;
        Ok(Json(result))
    }).await.map_err(|e|err(StatusCode::INTERNAL_SERVER_ERROR,e.to_string()))?
}

pub fn start_population(st:&Arc<AppState>){jobs::Jobs::start(st);}
#[derive(Deserialize)]
struct EnqueueBody{job:String,text:String,source:String,idempotency_key:Option<String>}
async fn enqueue_job(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<EnqueueBody>)->Result<(StatusCode,Json<Value>),ApiErr>{
 let scope=authorize(&st,&h,false)?;
 tokio::task::spawn_blocking(move||{
  st.tenants.open(&scope).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
  autonomy::commit_check()?;
  let id=st.jobs.enqueue(scope,&b.job,&b.text,&b.source,b.idempotency_key.as_deref()).map_err(|e|err(StatusCode::BAD_REQUEST,e))?;
  Ok((StatusCode::ACCEPTED,Json(json!({"queued":true,"id":id}))))
 }).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"enqueue failed"))?
}
async fn jobs_state(State(st):State<Arc<AppState>>,h:HeaderMap)->Result<Json<Value>,ApiErr>{
 let scope=authorize(&st,&h,true)?;
 tokio::task::spawn_blocking(move||st.jobs.state(&scope).map(Json).map_err(|e|err(StatusCode::SERVICE_UNAVAILABLE,e))).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"queue state failed"))?
}
async fn jobs_run(State(st):State<Arc<AppState>>,h:HeaderMap)->Result<Json<Value>,ApiErr>{
 let scope=authorize(&st,&h,false)?;
 tokio::task::spawn_blocking(move||{
  autonomy::commit_check()?;
  let ran=st.jobs.run(&st,Some(&scope),true).map_err(|e|err(StatusCode::SERVICE_UNAVAILABLE,e))?;
  let mut state=st.jobs.state(&scope).map_err(|e|err(StatusCode::SERVICE_UNAVAILABLE,e))?;state["ran"]=json!(ran);Ok(Json(state))
 }).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"queue run failed"))?
}
async fn knowledge_list(State(st):State<Arc<AppState>>,h:HeaderMap)->Result<Json<Value>,ApiErr>{
 let scope=authorize(&st,&h,true)?;tokio::task::spawn_blocking(move||st.jobs.knowledge_list(&st,&scope).map(Json).map_err(|e|err(StatusCode::BAD_REQUEST,e))).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"document list failed"))?
}
#[derive(Deserialize)]struct RemoveDocument{id:String}
async fn knowledge_remove(State(st):State<Arc<AppState>>,h:HeaderMap,Json(b):Json<RemoveDocument>)->Result<Json<Value>,ApiErr>{
 let scope=authorize(&st,&h,false)?;tokio::task::spawn_blocking(move||st.jobs.knowledge_remove(&st,&scope,&b.id).map(Json).map_err(|e|err(StatusCode::BAD_REQUEST,e))).await.map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"document deletion failed"))?
}
pub fn app(st: Arc<AppState>, max_body: usize) -> Router {
    Router::new()
        .route("/v1/autonomy/decide",post(autonomy::decide))
        .route("/v1/autonomy/event",post(autonomy::event))
        .route("/v1/autonomy/quiesce",post(autonomy::quiesce))
        .route("/v1/scheduler/lifecycle",post(scheduler::lifecycle))
        .route("/v1/scheduler/core",post(scheduler::core))
        .route("/v1/episodic/get",post(scheduler::get))
        .route("/v1/episodic/list",post(scheduler::list))
        .route("/v1/knowledge/list",post(knowledge_list))
        .route("/v1/knowledge/remove",post(knowledge_remove))
        .route("/v1/jobs/enqueue",post(enqueue_job))
        .route("/v1/jobs/state",post(jobs_state))
        .route("/v1/jobs/run",post(jobs_run))
        .route("/v1/health", get(|| async { Json(json!({"ok": true})) }))
        .route("/v1/semantic/search", post(search))
        .route("/v1/semantic/get", post(get_item))
        .route("/v1/semantic/list", post(list_items))
        .route("/v1/semantic/delete", post(delete_item))
        .route("/v1/ontology/audit", post(ontology_audit))
        .route("/v1/semantic/insert", post(insert))
        .route("/v1/semantic/reindex", post(reindex))
        .layer(axum::middleware::from_fn(autonomy::memory_stop_guard))
        .layer(DefaultBodyLimit::max(max_body))
        .with_state(st)
}
