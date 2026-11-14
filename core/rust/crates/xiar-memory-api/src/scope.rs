// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Triple scope physical isolation layered over the existing verbatim TenantRegistry.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use serde_json::json;
use xiar_memory::manager::Sealer;
use xiar_memory::tenant::{BusinessId, TenantRegistry, TenantSemantic};

#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Scope { pub agent: String, pub user: String, pub business: String }

pub struct ScopedStores {
    locks: Mutex<HashMap<Scope,Arc<RwLock<()>>>>,
    root: PathBuf,
    language: String,
    sealer: Arc<dyn Sealer>,
    registries: Mutex<HashMap<(String, String), Arc<TenantRegistry>>>,
    episodic: Mutex<HashMap<Scope, Arc<xiar_memory::manager::MemoryManager<xiar_memory::types::EpisodicEvent>>>>,
}

impl ScopedStores {
    pub fn new(root: PathBuf, language: String, sealer: Arc<dyn Sealer>) -> Self {
        Self { locks:Mutex::new(HashMap::new()), root, language, sealer, episodic:Mutex::new(HashMap::new()), registries: Mutex::new(HashMap::new()) }
    }
    pub fn episodic(&self,scope:&Scope)->Result<Arc<xiar_memory::manager::MemoryManager<xiar_memory::types::EpisodicEvent>>,String>{
        let dir=self.directory(scope)?.join("episodic");
        if std::fs::symlink_metadata(&dir).is_ok_and(|m|m.file_type().is_symlink()){return Err("episodic symlink refused".into());}
        let mut cache=self.episodic.lock().map_err(|_|"episodic registry lock")?;
        if let Some(m)=cache.get(scope){return Ok(m.clone());}
        let store=Arc::new(xiar_store::vector::EdgeVectorStore::open(&dir,&self.language).map_err(|_|"episodic store unavailable")?);
        let manager=Arc::new(xiar_memory::manager::MemoryManager::new(store,self.sealer.clone(),&[]).map_err(|_|"episodic manager unavailable")?);
        cache.insert(scope.clone(),manager.clone());Ok(manager)
    }
    pub fn bound_scopes(&self)->Vec<Scope>{
        let mut found=vec![];
        let dirs=|p:PathBuf|std::fs::read_dir(p).into_iter().flatten().flatten().filter(|e|e.file_type().is_ok_and(|t|t.is_dir())).map(|e|e.path()).collect::<Vec<_>>();
        for agent in dirs(self.root.join("agents")){for user in dirs(agent.join("users")){for tenant in dirs(user.join("tenants")){
            if let Ok(raw)=std::fs::read(tenant.join(".xiar-scope")){if let Ok(v)=serde_json::from_slice::<serde_json::Value>(&raw){
                let s=Scope{agent:v["agent_id"].as_str().unwrap_or("").into(),user:v["end_user_id"].as_str().unwrap_or("").into(),business:v["business_id"].as_str().unwrap_or("").into()};
                if self.is_bound(&s)&&self.root.join("agents").join(&s.agent).join("users").join(&s.user).join("tenants").join(&s.business)==tenant&&self.open(&s).is_ok(){found.push(s);}
            }}
        }}}found
    }
    pub fn is_bound(&self,scope:&Scope)->bool{
        for value in [&scope.agent,&scope.user,&scope.business]{
            if !matches!(BusinessId::parse(value),Ok(ref id) if id.slug()==value){return false;}
        }
        let dir=self.root.join("agents").join(&scope.agent).join("users").join(&scope.user).join("tenants").join(&scope.business);
        dir.join(".xiar-scope").is_file()&&dir.join(".xiar-tenant").is_file()
    }
    pub fn operation_lock(&self,scope:&Scope)->Result<Arc<RwLock<()>>,String>{
        self.open(scope)?;
        let mut locks=self.locks.lock().map_err(|_|"scope operation lock poisoned")?;
        Ok(locks.entry(scope.clone()).or_insert_with(||Arc::new(RwLock::new(()))).clone())
    }
    pub fn directory(&self,scope:&Scope)->Result<PathBuf,String>{
        self.open(scope)?;
        Ok(self.root.join("agents").join(&scope.agent).join("users").join(&scope.user).join("tenants").join(&scope.business))
    }
    pub fn ontology_path(&self, scope:&Scope)->Result<PathBuf,String>{
        self.open(scope)?;
        Ok(self.root.join("agents").join(&scope.agent).join("users").join(&scope.user).join("tenants").join(&scope.business).join("ontology-promoted.ofn"))
    }
    pub fn open(&self, scope: &Scope) -> Result<Arc<TenantSemantic>, String> {
        // Validate every component again at the filesystem boundary, even for cached handles.
        for value in [&scope.agent, &scope.user, &scope.business] {
            if BusinessId::parse(value).map_err(|e|e.to_string())?.slug() != value {
                return Err("scope must be a normalized slug".into());
            }
        }
        let root = self.root.join("agents").join(&scope.agent).join("users").join(&scope.user);
        let mut regs = self.registries.lock().map_err(|_| "scope registry lock poisoned")?;
        let reg = regs.entry((scope.agent.clone(), scope.user.clone())).or_insert_with(||
            Arc::new(TenantRegistry::new(&root, &self.language, &scope.user, self.sealer.clone(), &[]))).clone();
        let business = BusinessId::parse(&scope.business).map_err(|e|e.to_string())?;
        let dir = reg.tenant_dir(&business);
        let marker = dir.join(".xiar-scope");
        let expected = json!({"agent_id":scope.agent,"business_id":scope.business,"end_user_id":scope.user});
        if marker.exists() {
            let got: serde_json::Value = serde_json::from_slice(&std::fs::read(&marker).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            if got != expected { return Err("scope marker mismatch".into()); }
        } else {
            if dir.exists() && std::fs::read_dir(&dir).map_err(|e|e.to_string())?.next().is_some() {
                return Err("unbound scope directory refused".into());
            }
            reg.open(Some(&scope.business)).map_err(|e|e.to_string())?;
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&marker).map_err(|e|e.to_string())?;
            f.write_all(expected.to_string().as_bytes()).map_err(|e|e.to_string())?;
        }
        if std::fs::read_to_string(dir.join(".xiar-tenant"))
            .map_err(|e|e.to_string())?.trim() != business.scope() {
            return Err("tenant marker mismatch".into());
        }
        reg.open(Some(&scope.business)).map_err(|e|e.to_string())
    }
}
