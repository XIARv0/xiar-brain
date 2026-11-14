// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! `RelationalStore`: named tables of byte keys -> byte values with ordered scans and atomic transactions.
//! Typed repositories (users, sessions, messages, UI variables, ...) are built on top of this trait, so the
//! engine (redb here; bundled SQLite as an optional plug-in later) can be swapped.

use crate::{Result, StoreError};
use redb::{
    backends::InMemoryBackend, Database, ReadTransaction, ReadableDatabase, ReadableTable, TableDefinition,
    TableError, WriteTransaction,
};
use std::path::Path;

pub type Row = (Vec<u8>, Vec<u8>);

/// Operations available inside a transaction. Reads see writes made earlier in the same write transaction.
pub trait Txn {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>>;
    /// Last row (greatest key) of the table.
    fn last(&self, table: &str) -> Result<Option<Row>>;
    /// Up to `limit` rows whose key starts with `prefix`, in key order.
    fn scan(&self, table: &str, prefix: &[u8], limit: usize) -> Result<Vec<Row>>;
    fn put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<()>;
    fn delete(&mut self, table: &str, key: &[u8]) -> Result<bool>;
}

pub trait RelationalStore: Send + Sync {
    /// Read-only snapshot.
    fn read(&self, f: &mut dyn FnMut(&dyn Txn) -> Result<()>) -> Result<()>;
    /// Atomic read-modify-write. Commits if `f` returns Ok, rolls back on Err. Writers are serialized.
    fn write(&self, f: &mut dyn FnMut(&mut dyn Txn) -> Result<()>) -> Result<()>;

    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut out = None;
        self.read(&mut |t| {
            out = t.get(table, key)?;
            Ok(())
        })?;
        Ok(out)
    }
    fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<()> {
        self.write(&mut |t| t.put(table, key, value))
    }
    fn delete(&self, table: &str, key: &[u8]) -> Result<bool> {
        let mut removed = false;
        self.write(&mut |t| {
            removed = t.delete(table, key)?;
            Ok(())
        })?;
        Ok(removed)
    }
    fn scan(&self, table: &str, prefix: &[u8], limit: usize) -> Result<Vec<Row>> {
        let mut out = Vec::new();
        self.read(&mut |t| {
            out = t.scan(table, prefix, limit)?;
            Ok(())
        })?;
        Ok(out)
    }
}

pub struct RedbStore {
    db: Database,
}

impl RedbStore {
    pub fn open(path: &Path) -> Result<Self> {
        let db = if path.exists() { Database::open(path) } else { Database::create(path) }.map_err(StoreError::backend)?;
        Ok(Self { db })
    }

    /// Volatile store for tests.
    pub fn in_memory() -> Result<Self> {
        let db = Database::builder().create_with_backend(InMemoryBackend::new()).map_err(StoreError::backend)?;
        Ok(Self { db })
    }
}

type Def<'a> = TableDefinition<'a, &'static [u8], &'static [u8]>;

fn def(name: &str) -> Def<'_> {
    TableDefinition::new(name)
}

fn range_scan<T: ReadableTable<&'static [u8], &'static [u8]>>(t: &T, prefix: &[u8], limit: usize) -> Result<Vec<Row>> {
    let mut out = Vec::new();
    for item in t.range::<&[u8]>(prefix..).map_err(StoreError::backend)? {
        let (k, v) = item.map_err(StoreError::backend)?;
        if !k.value().starts_with(prefix) || out.len() >= limit {
            break;
        }
        out.push((k.value().to_vec(), v.value().to_vec()));
    }
    Ok(out)
}

struct RedbRead(ReadTransaction);
struct RedbWrite<'a>(&'a WriteTransaction);

impl RedbRead {
    fn open_tbl(&self, table: &str) -> std::result::Result<redb::ReadOnlyTable<&'static [u8], &'static [u8]>, TableError> {
        self.0.open_table(def(table))
    }
}

impl<'a> RedbWrite<'a> {
    fn open_tbl(&self, table: &str) -> std::result::Result<redb::Table<'a, &'static [u8], &'static [u8]>, TableError> {
        self.0.open_table(def(table))
    }
}

