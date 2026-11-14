<!-- SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com> -->
<!-- SPDX-License-Identifier: CC-BY-4.0 -->

# Autonomy service (memory writes)

**Powered by XIAR V0 — https://xiarv0.com**

Memory mutations (`/v1/semantic/insert`, `delete`, `reindex`, `/v1/jobs/*`, `/v1/knowledge/remove`) are **fail-closed**. A production `xiar-memory-api` (built without `test-hooks`) admits a write only when:

1. The autonomy policy file is a regular, non-symlink file that is not group/other-writable, and
2. `autonomy-state.json` reports `"stopped": false` under the same ownership rules, and
3. The Unix-socket broker listed in the policy answers `POST /internal/emergency` with `{"stopped": false}`.

Reads work without the broker. A failed or missing probe is treated as stopped.

## Production

Install a reviewed copy of `config/autonomy.toml` at `/etc/xiar/autonomy.toml` (root-owned) and run the privileged broker (`integrations/agent-control/server.py`) as root. That broker also needs a root-owned `agent-control.toml`, its auth helper, and (for desktop drive) optional helpers under `/opt/xiar`. Desktop drive, Hermes paths and `full_agents` are all optional or empty by default; the emergency stop still applies to every agent.

Defaults if unset:

| Item | Production default |
|---|---|
| Policy | `/etc/xiar/autonomy.toml` |
| State | `/etc/xiar/autonomy-state.json` (or `control.state_file`) |
| Broker socket | `control.broker_socket` (typically `/run/xiar-agent-control/broker.sock`) |
| Timezone | `UTC`, or `$TZ` / the system zone if you set `timezone = ""` or `"system"` |
| Desktop UID | Detected from `$XDG_RUNTIME_DIR` or the current user when `desktop_uid = 0` |
| `full_agents` | `[]` |

Override paths with `XIAR_ROOT`, `XIAR_AUTONOMY_CONFIG`, `XIAR_AUTONOMY_STATE`, `XIAR_OWNER_USER`, `XIAR_HERMES_NATIVE` (optional), `XIAR_AGENT_CONTROL_DIR`, and `XIAR_DESKTOP_UID`.

## Local development (memory writes without root)

Do **not** point this mode at `/etc/xiar/autonomy.toml`. Both flags are required; `XIAR_AUTONOMY_DEV` is ignored if the policy path is still the production file.

```bash
export XIAR_ROOT=/path/to/this/repo
export XIAR_AUTONOMY_DEV=1
export XIAR_AUTONOMY_CONFIG=$XIAR_ROOT/config/autonomy.dev.toml

# 1. Init and serve the memory API (production binary, no test-hooks)
cd "$XIAR_ROOT"
# Use a private config whose token/key paths stay under $XIAR_ROOT, not ~/.config,
# if you are just trying the example. See the README for a concrete file.
cargo build -p xiar-memory-api --manifest-path core/rust/Cargo.toml
XIAR_ROOT=$XIAR_ROOT core/rust/target/debug/xiar-memory-api --config config/memory_api.toml init
XIAR_ROOT=$XIAR_ROOT XIAR_AUTONOMY_DEV=1 XIAR_AUTONOMY_CONFIG=$XIAR_AUTONOMY_CONFIG \
  core/rust/target/debug/xiar-memory-api --config config/memory_api.toml serve

# 2. In another shell: the dev broker (running = writes allowed)
XIAR_ROOT=$XIAR_ROOT XIAR_AUTONOMY_DEV=1 XIAR_AUTONOMY_CONFIG=$XIAR_AUTONOMY_CONFIG \
  python integrations/agent-control/dev_broker.py

# 3. Example agent: write, then search
export XIAR_BUSINESS_ID=example-business
export XIAR_MEMORY_TOKEN_FILE=~/.config/xiar/memory-api-tokens.json   # or the path your config uses
python examples/minimal_agent.py --write "Sensor Supply Co" "Delivers evaluation boards for Project Example"
python examples/minimal_agent.py "Who supplies sensors?"
```

`dev_broker.py --stop` turns the emergency stop on (reads still work). `--resume` or a normal start allows writes again.

The dev broker is not a substitute for the privileged production broker: it does not enforce tool policy or drive a desktop. The `xiar-drive-helper` binary is optional and is not required for memory writes, so it is not shipped in this tree.
