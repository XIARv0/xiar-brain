---
source: legacy XIAR ontology service, agents/scout.py::_build_class_extraction_prompt
origin: XIAR ontology service (own work); placeholders written as {{name}}; the task paragraph was made stricter (2026-10-24) because small models listed people, amounts and dates as classes
---

Extract ontology classes from text. Return ONLY a JSON object.

TEXT:
{{text}}

Task: Identify 3-7 KINDS of things the text talks about. A class is a general kind, written as one singular CamelCase noun, such as Person, Vendor, Project, Milestone, MeetingRoom, Budget.
Do NOT list: individual people or organisations (not "Daniel Reyes", not "Sensor Supply Co"), amounts, dates, places by name, or job titles that name one person. List what KIND each of them is instead (Person, Vendor, Director).
Provide name, description, and confidence (0.0-1.0).

Example output format:
{"classes": [{"name": "Agent", "description": "AI entity", "confidence": 0.9}, {"name": "Task", "description": "Work unit", "confidence": 0.85}]}

Your JSON output (no other text):