macro_rules! read_ops {
    () => {
        fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>> {
            match self.open_tbl(table) {
                Ok(t) => Ok(t.get(key).map_err(StoreError::backend)?.map(|v| v.value().to_vec())),
                Err(TableError::TableDoesNotExist(_)) => Ok(None),
                Err(e) => Err(StoreError::backend(e)),
            }
        }
        fn last(&self, table: &str) -> Result<Option<Row>> {
            match self.open_tbl(table) {
                Ok(t) => Ok(t.last().map_err(StoreError::backend)?.map(|(k, v)| (k.value().to_vec(), v.value().to_vec()))),
                Err(TableError::TableDoesNotExist(_)) => Ok(None),
                Err(e) => Err(StoreError::backend(e)),
            }
        }
        fn scan(&self, table: &str, prefix: &[u8], limit: usize) -> Result<Vec<Row>> {
            match self.open_tbl(table) {
                Ok(t) => range_scan(&t, prefix, limit),
                Err(TableError::TableDoesNotExist(_)) => Ok(Vec::new()),
                Err(e) => Err(StoreError::backend(e)),
            }
        }
    };
}

impl Txn for RedbRead {
    read_ops!();
    fn put(&mut self, _: &str, _: &[u8], _: &[u8]) -> Result<()> {
        Err(StoreError::Backend("write on read-only transaction".into()))
    }
    fn delete(&mut self, _: &str, _: &[u8]) -> Result<bool> {
        Err(StoreError::Backend("write on read-only transaction".into()))
    }
}

impl Txn for RedbWrite<'_> {
    read_ops!();
    fn put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<()> {
        let mut t = self.0.open_table(def(table)).map_err(StoreError::backend)?;
        t.insert(key, value).map_err(StoreError::backend)?;
        Ok(())
    }
    fn delete(&mut self, table: &str, key: &[u8]) -> Result<bool> {
        let mut t = self.0.open_table(def(table)).map_err(StoreError::backend)?;
        let removed = t.remove(key).map_err(StoreError::backend)?.is_some();
        Ok(removed)
    }
}

impl RelationalStore for RedbStore {
    fn read(&self, f: &mut dyn FnMut(&dyn Txn) -> Result<()>) -> Result<()> {
        let txn = RedbRead(self.db.begin_read().map_err(StoreError::backend)?);
        f(&txn)
    }

    fn write(&self, f: &mut dyn FnMut(&mut dyn Txn) -> Result<()>) -> Result<()> {
        let w = self.db.begin_write().map_err(StoreError::backend)?;
        let res = {
            let mut txn = RedbWrite(&w);
            f(&mut txn)
        };
        match res {
            Ok(()) => w.commit().map_err(StoreError::backend),
            Err(e) => {
                let _ = w.abort();
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_delete_scan_last() {
        let s = RedbStore::in_memory().unwrap();
        assert_eq!(s.get("t", b"a").unwrap(), None, "missing table reads as empty");
        s.put("t", b"user/1/a", b"1").unwrap();
        s.put("t", b"user/1/b", b"2").unwrap();
        s.put("t", b"user/2/a", b"3").unwrap();
        assert_eq!(s.get("t", b"user/1/b").unwrap(), Some(b"2".to_vec()));
        let rows = s.scan("t", b"user/1/", 10).unwrap();
        assert_eq!(rows.iter().map(|r| r.1.clone()).collect::<Vec<_>>(), vec![b"1".to_vec(), b"2".to_vec()]);
        assert_eq!(s.scan("t", b"user/1/", 1).unwrap().len(), 1, "limit respected");
        assert_eq!(s.scan("nope", b"", 5).unwrap(), vec![]);
        s.read(&mut |t| {
            assert_eq!(t.last("t")?.unwrap().0, b"user/2/a".to_vec());
            Ok(())
        })
        .unwrap();
        assert!(s.delete("t", b"user/1/a").unwrap());
        assert!(!s.delete("t", b"user/1/a").unwrap());
    }

    #[test]
    fn failed_write_rolls_back_atomically() {
        let s = RedbStore::in_memory().unwrap();
        s.put("t", b"k", b"old").unwrap();
        let r = s.write(&mut |t| {
            t.put("t", b"k", b"new")?;
            t.put("t", b"other", b"x")?;
            Err(StoreError::Backend("boom".into()))
        });
        assert!(r.is_err());
        assert_eq!(s.get("t", b"k").unwrap(), Some(b"old".to_vec()));
        assert_eq!(s.get("t", b"other").unwrap(), None);
    }

    #[test]
    fn writes_see_own_earlier_writes_and_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.redb");
        {
            let s = RedbStore::open(&path).unwrap();
            s.write(&mut |t| {
                t.put("seq", b"\x00", b"first")?;
                let last = t.last("seq")?.unwrap();
                assert_eq!(last.1, b"first".to_vec());
                t.put("seq", b"\x01", b"second")
            })
            .unwrap();
        }
        let s = RedbStore::open(&path).unwrap();
        assert_eq!(s.scan("seq", b"", 10).unwrap().len(), 2);
    }
}
