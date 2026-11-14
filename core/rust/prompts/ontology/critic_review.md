---
source: legacy XIAR ontology service, agents/critic.py::_build_semantic_review_prompt
origin: XIAR ontology service (own work); prompt text unmodified, placeholders written as {{name}}
---

Review this ontology for semantic quality. Return ONLY a JSON object.

CLASSES: {{classes}}

RELATIONS:
{{relations}}

Task: Identify 1-3 potential improvements for clarity, consistency, or completeness.

Example output format:
{"suggestions": ["Consider adding inverse relation for 'uses'", "Agent and Task could benefit from explicit hierarchy"]}

Your JSON output (no other text):
