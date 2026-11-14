# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
import pytest
from xiar_brain.allowlist import LessonPolicy, LessonRejected

OK = {"kind": "correction", "action": "messages: send summary without draft", "outcome": "corrected",
      "why": "owner wants drafts first", "context": "session cli", "source": "hermes"}


@pytest.mark.parametrize("why", [
    "password: hunter2", "use key sk-abcdefghijklmnopqrstuv", "token= abc", "mail bob@example.com",
    "card 4111 1111 1111 1111", "costs $45", "about 300 euros", "business:greenlife pricing",
    "ghp_abcdefghijklmnopqrstuvwxyz0123", "AKIAABCDEFGHIJKLMNOP",
])
def test_refuses_secrets_money_contacts_business(cfg, why):
    with pytest.raises(LessonRejected):
        LessonPolicy(cfg).validate(dict(OK, why=why))


def test_refuses_unknown_fields_kinds_outcomes(cfg):
    p = LessonPolicy(cfg)
    with pytest.raises(LessonRejected):
        p.validate(dict(OK, business_id="greenlife"))
    with pytest.raises(LessonRejected):
        p.validate(dict(OK, kind="crm_fact"))
    with pytest.raises(LessonRejected):
        p.validate(dict(OK, outcome="maybe"))


def test_accepts_behaviour_lesson(cfg):
    assert LessonPolicy(cfg).validate(OK)["why"] == "owner wants drafts first"
