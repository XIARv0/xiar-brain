#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Copy legacy owner Semantic stores into Xiar's bound scope, leaving originals untouched.

Run once with memory-api stopped. No lesson store is read. No credential contents
are printed. New-agent/user provisioning must NOT use this owner-only migration.
"""
import json
import os
import pathlib
import shutil

root=pathlib.Path(os.environ.get("XIAR_ROOT", pathlib.Path.home()/"xiar-os"))
store=root/"data/memory-api"
legacy=store/"tenants"
count=0
for source in sorted(legacy.iterdir()) if legacy.exists() else []:
    if not source.is_dir(): continue
    business=source.name
    if not (source/".xiar-tenant").is_file() or (source/".xiar-tenant").read_text().strip() != "business:"+business:
        raise RuntimeError("legacy business marker mismatch")
    dest=store/"agents/xiar/users/owner/tenants"/business
    expected=dict(agent_id="xiar",end_user_id="owner",business_id=business)
    if dest.exists():
        marker=dest/".xiar-scope"
        if marker.exists() and json.loads(marker.read_text()) == expected:
            continue
        raise RuntimeError("existing destination is not bound to expected scope")
    shutil.copytree(source,dest)
    (dest/".xiar-scope").write_text(json.dumps(expected),encoding="utf-8")
    count+=1
print(json.dumps(dict(copied_business_stores=count,legacy_left_untouched=True)))