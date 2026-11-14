# prompts/mirix: original MIRIX prompts (kept under MIRIX's licence)

These files are the original LLM prompts from **MIRIX** (https://github.com/Mirix-AI/MIRIX), used by XIAR only
as fallbacks/backups. They are **not** relicensed: they stay under MIRIX's own licence, the
**Apache License, Version 2.0** (full text, as published by MIRIX, in `LICENSE` in this folder).

- Copyright: Mirix-AI and MIRIX contributors. The upstream LICENSE file carries the line
  "Copyright 2023, Letta authors" (MIRIX derives from Letta, formerly MemGPT, also Apache-2.0).
- SPDX-License-Identifier: Apache-2.0
- Upstream commit used: `8cb06a62` (MIRIX `main`, 2026-08-20). Licence verified from the official repository's
  LICENSE file on 2026-11-14: https://github.com/Mirix-AI/MIRIX/blob/main/LICENSE

## Changes made by XIAR (Apache-2.0 section 4(b) notice of modification)

- Each `.txt` prompt was converted to a Markdown file (`.md`) and given a YAML provenance header
  (`source:` path @ commit, `origin:` line). The prompt text itself is otherwise unmodified, as stated in
  each file's header.
- `base/summarizer.md` was extracted from the Python string constant in `mirix/prompts/gpt_summarize.py`
  (text unmodified, `WORD_LIMIT = 100`).
- Files were moved from `prompts/{base,auto_dream,screen_monitor,personas}` to
  `prompts/mirix/{base,auto_dream,screen_monitor,personas}` (2026-11-14) to keep MIRIX-licensed material
  in its own folder; `xiar-memory-api/src/jobs.rs` was updated to the new path.

| Folder | Upstream source |
|---|---|
| `base/` | `mirix/prompts/system/base/*.txt` and `mirix/prompts/gpt_summarize.py` |
| `auto_dream/` | `mirix/prompts/system/base/auto_dream_agent/*.txt` |
| `screen_monitor/` | `mirix/prompts/system/screen_monitor/*.txt` |
| `personas/` | `mirix/prompts/personas/*.txt` |

XIAR's own prompts live outside this folder (`prompts/ontology/`, `prompts/base/core_memory_block.md`) and are
AGPL-3.0-or-later.
