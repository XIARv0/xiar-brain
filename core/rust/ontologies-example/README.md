<!-- SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com> -->
<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->

# Example ontology

A small, fully synthetic example for the ontology builder/expander and its tests. The company, people and
project ("Example Co", "Project Example") are fictional.

- `seeds/org_chart.txt`, `seeds/project_plan.txt`: the two source texts the builder read.
- `example-org_v3_promoted.ofn`: the promoted v3 ontology the builder produced from them in a test run
  (11 classes, 10 object properties, OWL functional syntax).

Used by:
- `crates/xiar-memory-api/tests/api.rs` (`ontology_expansion_in_live_search`);
- `crates/xiar-ontology-recall` (recall@5 ontology OFF 0.60 → ON 1.00 on 10 gold questions):
  `cargo run -p xiar-ontology-recall -- --ofn ontologies-example/example-org_v3_promoted.ofn --report /tmp/recall.md`

No ontology from any live database is included.
