# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""SDK credential-command pipe only: never invoke from a terminal or log stdout."""
from pathlib import Path
import argparse,os,stat,sys
p=argparse.ArgumentParser();p.add_argument('--file',required=True);a=p.parse_args();path=Path(a.file)
try:
 s=path.stat()
 if path.is_symlink() or not stat.S_ISREG(s.st_mode) or s.st_uid!=os.getuid() or s.st_mode&0o077:raise ValueError()
 token=path.read_text().strip()
 if len(token)<32 or any(c.isspace() for c in token):raise ValueError()
except (OSError,ValueError):raise SystemExit('Private routing credential unavailable')
sys.stdout.write(token)
