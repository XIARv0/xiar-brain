<!-- SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com> -->
<!-- SPDX-License-Identifier: CC-BY-4.0 -->

# Secret file formats (placeholders only)

Real secret files are created by the services and must never be committed (they are in `.gitignore`).

| Example | Real file (default path) | Created by |
|---|---|---|
| `memory-api-tokens.json.example` | `~/.config/xiar/memory-api-tokens.json` (`[auth].tokens_file`) | `xiar-memory-api init` |
| `memory-api-master.key.example` | `~/.config/xiar/memory-api-master.key` (`[store].master_key_file`) | `xiar-memory-api init` |
| `memory-grants.json.example` | `/etc/xiar/memory-grants.json` (`[auth].dynamic_grants_file`) | the XIAR root broker (private agents) |
| `brain-api-tokens.json.example` | `data/neural-brain/secrets/api-tokens.json` (`[paths].tokens_file`) | brain service, first start |
| — | `data/neural-brain/trainer/trainer-secret.key`, `sealed/writer.pub` | brain trainer (sealed lesson store keys) |
| — | `~/.config/xiar/memory-api-token`, `~/.config/xiar/brain-tokens.json`, `~/.config/xiar/laya-token` | copied for the Hermes adapter (`config/hermes_adapter.toml`) |
