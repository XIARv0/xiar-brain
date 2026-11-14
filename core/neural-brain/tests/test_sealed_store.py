# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
import sqlite3
import pytest
from xiar_brain.sealed_store import SealedAccessError, SealedReader, SealedWriter, is_sealed_path

L = {"kind": "mistake", "action": "terminal: delete the build folder", "outcome": "fail", "why": "UNIQUE-LESSON-TEXT-123"}


def test_writer_is_insert_only_and_cannot_read(keys):
    w = SealedWriter(keys.path("paths.sealed_db"), keys.path("paths.sealed_public_key"))
    w.append(L)
    assert w.count() == 1
    with pytest.raises(SealedAccessError):
        w.read()


def test_only_trainer_role_with_private_key_can_read(keys):
    w = SealedWriter(keys.path("paths.sealed_db"), keys.path("paths.sealed_public_key")); w.append(L)
    for role in ("hermes", "mirix", "search", "service", "agent"):
        with pytest.raises(SealedAccessError):
            SealedReader(keys.path("paths.sealed_db"), keys.path("paths.sealed_private_key"), role=role)
    with pytest.raises(SealedAccessError):
        SealedReader(keys.path("paths.sealed_db"), keys.path("paths.sealed_dir") / "missing.key", role="trainer")
    r = SealedReader(keys.path("paths.sealed_db"), keys.path("paths.sealed_private_key"), role="trainer")
    got = list(r.iter_lessons())
    assert got[0]["why"] == L["why"]


def test_db_holds_only_ciphertext(keys):
    w = SealedWriter(keys.path("paths.sealed_db"), keys.path("paths.sealed_public_key")); w.append(L); w.close()
    raw = b"".join(p.read_bytes() for p in keys.path("paths.sealed_dir").glob("lessons.sqlite*"))
    assert b"UNIQUE-LESSON-TEXT-123" not in raw and b"delete the build" not in raw
    # direct SQL access (any role bypassing the API) still sees only ciphertext
    con = sqlite3.connect(keys.path("paths.sealed_db"))
    for row in con.execute("SELECT * FROM lessons"):
        assert all(b"UNIQUE" not in (c if isinstance(c, bytes) else str(c).encode()) for c in row)


def test_db_rejects_update_and_delete(keys):
    w = SealedWriter(keys.path("paths.sealed_db"), keys.path("paths.sealed_public_key")); w.append(L); w.close()
    con = sqlite3.connect(keys.path("paths.sealed_db"))
    with pytest.raises(sqlite3.DatabaseError):
        con.execute("UPDATE lessons SET ts_ms=0")
    with pytest.raises(sqlite3.DatabaseError):
        con.execute("DELETE FROM lessons")


def test_indexers_can_detect_sealed_paths(keys, tmp_path):
    w = SealedWriter(keys.path("paths.sealed_db"), keys.path("paths.sealed_public_key")); w.close()
    assert is_sealed_path(keys.path("paths.sealed_db"))
    assert is_sealed_path(keys.path("paths.sealed_dir"))
    # a copied DB is still recognised by its SQLite application_id
    import shutil
    other = tmp_path / "elsewhere"; other.mkdir()
    shutil.copy(keys.path("paths.sealed_db"), other / "copy.sqlite")
    assert is_sealed_path(other / "copy.sqlite")
    assert not is_sealed_path(tmp_path / "tok")
