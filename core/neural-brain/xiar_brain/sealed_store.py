# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Sealed lesson store: append-only, readable ONLY by the training job (owner t41u/t42u).

Enforcement layers:
1. Crypto (DB content): every row is encrypted to the trainer's X25519 public key (ECIES:
   ephemeral X25519 + HKDF-SHA256 + AES-256-GCM). Writers hold only the public key, so the
   service, Hermes and MIRIX can INSERT but cannot decrypt/SELECT plaintext. Only the trainer loads
   the private key (`paths.sealed_private_key`, never shipped to query or writer clients).
2. DB: SQLite triggers make rows immutable (no UPDATE/DELETE); schema carries a `sealed` marker and
   `PRAGMA application_id`, and the directory carries `.xiar-sealed` so indexers refuse it.
3. API: the service exposes write-only `/v1/lessons`; there is no endpoint that returns lessons.
"""
from __future__ import annotations

import json
import os
import pathlib
import sqlite3
import threading
import time
import uuid

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

APP_ID = 0x58494152  # "XIAR"
MARKER = ".xiar-sealed"
INFO = b"xiar-sealed-lesson-v1"

SCHEMA = """
CREATE TABLE IF NOT EXISTS meta(k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS lessons(
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT UNIQUE NOT NULL,
  ts_ms INTEGER NOT NULL,
  epk BLOB NOT NULL, nonce BLOB NOT NULL, ct BLOB NOT NULL);
CREATE TRIGGER IF NOT EXISTS lessons_no_update BEFORE UPDATE ON lessons
  BEGIN SELECT RAISE(ABORT, 'sealed lesson store is append-only'); END;
CREATE TRIGGER IF NOT EXISTS lessons_no_delete BEFORE DELETE ON lessons
  BEGIN SELECT RAISE(ABORT, 'sealed lesson store is append-only'); END;
CREATE TRIGGER IF NOT EXISTS meta_no_update BEFORE UPDATE ON meta
  BEGIN SELECT RAISE(ABORT, 'sealed meta is immutable'); END;
"""


class SealedAccessError(PermissionError):
    pass


def _kdf(shared: bytes, epk: bytes) -> bytes:
    return HKDF(algorithm=hashes.SHA256(), length=32, salt=epk, info=INFO).derive(shared)


def _raw_pub(pk: X25519PublicKey) -> bytes:
    return pk.public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


def generate_keys(public_path: pathlib.Path, private_path: pathlib.Path, overwrite: bool = False) -> None:
    if private_path.exists() and not overwrite:
        raise FileExistsError(f"{private_path} exists; refusing to replace the trainer key")
    sk = X25519PrivateKey.generate()
    private_path.parent.mkdir(parents=True, exist_ok=True)
    public_path.parent.mkdir(parents=True, exist_ok=True)
    raw = sk.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw, serialization.NoEncryption())
    fd = os.open(private_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "wb") as f:
        f.write(raw)
    public_path.write_bytes(_raw_pub(sk.public_key()))


def _open_db(db_path: pathlib.Path) -> sqlite3.Connection:
    db_path.parent.mkdir(parents=True, exist_ok=True)
    (db_path.parent / MARKER).write_text("sealed: readable only by the XIAR NeuralBrain training job\n", encoding="utf-8")
    con = sqlite3.connect(str(db_path), timeout=30, isolation_level=None, check_same_thread=False)
    con.execute("PRAGMA journal_mode=WAL")
    con.execute(f"PRAGMA application_id={APP_ID}")
    con.executescript(SCHEMA)
    con.execute("INSERT OR IGNORE INTO meta(k,v) VALUES('sealed','1')")
    return con


def is_sealed_path(path: os.PathLike | str) -> bool:
    """True if `path` is inside a sealed store dir or is a sealed DB. Indexers/search MUST skip these."""
    p = pathlib.Path(path).resolve()
    for d in [p, *p.parents]:
        if (d / MARKER).exists():
            return True
    if p.is_file():
        try:
            with open(p, "rb") as f:
                head = f.read(100)
            if head.startswith(b"SQLite format 3") and int.from_bytes(head[68:72], "big") == APP_ID:
                return True
        except OSError:
            pass
    return False


class SealedWriter:
    """INSERT-only handle. Holds the public key; cannot decrypt anything it wrote."""

    def __init__(self, db_path: pathlib.Path, public_key_path: pathlib.Path):
        self._pk = X25519PublicKey.from_public_bytes(public_key_path.read_bytes())
        self._con = _open_db(db_path)
        self._lock = threading.Lock()

    def append(self, lesson: dict) -> str:
        lid = lesson.get("id") or str(uuid.uuid4())
        rec = dict(lesson, id=lid)
        rec.setdefault("ts_ms", int(time.time() * 1000))
        esk = X25519PrivateKey.generate()
        epk = _raw_pub(esk.public_key())
        key = _kdf(esk.exchange(self._pk), epk)
        nonce = os.urandom(12)
        ct = AESGCM(key).encrypt(nonce, json.dumps(rec, ensure_ascii=False).encode(), lid.encode())
        with self._lock:
            self._con.execute("INSERT INTO lessons(id,ts_ms,epk,nonce,ct) VALUES(?,?,?,?,?)",
                              (lid, rec["ts_ms"], epk, nonce, ct))
        return lid

    def count(self) -> int:
        with self._lock:
            return self._con.execute("SELECT COUNT(*) FROM lessons").fetchone()[0]

    def read(self, *_a, **_k):
        raise SealedAccessError("writers cannot read the sealed lesson store")

    def close(self):
        self._con.close()


class SealedReader:
    """Training-job-only handle. Requires the trainer private key."""

    def __init__(self, db_path: pathlib.Path, private_key_path: pathlib.Path, role: str):
        if role != "trainer":
            raise SealedAccessError(f"role {role!r} may not read the sealed lesson store")
        if not private_key_path.exists():
            raise SealedAccessError("trainer private key not present on this machine")
        self._sk = X25519PrivateKey.from_private_bytes(private_key_path.read_bytes())
        self._con = _open_db(db_path)

    def iter_lessons(self, after_seq: int = 0):
        rows = self._con.execute("SELECT seq,id,epk,nonce,ct FROM lessons WHERE seq>? ORDER BY seq", (after_seq,))
        for seq, lid, epk, nonce, ct in rows:
            key = _kdf(self._sk.exchange(X25519PublicKey.from_public_bytes(epk)), epk)
            rec = json.loads(AESGCM(key).decrypt(nonce, ct, lid.encode()))
            rec["_seq"] = seq
            yield rec

    def max_seq(self) -> int:
        return self._con.execute("SELECT COALESCE(MAX(seq),0) FROM lessons").fetchone()[0]

    def close(self):
        self._con.close()
