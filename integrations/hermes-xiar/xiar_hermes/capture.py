# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Experience capture: turn Hermes events into lesson records (write-only to the sealed store)."""
from __future__ import annotations

import json
import re


def summarize_action(tool_name: str, args: dict | None, max_chars: int) -> str:
    args = args or {}
    for k in ("command", "cmd", "query", "path", "url", "file_path", "name", "action"):
        if isinstance(args.get(k), str) and args[k].strip():
            return f"{tool_name}: {args[k].strip()}"[:max_chars]
    try:
        return f"{tool_name}: {json.dumps(args, sort_keys=True, ensure_ascii=False)}"[:max_chars]
    except (TypeError, ValueError):
        return f"{tool_name}"[:max_chars]


def classify_result(result, failure_patterns: list[re.Pattern]) -> tuple[bool, str]:
    """(failed, short reason). Understands JSON tool results and plain text."""
    text = result if isinstance(result, str) else json.dumps(result, default=str)
    try:
        obj = json.loads(text) if isinstance(text, str) and text.lstrip().startswith("{") else None
    except ValueError:
        obj = None
    if isinstance(obj, dict):
        if obj.get("error"):
            return True, str(obj["error"]).splitlines()[0][:200]
        if obj.get("success") is False or obj.get("ok") is False:
            return True, str(obj.get("message") or obj.get("output") or "tool reported failure").splitlines()[0][:200]
        code = obj.get("exit_code", obj.get("returncode"))
        if isinstance(code, int) and code != 0:
            return True, f"exit code {code}"
        return False, ""
    for rx in failure_patterns:
        m = rx.search(text or "")
        if m:
            line = (text[m.start():].splitlines() or [""])[0]
            return True, line[:200]
    return False, ""


def is_correction(user_message: str, patterns: list[re.Pattern]) -> bool:
    msg = (user_message or "").strip()
    return any(rx.search(msg) for rx in patterns)
