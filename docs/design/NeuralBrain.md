# XIAR NeuralBrain

Status: **BUILT 2026-11-05 (America/New_York).** It was designed in owner messages t39u-t42u, built under t43u, and the tokenizer was chosen under t44u/t45u.
Code is in `core/neural-brain/`. Config is `config/neural_brain.toml`: every size, path, schedule and threshold is a config variable. You can override any of them with `XIAR_BRAIN__<section>__<key>=<toml literal>`.
The Hermes integration is documented in `HermesIntegration.md`. The eval report is in `docs/eval/neural-brain-eval-2026-11-05.md`.

## 1. Scope split

| Data | Where it lives | In the network? |
|---|---|---|
| Business/client data (facts, CRM, docs, money) | MIRIX fork **Semantic layer**, multi-tenant (`xiar-memory-api`) | **No** |
| XIAR's learned behaviour and lessons | **NeuralBrain** (weights) | Yes |
| Plain log of every lesson | **Sealed lesson store** (training job only) | Source for retraining only |

### Business data multi-tenant scoping (as built)
- **API level.** `xiar-memory-api` (Rust, axum, `core/rust/crates/xiar-memory-api`) answers at 127.0.0.1:8771.
  - Every `/v1/semantic/*` call needs a bearer token **and** `X-Business-Id`.
  - A missing id returns 400. A business the token is not granted returns 403. A bad token returns 401. The token compare is constant-time.
  - Tokens are per client, with a business grant list (`config/memory_api.toml`).
- **DB level.** `xiar_memory::tenant` gives each business its **own store** at `<root>/tenants/<slug>/`, bound by a `.xiar-tenant` marker.
  - `TenantRegistry` refuses swapped or unbound directories.
  - `TenantSemantic` stamps `business:<slug>` (it extends `business_ns`) on insert and refuses foreign scopes.
  - Get and search filter to the tenant. A query can't reach another business's rows because they are not in its database.
- The Hermes tool `xiar_memory_search` refuses calls without a business id (verified inside real Hermes, see section 11.3).
- Cross-business insights remain quarantined proposals that need owner approval (see Memory.md).

## 2. The brain: one newborn network
- **One** network for XIAR. There are no per-business models; the per-business LoRA/adapter idea was dropped.
- **Random init. No pretrained weights.** Only the *tokenizer* is reused (section 9). It has no weights and carries no knowledge.
- It learns from:
  - **The owner:** corrections, approvals and denials.
  - **Its own experience:** action outcomes, including the failure reason.
- It learns behaviour and lessons, not client facts.
  - The allow-list (`xiar_brain/allowlist.py`, `lessons.*` in config) accepts only the fields kind/action/context/outcome/why/source.
  - It accepts only configured kinds and outcomes, caps field length, and rejects records that match the secret/credential/money deny patterns.

## 3. Architecture (as built)
- **Model** (`xiar_brain/model.py`, `pkm.py`):
  - Small decoder-only transformer: d_model 256, 4 layers.
  - **Product-key memory** replaces the FFN in layers [1, 3], following *Memory Layers at Scale* (arXiv 2412.09764). It uses n_keys 128, which gives 128^2 = 16,384 slots per layer.
  - Embeddings are tied. Total is about 19.4M parameters (12.3M of that is the embedding).
- **Encoding** (`encoding.py`): `<ctx> context <act> action <out> LABEL <why> reason`.
  - The label token gets extra loss weight.
  - The brain predicts the outcome label and generates the "why".
- **Learning** (`trainer.py`):
  - The first window is a dense bootstrap. After that comes **sparse memory finetuning** (arXiv 2510.15103).
  - Only the top-t slots, ranked by TF-IDF of slot access against background access, get gradient. Non-memory parameters use a lower LR.
  - **Replay:** each window mixes in a configurable fraction of older lessons.
  - **Growable memory:** when slot utilisation passes `memory.grow_threshold`, `grow_step` sub-keys are added up to `max_n_keys`. New keys are initialised small so old routing stays stable.
- **Idle windows:**
  - The trainer runs only when free VRAM ≥ `train.idle_min_free_vram_gb` and GPU utilisation ≤ the threshold (checked with nvidia-smi). The window length is capped.
  - A user-level scheduled task runs it every `check_interval_minutes`. Hermes inference keeps priority.
- **Checkpoints** (`checkpoints.py`):
  - Checkpoints are versioned (v0001, ...) and use a `current` pointer with rollback.
  - A run is promoted only if recall_all ≥ `eval.promote_min_recall` and forgetting ≤ `eval.max_forgetting`.
