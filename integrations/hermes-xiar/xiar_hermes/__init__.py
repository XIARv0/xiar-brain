# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""XIAR <-> Hermes adapter (Hermes plugin `xiar`). Integration points: HermesIntegration.md."""
from .plugin import register  # noqa: F401  (Hermes entry point / directory plugin both call register(ctx))
