# Phase 5 — Ontology recall evaluation

Model / retrieval: retrieval-only (BM25 + entity_types); no LLM judge. A local LLM was available but unused for this metric (MigrationPlan: facts as semantic memories + class questions).

| Mode | recall@5 | MRR | answer accuracy | hits/N |
|---|---|---|---|---|
| ontology OFF | 0.600 | 0.400 | 0.600 | 6/10 |
| ontology ON  | 1.000 | 0.817 | 1.000 | 10/10 |

Delta recall@5 (ON − OFF): +0.400

Result: ontology **helps** on this gold set.

## Per-question

| id | off_hit | on_hit | off_rank | on_rank | question |
|---|---|---|---|---|---|
| q1 | false | true | None | Some(1) | Which vendors are mentioned? |
| q2 | false | true | None | Some(1) | Which directors are mentioned? |
| q3 | true | true | Some(3) | Some(3) | Who does Ingrid Solheim report to? |
| q4 | true | true | Some(1) | Some(1) | What milestones does Project Example have? |
| q5 | false | true | None | Some(1) | Which meeting rooms are mentioned? |
| q6 | true | true | Some(1) | Some(1) | Which engineers are assigned to Project Example? |
| q7 | true | true | Some(3) | Some(3) | Who approves budgets? |
| q8 | true | true | Some(3) | Some(1) | What is the budget for Project Example? |
| q9 | true | true | Some(1) | Some(2) | Which vendor supplies sensors? |
| q10 | false | true | None | Some(1) | List all Vendor class entities |

## Integration changes

- Semantic `text_fields` now includes `entity_types` (array joined for BM25).
- `ontology_retrieve::search_semantic_ontology` fuses BM25 (expanded query) with `list_with_any` on `entity_types`.
- Gold facts seeded as typed semantic memories (MigrationPlan requirement).

## 5d / 5g

- `xiar-memory::business_ns` — `business:<slug>` scopes, shared `core`, cross-biz deny tests.
- `xiar-memory::governance` — MirixGovernor ledger for promote/forget/cross-business proposal/vault erase/namespace write.

## Parked (t36u)

- 5e neural associative layer, 5f per-business neural adapters, neural parts of 5h/5i: pending discussion (open-source prebuilt agentic-memory neural options vs custom). No neural code committed.
