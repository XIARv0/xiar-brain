# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Render a sweep summary (sweep-summary.json) as a markdown table. Usage: python eval/report.py <summary.json>"""
import json
import sys

COLS = ["recall", "day1", "forget", "para", "avoid@gate", "avoid@warn", "avoid_label", "fa@warn", "why"]


def render(rows):
    out = ["| set | variant | " + " | ".join(COLS) + " |", "|---|---|" + "---|" * len(COLS)]
    for r in rows:
        out.append(f"| {r['set']} | {r['variant']} | " + " | ".join(f"{r[c]:.3f}" for c in COLS) + " |")
    return "\n".join(out)


if __name__ == "__main__":
    print(render(json.load(open(sys.argv[1], encoding="utf-8"))))
