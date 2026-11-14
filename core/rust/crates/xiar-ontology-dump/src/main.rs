// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! List tables / ontology_versions; optional OFN export.
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, TableHandle};
use std::{env, fs, path::PathBuf};

const ONTO: TableDefinition<&[u8], &[u8]> = TableDefinition::new("ontology_versions");

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().expect("usage: xiar-ontology-dump <redb> [export_dir] [label]");
    let export_dir = args.next();
    let label = args.next().unwrap_or_else(|| "onto".into());

    let db = Database::open(&path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    let txn = db.begin_read().unwrap();
    let mut names: Vec<String> = txn.list_tables().unwrap().map(|d| d.name().to_string()).collect();
    names.sort();
    println!("PATH={path}");
    println!("tables={}", names.len());
    let has = names.iter().any(|n| n == "ontology_versions");
    println!("ontology_versions={has}");
    if !has { return; }
    let t = txn.open_table(ONTO).unwrap();
    for row in t.iter().unwrap() {
        let (k, v) = row.unwrap();
        let key = String::from_utf8_lossy(k.value()).to_string();
        let val: serde_json::Value = serde_json::from_slice(v.value()).unwrap();
        let ver = val["info"]["version"].as_str().unwrap_or(&key).to_string();
        let status = val["info"]["status"].as_str().unwrap_or("?").to_string();
        let text = val["text"].as_str().unwrap_or("").to_string();
        let classes = text.matches("Declaration(Class(").count();
        let properties = text.matches("Declaration(ObjectProperty(").count()
            + text.matches("Declaration(DataProperty(").count()
            + text.matches("Declaration(AnnotationProperty(").count();
        let individuals = text.matches("Declaration(NamedIndividual(").count();
        println!("onto ver={ver} status={status} classes={classes} properties={properties} individuals={individuals} text_bytes={}", text.len());
        if let Some(ref dir) = export_dir {
            let out = PathBuf::from(dir);
            fs::create_dir_all(&out).unwrap();
            let base = format!("{label}_{ver}_{status}");
            let meta = serde_json::json!({
                "source_db": path,
                "label": label,
                "version": ver,
                "status": status,
                "classes": classes,
                "properties": properties,
                "individuals": individuals,
                "text_bytes": text.len(),
                "file_hash": val["info"]["file_hash"],
                "promoted_by": val["info"]["promoted_by"],
                "diff_summary": val["info"]["diff_summary"],
            });
            fs::write(out.join(format!("{base}.ofn")), &text).unwrap();
            fs::write(out.join(format!("{base}.meta.json")), serde_json::to_vec_pretty(&meta).unwrap()).unwrap();
            println!("exported {base}");
        }
    }
}
