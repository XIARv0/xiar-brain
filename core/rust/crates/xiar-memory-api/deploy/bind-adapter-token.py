# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Bind the public client name to an adapter token file; never prints the token.
Run only as the account that owns the mode-0600 token files.
"""
import json
import os
import pathlib
import sys
client=sys.argv[1]
base=pathlib.Path.home()/'.config/xiar'
tokens=json.loads((base/'memory-api-tokens.json').read_text())
if client not in tokens: raise SystemExit('configured client token missing')
target=base/'memory-api-token'
fd=os.open(target,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
try: os.write(fd,tokens[client].encode())
finally: os.close(fd)
os.chmod(target,0o600)
print('adapter token bound to configured client (value not printed)')