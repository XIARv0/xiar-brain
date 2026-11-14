// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! `xiar-memory-api [--config PATH] init|serve`
use std::sync::Arc;
use xiar_memory_api::{app, init, load_config, resolve, xiar_root, AppState};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = xiar_root();
    let cfg_path = args.iter().position(|a| a == "--config").and_then(|i| args.get(i + 1)).map(|p| resolve(&root, p))
        .unwrap_or_else(|| std::env::var_os("XIAR_MEMORY_API_CONFIG").map(Into::into).unwrap_or_else(|| root.join("config/memory_api.toml")));
    let cfg = load_config(&cfg_path).unwrap_or_else(|e| { eprintln!("config: {e}"); std::process::exit(2) });
    let mut cmd = String::from("serve");
    let mut skip = false;
    for a in args.iter().skip(1) {
        if skip { skip = false; continue; }
        if a == "--config" { skip = true; continue; }
        cmd = a.clone();
        break;
    }
    match cmd.as_str() {
        "init" => match init(&cfg, &root) {
            Ok(made) => { for m in made { println!("created {m}"); } println!("init ok (secrets are in files, never printed)"); }
            Err(e) => { eprintln!("init: {e}"); std::process::exit(1) }
        },
        "serve" => {
            let st = Arc::new(AppState::from_config(&cfg, &root).unwrap_or_else(|e| { eprintln!("startup: {e}"); std::process::exit(1) }));
            xiar_memory_api::start_population(&st);
            let l = tokio::net::TcpListener::bind(&cfg.server.bind).await.unwrap_or_else(|e| { eprintln!("bind {}: {e}", cfg.server.bind); std::process::exit(1) });
            eprintln!("xiar-memory-api on {} (ontology: {})", cfg.server.bind, st.expander.is_some());
            axum::serve(l, app(st, cfg.server.max_body_bytes)).with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; }).await.ok();
        }
        other => { eprintln!("unknown command {other:?} (init|serve)"); std::process::exit(2) }
    }
}
