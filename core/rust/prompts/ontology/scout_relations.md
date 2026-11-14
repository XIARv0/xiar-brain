---
source: legacy XIAR ontology service, agents/scout.py::_build_relation_extraction_prompt
origin: XIAR ontology service (own work); placeholders written as {{name}}; the task paragraph was made stricter (2026-10-24)
---

Extract relationships between concepts. Return ONLY a JSON object.

CONCEPTS: {{classes}}

TEXT:
{{text}}

Task: Identify 2-5 relationships between the KINDS of things listed above (not between individuals). Use simple verb predicates in camelCase, such as reportsTo, uses, supplies, approves, hasMilestone. Only use the concepts listed above, spelled exactly as listed.

Example output format:
{"relations": [{"subject": "Agent", "predicate": "uses", "object": "Task", "description": "Agent executes task", "confidence": 0.8}]}

Your JSON output (no other text):
