// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
const T: TableDefinition<&str, &[u8]> = TableDefinition::new("core_blocks");
fn main() {
  let db = Database::open(std::env::args().nth(1).unwrap()).unwrap();
  let txn = db.begin_read().unwrap();
  let t = txn.open_table(T).unwrap();
  let mut n=0u64; let mut bytes=0u64;
  for i in t.iter().unwrap() { let (_,v)=i.unwrap(); n+=1; bytes+=v.value().len() as u64; }
  println!("core_blocks_rows={n} bytes={bytes}");
}