- **Signals** (`brain.py`):
  - confidence = P(label) × familiarity.
  - familiarity = sigmoid((`familiar_nll` − mean NLL of the action) / scale). An action the brain has never seen therefore can't produce a confident signal.
  - Signals below `signals.min_confidence` are dropped.

## 4. How XIAR uses it
1. Before Hermes acts, the `xiar` Hermes plugin (`pre_tool_call` and `pre_llm_call`) sends context + planned action to `POST /v1/signals`.
2. The brain returns signals such as "failed before - <why>", "owner usually rejects", or "owner corrected". Each has a confidence.
3. Hermes uses them:
   - `pre_llm_call` injects them as context. Hermes stays the voice.
   - `pre_tool_call` returns an **approval gate** (Hermes asks the owner) when a fail/rejected signal is ≥ `gate.min_confidence` (0.8).
4. Outcomes become lessons through `POST /v1/lessons` (writer token, write-only):
   - Tool results from `post_tool_call`: failures always, successes sampled.
   - Owner approve/deny from `post_approval_response`. Timeouts and smart-approve are not learned.
   - Corrections in owner messages, matched by regex.

## 5. Sealed lesson store (as built, `xiar_brain/sealed_store.py`)
- **Crypto:** each row is ECIES-sealed (X25519 + HKDF-SHA256 + AES-256-GCM).
  - The service and writers hold **only the public key**. They can append, but nobody holding the writer's key can decrypt.
  - The private key (`paths.sealed_private_key`) is loaded only by the trainer process and never leaves the machine that runs the trainer.
- **API:**
  - The store has no read route: `GET /v1/lessons` returns 405.
  - `SealedWriter.read()` raises.
  - `SealedReader` requires role `trainer` plus the private key.
  - `/v1/train` needs the admin token. Writer and query tokens get 401.
  - Clients that only query or write lessons hold only the query and writer tokens.
- **DB:** SQLite triggers block UPDATE and DELETE on lessons and meta, so the log is append-only. A `.xiar-sealed` marker lets indexers skip the directory (`is_sealed_path`).
- **Verified:**
  - The pipeline check (every sweep run) wrote 104 lessons through the API, trained on cuda and promoted v0001 (recall_all 0.98 at the new defaults). It then grepped every file in the data directory: **0 plaintext hits out of 131 probe strings**.
  - Unit tests cover the deny paths. Hermes search and MIRIX never see the store.

## 6. Honest limits
- A from-scratch network will not learn language from small personal data. Hermes does language; the brain only gives signals and short "why" strings, and those can be slightly garbled. For example, the e2e run produced "deleted the download broke models".
- No neural network guarantees zero forgetting. The mitigations are sparse slot updates, replay, growable memory, eval-gated promotion, rollback, and retraining from the sealed store.
- **Generalisation to new wording is weak at this scale** (held-out paraphrase about 0.2-0.28; section 11). The brain is reliable for actions it has actually seen, and not yet for "similar" ones.

## 7. Evaluation (as built, `core/neural-brain/eval/`)
- `synthetic.py` simulates days of experience: owner preferences, failures with reasons, approvals and denials, paraphrases, and novel actions.
- `harness.py` trains day by day with the real trainer and reports these per day and per variant:
  - recall on trained lessons and on day-1 lessons
  - forgetting
  - held-out paraphrase recall
  - "avoids past mistake without search" at the gate (0.8) and warn (0.6) levels, plus label-only
  - false alarms on novel actions
  - exact why
- Variants: smf/dense × replay/noreplay.
- `pipeline_check` runs the real service → sealed store → trainer → promotion, then greps for plaintext.
- `sweep.py` runs a hyper-parameter sweep. Results are in section 11.

## 8. Former open questions - resolved
| Question | Decision |
|---|---|
| Tokenizer | Existing permissive BPE tokenizer (Mistral-7B-v0.1, Apache-2.0), tokenizer only. See section 9. |
| Size vs VRAM | ~19.4M params, 16,384 slots/layer. A 6-day eval window took 5-12 s at 300 epochs on a CUDA GPU. Peak VRAM was not measured separately; the idle gate only trains while ≥ `train.idle_min_free_vram_gb` is free, so Hermes keeps priority. Slots grow (seen 128→160→224 sub-keys in eval) up to `max_n_keys` 512 (262k slots). |
| Encoding | Structured text fields `<ctx>/<act>/<out>/<why>` (section 3). |
| Cadence | Idle-triggered: check every `check_interval_minutes`, train when ≥ `min_new_lessons` and idle; `train --force` for manual. |
| Signal format | Predicted label (probabilities over kinds) + generated short "why" + confidence. |
| Retention / owner view | Sealed store keeps everything (append-only); owner-visible view is *signals* via `xiar-brain status` / `/v1/status` (counts + version), never the store. A trainer-side owner export is a saved question. |

