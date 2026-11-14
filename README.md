<!-- SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com> -->
<!-- SPDX-License-Identifier: CC-BY-4.0 -->

# XIAR V0 — Brain System (private preview)

**Powered by XIAR V0 — https://xiarv0.com**

![licence: AGPL-3.0-or-later + 7(b)](https://img.shields.io/badge/licence-AGPL--3.0--or--later%20%2B%207(b)-blue)

The memory and learning layer of XIAR: MIRIX-style long-term memory in Rust, a tenant-scoped memory HTTP API,
an OWL ontology builder/expander, auto-dream consolidation prompts, a small neural "brain" that learns from
outcomes, the MirixGovernor ledger, 99 mental models and the Swipe Deck.

> **Status: private preview for invited reviewers (Read access).** Not a release. Nothing here is a finished
> product, and the licence's additional term is a draft awaiting counsel review.

## What is in this repo

| Path | What it is |
|---|---|
| `core/rust/crates/xiar-memory` | Memory core: Core, Episodic, Semantic, Procedural, Resource, KnowledgeVault, Raw and Experience memory on redb + `xiar-qdrant-edge` (ported from MIRIX). `governance.rs` = **MirixGovernor ledger** (promote / forget / cross-business proposal / vault erase / namespace write). `core.rs` = core-memory rewrite/append tools used by auto-dream. |
| `core/rust/crates/xiar-memory-api` | HTTP API on **127.0.0.1:8771** (axum). Tenants per agent × end user × business. |
| `core/rust/crates/xiar-ontology`, `xiar-owl` | **Ontology builder/expander** (scout → engineer → critic, OWL 2 reasoner, OFN/RDF-XML) and query expansion. |
| `core/rust/crates/xiar-ontology-{recall,verify,dump}` | Recall evaluation (ontology OFF 0.60 → ON 1.00 recall@5 on the example), verification and dump tools. |
| `core/rust/ontologies-example/` | A **small synthetic example ontology** (11 classes, fictional "Example Co" org) and its seed texts. No live ontology is exported. |
| `core/rust/crates/xiar-{core,llm,prompts,store,audit,engine-http,screen,voice}` | Support crates the memory API needs to build. |
| `core/rust/vendor/xiar-qdrant-edge` | Modified qdrant-edge 0.8.0 (Apache-2.0). |
| `core/rust/prompts/` | `ontology/` (XIAR), `base/core_memory_block.md` (XIAR), `mirix/` (original MIRIX prompts incl. **auto-dream**, Apache-2.0, kept separate). |
| `core/neural-brain` | Neural brain (Python/PyTorch, **127.0.0.1:8770**) with the **sealed lesson store** (write-only, encrypted to the trainer's key). No weights, no keys. |
| `integrations/hermes-xiar` | Reference agent adapter (Hermes plugin) that wires the hooks below; includes the **Swipe Deck** (`swipe_deck.py`, `swipe_judge.py`) and mental-model selection. |
| `integrations/settings-panel/swipe_deck.py`, `GUI/src/settings-web/SwipeDeck*` | Swipe Deck panel backend and React view (reference copy; the view needs the full XIAR GUI to build). |
| `integrations/loops` | Reflex / Circadian / **Dream** loops (`xiar_loops.py dream`, systemd timer). |
| `integrations/agent-control` | Autonomy service: privileged production broker (`server.py`) and a **dev broker** (`dev_broker.py`) so local memory writes work without root. |
| `integrations/model-connections` | Optional vetted provider adapter (`call.py`, `manage.py`, `proxy.py`). Not required for memory writes. |
| `Brain/Shared/mental-models` | The 99 mental models in 8 families + `Index.md`. |
| `config/` | `memory_api.toml`, `autonomy.toml`, `autonomy.dev.toml`, `neural_brain.toml`, `hermes_adapter.toml`, `loops.toml` (paths only, no secrets). `config/secrets.example/` shows token-file formats. |
| `docs/` | `autonomy.md`, `swipe-deck.md`, design notes, neural-brain evaluation. |
| `examples/minimal_agent.py` | A minimal agent (stdlib only) that uses every hook, including a memory **write** then search. |

## Build and run

```bash
# Rust (stable >= 1.95; tested with 1.100)
cd core/rust
cargo build --release -p xiar-memory-api
cargo test -p xiar-memory -p xiar-ontology -p xiar-owl
cargo test -p xiar-memory-api --features test-hooks      # API integration tests
cargo run -p xiar-ontology-recall -- --ofn ontologies-example/example-org_v3_promoted.ofn --report /tmp/recall.md

# Memory API (paths in config are relative to $XIAR_ROOT = repo root)
export XIAR_ROOT=$PWD/../..
target/release/xiar-memory-api --config config/memory_api.toml init    # creates master key + one token per [[clients]] (0600 files, never printed)
target/release/xiar-memory-api --config config/memory_api.toml serve   # 127.0.0.1:8771

# Neural brain (Python >= 3.11)
cd core/neural-brain && python -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt            # install a torch build that matches your GPU/CPU
python -m xiar_brain --config ../../config/neural_brain.toml serve      # 127.0.0.1:8770; creates data/neural-brain/secrets/api-tokens.json on first start
```

**Writes are fail-closed.** Memory mutations (`insert`, `delete`, `reindex`, `jobs/enqueue`, `jobs/run`,
`knowledge/remove`) are refused with *"Emergency stop"* unless the autonomy broker reports "running". Reads
work without it. Production uses root-owned `/etc/xiar/autonomy.toml` and `/etc/xiar/autonomy-state.json`.
For a local dev setup (no root), see `docs/autonomy.md` and:

```bash
export XIAR_ROOT=/path/to/this/repo
export XIAR_AUTONOMY_DEV=1
export XIAR_AUTONOMY_CONFIG=$XIAR_ROOT/config/autonomy.dev.toml
# terminal A — memory API (build *without* --features test-hooks)
cd $XIAR_ROOT/core/rust
cargo build -p xiar-memory-api
XIAR_ROOT=$XIAR_ROOT target/debug/xiar-memory-api --config $XIAR_ROOT/config/memory_api.toml init
XIAR_ROOT=$XIAR_ROOT XIAR_AUTONOMY_DEV=1 XIAR_AUTONOMY_CONFIG=$XIAR_AUTONOMY_CONFIG   target/debug/xiar-memory-api --config $XIAR_ROOT/config/memory_api.toml serve
# terminal B — dev broker
XIAR_ROOT=$XIAR_ROOT XIAR_AUTONOMY_DEV=1 XIAR_AUTONOMY_CONFIG=$XIAR_AUTONOMY_CONFIG   python $XIAR_ROOT/integrations/agent-control/dev_broker.py
# terminal C — example agent write + search
export XIAR_MEMORY_TOKEN_FILE=~/.config/xiar/memory-api-tokens.json XIAR_MEMORY_CLIENT=xiar-owner
export XIAR_BUSINESS_ID=example-business
python $XIAR_ROOT/examples/minimal_agent.py --write "Sensor Supply Co" "Delivers evaluation boards for Project Example"
python $XIAR_ROOT/examples/minimal_agent.py "Who supplies sensors?"
```

The integration tests use the `test-hooks` feature to simulate the broker; production builds cannot.

## How an agent connects

### 1. Memory API — `http://127.0.0.1:8771`

Every call is `POST` with JSON, except `GET /v1/health` (no auth).

**Headers**

| Header | Required | Meaning |
|---|---|---|
| `Authorization: Bearer <token>` | yes | A client token from the tokens file (`[auth].tokens_file`, `{client name: token}`), or a private-agent capability whose SHA-256 is listed in `[auth].dynamic_grants_file`. Wrong/missing → **401**. |
| `X-Business-Id: <slug>` | yes | The business (tenant) the call is scoped to. Missing/invalid → **400**; not granted to the token → **403**. |
| `X-Agent-Id`, `X-End-User-Id` | optional | If sent, must match the token's agent/user, else **403**. |
| `X-Read-Agent-Id`, `X-Read-User-Id` | optional | Cross-scope **read** of another agent/user; only the `xiar` agent with `read_all_agents = true`, read routes only. |

Each `[[clients]]` entry in `config/memory_api.toml` names `agent_id`, `end_user_id` and the `businesses` it may
touch (`"*"` = any, but every call must still name one). Storage is physically separate per
agent × user × business.

**Routes** (verified against `core/rust/crates/xiar-memory-api/src/lib.rs`)

| Route | Body | Returns |
|---|---|---|
| `/v1/semantic/search` | `{query, limit?, ontology?: bool, method?: "bm25"\|"embedding"\|"hybrid"}` | `200 {agent_id, end_user_id, business_id, ontology, method, hits:[{id, kind, name, summary, entity_types, source, score}]}` |
| `/v1/semantic/insert` | `{name, summary, details?, source?, entity_types?: [..]}` | `201 {business_id, id, embedding_indexed, ...}` (entity types are checked against the scope's promoted ontology) |
| `/v1/semantic/get` | `{id}` | `{item}` |
| `/v1/semantic/list` | `{query?, limit?}` | `{items, total}` |
| `/v1/semantic/delete` | `{id}` | `{deleted}` |
| `/v1/semantic/reindex` | — | reindex the scope |
| `/v1/ontology/audit` | `{action: "started"\|"completed", version?: "vN", promoted?}` | `{recorded}` |
| `/v1/jobs/enqueue` | `{job, text, source, idempotency_key?}` (e.g. `job:"meta"`) | `202 {queued, id}` — LLM population job using the MIRIX memory-agent prompts |
| `/v1/jobs/state`, `/v1/jobs/run` | — | job queue state / run now |
| `/v1/knowledge/list`, `/v1/knowledge/remove` | — / `{id}` | knowledge documents |
| `/v1/episodic/get`, `/v1/episodic/list`, `/v1/scheduler/core`, `/v1/scheduler/lifecycle` | see `scheduler.rs` | scheduler/episodic access |
| `/v1/autonomy/decide`, `/v1/autonomy/event`, `/v1/autonomy/quiesce` | see `autonomy.rs` | autonomy gate used by the XIAR runtime |

### 2. Neural brain — `http://127.0.0.1:8770`

Tokens: `paths.tokens_file` (`{"writer", "query", "admin"}`), created on first start. Agents get **query** and
**writer** only, never admin or the trainer key.

| Route | Role | Body → result |
|---|---|---|
| `GET /v1/health` | none | `{ok: true}` |
| `GET /v1/status` | query | counts/version only |
| `POST /v1/signals` | query | `{context, action}` → `{version, signals:[{text, label, confidence, why?}]}` |
| `POST /v1/lessons` | writer | `{kind, action, outcome, context?, why?, source?}` → `201 {id}`. Write-only: no route returns lessons. Secrets, money, emails and business facts are refused (422). |
| `POST /v1/train` | admin | start an idle-window training job |

`kind` ∈ correction, approval, rejection, preference, outcome, mistake; `outcome` ∈ ok, fail, rejected,
approved, prefer, corrected (`config/neural_brain.toml [lessons]`).

### 3. Hook points

These are the hook names registered by the reference adapter (`integrations/hermes-xiar/xiar_hermes/plugin.py`,
`plugin.yaml`). Any runtime can call the same services at the same moments.

| Hook | When | What XIAR does |
|---|---|---|
| `pre_llm_call` | before each LLM turn | `POST /v1/signals` for the request; suggests mental models; optional memory recall (`/v1/semantic/search`, `memory.auto_recall`); detects owner corrections of the last action → lesson. Returns `{"context": "..."}` to add to the prompt. |
| `pre_tool_call` | before a tool runs | Asks the brain about the action; a strong `fail`/`rejected` signal (≥ `gate.confidence`) returns `{"action":"approve", ...}` (ask the owner) or `{"action":"block", ...}` (`gate.directive`). |
| `post_tool_call` | after a tool runs | Failure → `mistake` lesson; success → sampled `outcome` lesson (`capture.ok_sample_rate`). |
| `post_approval_response` | owner answered an approval prompt | approve → `approval` lesson, deny → `rejection` lesson (timeouts and automatic decisions are ignored). |
| `on_session_end` | session closes | drops per-session state. |
| `post_llm_call` | after the LLM reply | remembers the reply so the next turn can detect corrections. |

### 4. Minimal example

`examples/minimal_agent.py` implements all five hooks with the standard library only:

```bash
export XIAR_MEMORY_TOKEN_FILE=~/.config/xiar/memory-api-tokens.json XIAR_MEMORY_CLIENT=xiar-owner
export XIAR_BUSINESS_ID=example-business XIAR_BRAIN_TOKENS_FILE=data/neural-brain/secrets/api-tokens.json
python examples/minimal_agent.py "Which vendors supply sensors?"
# after the autonomy broker is running (dev or production):
python examples/minimal_agent.py --write "Sensor Supply Co" "Delivers evaluation boards for Project Example"
```

## Licence

- XIAR's code: **GNU AGPL-3.0-or-later** (`LICENSE`) with an AGPLv3 §7(b) additional term requiring the visible
  credit **"Powered by XIAR V0 — https://xiarv0.com"** (`LICENSE-ADDITIONAL-TERMS.md`, **draft for counsel review**).
- The memory core is ported from **MIRIX** (Apache-2.0); its notice is kept in `NOTICE` and
  `THIRD_PARTY_LICENSES.md`. The original MIRIX prompts stay Apache-2.0 in `core/rust/prompts/mirix/`.
- Third-party components keep their own licences: see `THIRD_PARTY_LICENSES.md`.
- Copyright holder and credits: see `NOTICE`. "XIAR" is a trademark of its owner (policy:
  https://xiarv0.com/trademark). Third-party names are used only to identify their projects.
