// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
use std::{env, fs, path::Path, sync::Arc};
use xiar_core::secrets::{OsSecretStore, SecretStore};
use xiar_ontology::models::Status;
use xiar_ontology::registry::Registry;
use xiar_owl::functional::parse;
use xiar_owl::model::Axiom;
use xiar_store::keyring::{Keyring, MasterKey};
use xiar_store::relational::{RedbStore, RelationalStore};

fn ontology_parts(axioms: &[Axiom]) -> (usize, usize, usize, usize) {
    let mut classes = 0usize;
    let mut props = 0usize;
    let mut inds = 0usize;
    for ax in axioms {
        match ax {
            Axiom::DeclareClass(_) => classes += 1,
            Axiom::DeclareObjectProperty(_) | Axiom::DeclareDataProperty(_) => props += 1,
            Axiom::DeclareIndividual(_) => inds += 1,
            _ => {}
        }
    }
    (classes, props, inds, axioms.len())
}

fn try_decrypt(ring: &Keyring, store: &Arc<dyn RelationalStore>) {
    let rows = store.scan("core_blocks", b"", 50).unwrap_or_default();
    println!("core_blocks_total={}", rows.len());
    let mut opened = 0u32;
    let mut failed = 0u32;
    for (k, v) in &rows {
        let parts: Vec<&[u8]> = k.split(|&b| b == 0).collect();
        if parts.len() != 3 {
            println!("bad_key parts={}", parts.len());
            continue;
        }
        let user = String::from_utf8_lossy(parts[0]).to_string();
        let scope = String::from_utf8_lossy(parts[1]).to_string();
        let label = String::from_utf8_lossy(parts[2]).to_string();
        let aad = [b"xiar-core-block:".as_slice(), parts[0], b"\0", parts[1], b"\0", parts[2]].concat();
        match ring.open(&user, &aad, v) {
            Ok(pt) => {
                opened += 1;
                let preview: String = String::from_utf8_lossy(&pt).chars().take(120).collect();
                println!("decrypt_ok user={user} scope={scope} label={label} bytes={} preview={preview}", pt.len());
            }
            Err(e) => {
                failed += 1;
                println!("decrypt_fail user={user} scope={scope} label={label} err={e}");
                // also try with user=system in case of mis-association
                match ring.open("system", &aad, v) {
                    Ok(pt) => println!("  retry_system_ok bytes={}", pt.len()),
                    Err(e2) => println!("  retry_system_fail err={e2}"),
                }
            }
        }
        println!("  has_key({user})={:?} is_open={}", ring.has_key(&user), ring.is_open(&user));
    }
    println!("core_decrypt_summary opened={opened} failed={failed}");
}

fn main() {
    let mut args = env::args().skip(1);
    let db_path = args.next().expect("live db");
    let onto_dir = args.next().expect("ofn dir");
    let registry_db = args.next().expect("registry db");

    println!("=== DEK decrypt ===");
    let store: Arc<dyn RelationalStore> = Arc::new(RedbStore::open(Path::new(&db_path)).unwrap());
    for table in ["sys_user_keys", "sys_user_locks", "sys_user_recovery", "bots", "core_blocks"] {
        println!("store_scan {table} rows={}", store.scan(table, b"", 1000).map(|r| r.len()).unwrap_or(0));
    }
    let users: Vec<String> = store.scan("sys_user_keys", b"", 100).unwrap().into_iter().map(|(k,_)| String::from_utf8_lossy(&k).into()).collect();
    println!("dek_users={users:?}");
    let master = MasterKey::from_bytes(OsSecretStore::new("XIAR V0 Agent OS").master_key().expect("master"));
    println!("os_master_key=loaded");
    let ring = Keyring::new(store.clone(), master);
    for u in &users {
        println!("has_key {u}={:?} is_open={}", ring.has_key(u), ring.is_open(u));
    }
    try_decrypt(&ring, &store);

    println!("=== load OFN via Registry ===");
    if Path::new(&registry_db).exists() { let _ = fs::remove_file(&registry_db); }
    let registry = Registry::new(Arc::new(RedbStore::open(Path::new(&registry_db)).unwrap()));
    let mut paths: Vec<_> = fs::read_dir(&onto_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ofn")).collect();
    paths.sort();
    for p in paths {
        let text = fs::read_to_string(&p).unwrap();
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        let status = if stem.contains("_promoted") { Status::Promoted }
            else if stem.contains("_deprecated") { Status::Deprecated }
            else if stem.contains("_validated") { Status::Validated }
            else { Status::Draft };
        match registry.register_text(&text, &stem, false, Some(stem.clone()), status) {
            Ok(info) => {
                let o = parse(&text).unwrap();
                let (c, pr, i, ax) = ontology_parts(&o.axioms);
                println!("loaded name={stem} as={} status={:?} classes={c} properties={pr} individuals={i} axioms={ax}", info.version, info.status);
            }
            Err(e) => println!("load_fail {stem} {e}"),
        }
    }
    for i in registry.list(100).unwrap_or_default() {
        println!("registry_entry version={} status={:?} by={}", i.version, i.status, i.promoted_by);
    }
}