## 9. Tokenizer decision (owner t44u/t45u)
An existing open-source tokenizer is loaded through HF `tokenizers` (**tokenizer.json only, no weights**). Config: `tokenizer.path` / `tokenizer.hf_repo`. Benchmark: `core/neural-brain/bench/` (`tokenizer_results.json`). The sample was English prose, code, lesson-style short records, multilingual text and numbers. Lower tokens-per-char (tpc) means better compression.

| Tokenizer | License | Vocab | Embed params @d256 | tpc all | tpc lessons | tpc multilingual | tpc numbers | "1234567" |
|---|---|---|---|---|---|---|---|---|
| GPT-2 | MIT | 50,257 | 19.3M | 0.3402 | 0.2574 | 0.674 | 0.435 | 3 tok |
| GPT-NeoX-20B | Apache-2.0 | 50,277 | 19.3M | 0.3054 | 0.2550 | 0.529 | 0.440 | 2 tok |
| SmolLM2 | Apache-2.0 | 49,152 | 18.9M | 0.3447 | 0.2562 | 0.659 | 0.711 | 7 tok |
| **Mistral-7B-v0.1** | Apache-2.0 | **32,000** | **12.3M** | 0.3406 | 0.2698 | 0.510 | 0.737 | 8 (per digit) |
| Mistral Tekken | Apache-2.0 | 131,072 | 50.3M | 0.2977 | - | 0.372 | - | - |
| OLMo-2 (= cl100k) | Apache-2.0 | 100,278 | 38.5M | 0.2771 | - | - | - | - |
| Qwen2.5 / Qwen3 | Apache-2.0 | 151,643 | 58.2M | 0.2917 | - | - | - | - |
| cl100k (tiktoken) | MIT | 100,277 | 38.5M | 0.2771 | - | - | - | - |
| o200k (tiktoken) | MIT | 200,019 | 76.8M | 0.2646 | - | 0.310 | - | - |

Llama-3 and Gemma-3 were excluded because they are gated and use non-permissive licences.

**Pick: Mistral-7B-v0.1.**
- **Smallest embedding cost.** 12.3M parameters, against 19-77M for the others. In a ~19M-parameter newborn model, a large vocab would eat the whole budget and leave most rows untrained.
- **Byte fallback.** It is lossless and never produces UNK.
- **Per-digit numbers.** These are consistent for exit codes, ports and versions.
- **Trade-off:** about 11% more tokens than NeoX overall. GPT-NeoX-20B is the configured alternative.

## 10. Where it runs, and why
- Train and serve on a machine that has a suitable GPU (or CPU fallback, if enabled), using a torch build that matches that device, in `core/neural-brain/.venv`.
- Optional user-level scheduled tasks (`scripts/install-host.ps1`, with `-Remove` to undo; no admin or UAC needed):
  - `XIAR-NeuralBrain-Service` (pythonw serve at logon; logs go to `data/neural-brain/logs`)
  - `XIAR-NeuralBrain-Trainer` (idle check every `check_interval_minutes`)
  - `XIAR-NeuralBrain-Tunnel` (optional; if the API should be reached from another machine, a reverse SSH tunnel can present it at `127.0.0.1:8770` there)
- The trainer private key, the admin token and the store stay on the machine that runs the trainer (`data/neural-brain/`, gitignored).

## 11. Results (real runs, 2026-11-05)
See `docs/eval/neural-brain-eval-2026-11-05.md` for the full tables.

1. **Defaults chosen by the sweep:** `smf.top_t` = 4000 and `train.epochs_per_window` = 300. With smf+replay this gave final recall 0.990, day-1 recall 0.944, forgetting 0.056, exact why 1.0, avoid@gate 0.128 and 0 false alarms.
2. **Dense+replay** at the same settings gave recall 0.971, forgetting 0.0, avoid@gate 0.298 and false alarms 0.025. SMF stays the default because:
   - it has the better recall and exact why;
   - its sparse updates are what limits forgetting at scale (paper);
   - `smf.enabled = false` switches to dense.
   This should be re-checked once real lessons pass about 1k.
3. **Without replay, every variant forgets heavily** (final recall 0.41-0.52). Replay is essential.
4. **End-to-end inside real Hermes**, using a separate test brain instance so the owner's brain was not polluted:
   - 11 lessons were captured from Hermes hooks and trained on cuda (v0001, recall 1.0).
   - The trained brain then **gated all 3 previously failed commands** with confidence 0.98 and the right reason.
   - It did not gate any of the 3 previously successful ones.

## Maps to UpgradePlan Phase 5
5e (brain), 5f (sealed store + learning loop), 5h neural part (signals into planning), 5i neural part (evaluation above).
