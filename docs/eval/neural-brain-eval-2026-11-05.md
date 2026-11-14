# NeuralBrain eval - 2026-11-05 (America/New_York)

These are real runs on a CUDA GPU with `core/neural-brain/eval/harness.py` and `sweep.py`.
- **Data:** synthetic, 6 simulated days, 104 lessons (about 18 per day) plus 40 novel actions for false alarms. Each day is trained as one idle window with the real trainer.
- **Variants:** SMF or dense updates, each with or without replay.
- Raw data: `sweep-summary.json` (`docs/eval/sweep-summary-2026-11-05.json`); per-run JSON and logs are in `data/neural-brain/eval/sweep/` (gitignored).

## Metrics (final day unless noted)
| Metric | Meaning |
|---|---|
| recall | Correct outcome label on all lessons trained so far |
| day1 | Correct label on day-1 lessons after day 6 |
| forget | Peak day-1 recall minus final day-1 recall |
| para | Held-out paraphrases of trained lessons |
| avoid@gate | "Avoids past mistake without search": the brain flags a past failure or rejection in a **new context** at confidence â‰¥ 0.8, which triggers the Hermes approval gate. No search is involved. |
| avoid@warn | The same check at confidence â‰¥ 0.6, which injects a warning |
| avoid_label | The same check, top label only, with no confidence threshold |
| fa@warn | False alarms on 40 never-seen actions at â‰¥ 0.6 |
| why | Generated "why" exactly matches the stored reason |

## Sweep
| set | variant | recall | day1 | forget | para | avoid@gate | avoid@warn | avoid_label | fa@warn | why |
|---|---|---|---|---|---|---|---|---|---|---|
| default | smf_replay | 0.702 | 0.722 | 0.222 | 0.206 | 0.000 | 0.021 | 0.851 | 0.000 | 0.606 |
| default | smf_noreplay | 0.462 | 0.500 | 0.333 | 0.206 | 0.000 | 0.000 | 0.425 | 0.000 | 0.151 |
| default | dense_replay | 0.971 | 1.000 | 0.000 | 0.250 | 0.000 | 0.000 | 0.511 | 0.000 | 0.939 |
| default | dense_noreplay | 0.433 | 0.333 | 0.500 | 0.206 | 0.043 | 0.043 | 0.511 | 0.000 | 0.121 |
| topt4000 | smf_replay | 0.942 | 0.944 | 0.056 | 0.141 | 0.106 | 0.170 | 0.872 | 0.000 | 0.879 |
| topt4000 | smf_noreplay | 0.423 | 0.278 | 0.555 | 0.196 | 0.000 | 0.043 | 0.468 | 0.000 | 0.182 |
| topt4000 | dense_replay | 0.971 | 1.000 | 0.000 | 0.250 | 0.000 | 0.000 | 0.511 | 0.000 | 0.939 |
| topt4000 | dense_noreplay | 0.433 | 0.333 | 0.500 | 0.206 | 0.043 | 0.043 | 0.511 | 0.000 | 0.121 |
| ep300 | smf_replay | 0.981 | 0.944 | 0.056 | 0.228 | 0.128 | 0.170 | 0.702 | 0.000 | 0.939 |
| ep300 | smf_noreplay | 0.500 | 0.389 | 0.611 | 0.217 | 0.000 | 0.043 | 0.255 | 0.000 | 0.151 |
| ep300 | dense_replay | 0.971 | 1.000 | 0.000 | 0.261 | 0.298 | 0.340 | 0.638 | 0.025 | 0.939 |
| ep300 | dense_noreplay | 0.413 | 0.111 | 0.889 | 0.250 | 0.000 | 0.000 | 0.447 | 0.000 | 0.121 |
| topt4000_ep300 | smf_replay | 0.990 | 0.944 | 0.056 | 0.239 | 0.128 | 0.170 | 0.702 | 0.000 | 1.000 |
| topt4000_ep300 | smf_noreplay | 0.519 | 0.278 | 0.722 | 0.283 | 0.021 | 0.043 | 0.425 | 0.000 | 0.121 |
| topt4000_ep300 | dense_replay | 0.971 | 1.000 | 0.000 | 0.261 | 0.298 | 0.340 | 0.638 | 0.025 | 0.939 |
| topt4000_ep300 | dense_noreplay | 0.413 | 0.111 | 0.889 | 0.250 | 0.000 | 0.000 | 0.447 | 0.000 | 0.121 |
| memlr3e-2_topt4000 | smf_replay | 0.904 | 0.889 | 0.111 | 0.196 | 0.021 | 0.128 | 0.872 | 0.000 | 0.758 |
| memlr3e-2_topt4000 | smf_noreplay | 0.442 | 0.167 | 0.444 | 0.185 | 0.000 | 0.021 | 0.511 | 0.000 | 0.121 |
| memlr3e-2_topt4000 | dense_replay | 1.000 | 1.000 | 0.000 | 0.228 | 0.277 | 0.340 | 0.808 | 0.000 | 0.939 |
| memlr3e-2_topt4000 | dense_noreplay | 0.423 | 0.222 | 0.389 | 0.250 | 0.043 | 0.043 | 0.468 | 0.000 | 0.091 |

## Reading it honestly
- **Defaults chosen:** `smf.top_t` = 4000 and `train.epochs_per_window` = 300.
  - With SMF+replay this gives recall 0.990, forgetting 0.056, exact why 1.000 and 0 false alarms.
  - The old defaults (top_t 1000, 150 epochs) under-trained SMF: recall 0.702, forgetting 0.222.
- **Dense+replay** at the same settings gives recall 0.971, forgetting 0.000, and better avoid@gate (0.298 vs 0.128), at the cost of a 0.025 false-alarm rate.
  - SMF stays the default because of the paper's forgetting results at scale. Re-run this sweep once there are about 1k real lessons.
  - `smf.enabled = false` switches to dense.
- **Replay is essential.** Every no-replay variant ends at recall 0.41-0.52, with day-1 recall as low as 0.11.
- **Weak spots:**
  - **Generalising to new wording and contexts** is weak at this size. Paraphrase recall is 0.14-0.28.
  - **Avoid@gate in a new context** peaks at about 0.38 on early days and falls to 0.13-0.30 by day 6.
  - The brain is reliable on the same action and context it saw, which is the common case for tool calls; see the e2e below.
- `top_t` has no effect on the dense rows (they are identical in default and topt4000), as expected. This is a sanity check.

## Pipeline check (every run)
- 104 lessons went through the real service into the sealed store, and the trainer ran on cuda and promoted v0001. recall_all was 0.875 at the old defaults and 0.981 at the new ones.
- **0 plaintext hits out of 131 probe texts** across every file in the data directory.
- Signals through the API flagged 4/10 (old defaults) and 6/10 (new defaults) of 10 failure-lesson probes.

## End-to-end inside real Hermes (separate test brain)
- 11 lessons came from real Hermes hook calls. They were trained once (v0001, recall 1.0, about 7 s on cuda).
- The `pre_tool_call` gate fired for **3/3** previously failed commands (confidence 0.98, with the correct reason) and **0/3** previously successful commands.
