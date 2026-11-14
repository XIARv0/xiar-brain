// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The ontology service: the operations the legacy HTTP API offered, as plain methods. Governance rules carried over:
//! an ontology that fails the logical check is rejected when engineered and can never be promoted (not even forced);
//! promotion moves draft -> validated -> promoted, each step needing a quality score of at least 0.95 unless forced;
//! only a promoted version can be exported; re-ingestion is recommended from what changed between versions.

use crate::consistency;
use crate::critic::Critic;
use crate::engineer::{self, parts};
use crate::models::*;
use crate::registry::{Registry, RegistryError};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use xiar_owl::functional::write;
use xiar_owl::model::{Axiom, ClassExpr, Ontology};
use xiar_owl::reasoner::Reasoner;

pub const QUALITY_THRESHOLD: f64 = 0.95;
const MAX_EXPANDED_TERMS: usize = 20;
const MAX_RELATIONS_PER_ENTITY: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("ontology version {0:?} not found")]
    NotFound(String),
    #[error("no base ontology found: register a first version before engineering")]
    NoBase,
    #[error("OWL consistency check FAILED: {0}. Ontology rejected.")]
    Rejected(String),
    #[error("only promoted ontologies can be exported (this one is {0})")]
    NotPromoted(&'static str),
    #[error("no promoted ontology available")]
    NothingPromoted,
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error("could not write the export: {0}")]
    Export(String),
}

pub type Result<T> = std::result::Result<T, ServiceError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportInfo {
    pub version: String,
    pub path: std::path::PathBuf,
    pub bytes: usize,
    pub triples: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    pub ontology_version: Option<String>,
    pub expanded_terms: Vec<String>,
    pub original_query: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationRef {
    pub kind: &'static str,
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationsResult {
    pub ontology_version: String,
    pub relations: BTreeMap<String, Vec<RelationRef>>,
    pub entity_count: usize,
    pub total_relations_found: usize,
}

/// The starting point of the first build: an ontology with no classes yet.
pub const EMPTY_BASE: &str = "Prefix(x:=<http://xiar.ai/ontology#>)
Ontology(<http://xiar.ai/ontology>)";

pub struct OntologyService {
    pub registry: Arc<Registry>,
    critic: Critic,
}

/// Direct named superclasses of every class: from the reasoner when it can classify, else from the stated SubClassOf axioms.
fn hierarchy(o: &Ontology) -> BTreeMap<String, BTreeSet<String>> {
    if let Ok(k) = Reasoner::new(o).classify() {
        let mut m: BTreeMap<String, BTreeSet<String>> = k.direct_supers.iter().map(|(c, s)| (c.local().to_string(), s.iter().map(|x| x.local().to_string()).collect())).collect();
        for c in &k.unsatisfiable {
            m.entry(c.local().to_string()).or_default();
        }
        return m;
    }
    let mut m: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for c in o.class_names() {
        m.entry(c.local().to_string()).or_default();
    }
    for a in &o.axioms {
        if let Axiom::SubClassOf(ClassExpr::Class(a), ClassExpr::Class(b)) = a {
            m.entry(a.local().to_string()).or_default().insert(b.local().to_string());
        }
    }
    m
}

impl OntologyService {
    pub fn new(registry: Arc<Registry>, critic: Critic) -> Self {
        Self { registry, critic }
    }

    fn load(&self, version: &str) -> Result<Ontology> {
        self.registry.ontology(version)?.ok_or_else(|| ServiceError::NotFound(version.to_string()))
    }

    /// Makes sure there is a first version to build on: when nothing is registered yet, an empty ontology is, so the first build has a base.
    /// Returns whether it had to make one.
    pub fn ensure_base(&self) -> Result<bool> {
        if self.registry.latest(None)?.is_some() {
            return Ok(false);
        }
        self.registry.register_text(EMPTY_BASE, "system", false, Some("empty starting point".into()), Status::Draft)?;
        Ok(true)
    }

    /// Merge candidates into a base version (default: the newest) and register the result as a draft. The result must pass
    /// the logical check first.
    pub fn engineer(&self, candidates: &OntologyCandidate, base_version: Option<&str>) -> Result<EngineerResponse> {
        let base = match base_version {
            Some(v) => self.registry.get(v)?.ok_or_else(|| ServiceError::NotFound(v.to_string()))?,
            None => self.registry.latest(None)?.ok_or(ServiceError::NoBase)?,
        };
        let base_onto = self.load(&base.version)?;
        let (new_onto, mut response) = engineer::engineer(candidates, &base_onto, &base.version);
        let check = consistency::check(&new_onto);
        if !check.is_consistent {
            return Err(ServiceError::Rejected(check.errors.iter().take(3).cloned().collect::<Vec<_>>().join("; ")));
        }
        let info = self.registry.register(&new_onto, "engineer_agent", false, Some(response.diff_summary.clone()), Status::Draft, Some(check))?;
        response.new_version = info.version;
        Ok(response)
    }

    /// Review a version: the logical check (stored with the version) plus the critic's structural review and suggestions.
    pub fn validate(&self, version: &str) -> Result<ValidationResult> {
        let o = self.load(version)?;
        let check = consistency::check(&o);
        self.registry.set_consistency(version, check.clone())?;
        let mut result = self.critic.validate(&o);
        if !check.is_consistent {
            result.errors.extend(check.errors);
            result.is_valid = false;
        }
        result.warnings.extend(check.warnings);
        Ok(result)
    }

    /// draft -> validated -> promoted. The logical check always applies; the quality gate applies unless `force`.
    pub fn promote(&self, version: &str, force: bool) -> Result<PromotionResponse> {
        let info = self.registry.get(version)?.ok_or_else(|| ServiceError::NotFound(version.to_string()))?;
        let previous = info.status;
        let refuse = |q: f64, msg: String| PromotionResponse { version: version.into(), previous_status: previous, new_status: previous, quality_score: q, promoted: false, message: msg };
        let target = match previous {
            Status::Promoted => return Ok(refuse(0.0, "Version is already promoted".into())),
            Status::Draft => Status::Validated,
            Status::Validated => Status::Promoted,
            Status::Deprecated => return Ok(refuse(0.0, format!("Cannot promote from status '{}'", previous.as_str()))),
        };
        let o = self.load(version)?;
        let check = consistency::check(&o);
        self.registry.set_consistency(version, check.clone())?;
        if !check.is_consistent {
            return Ok(refuse(0.0, format!("OWL consistency check FAILED: {}. Cannot promote.", check.errors.iter().take(3).cloned().collect::<Vec<_>>().join("; "))));
        }
        let mut quality = 0.0;
        if !force {
            let review = self.critic.validate(&o);
            quality = review.quality_score;
            if !review.is_valid {
                return Ok(refuse(quality, format!("Ontology has validation errors. Cannot promote. Errors: {:?}", review.errors)));
            }
            if quality < QUALITY_THRESHOLD {
                return Ok(refuse(quality, format!("Quality score {quality:.2} below threshold {QUALITY_THRESHOLD}. Cannot promote.")));
            }
        }
        self.registry.set_status(version, target)?;
        let mut message = format!("Successfully promoted from '{}' → '{}'", previous.as_str(), target.as_str());
        message.push_str(&if force { " (forced, validation skipped)".to_string() } else { format!(" (quality score: {quality:.2})") });
        Ok(PromotionResponse { version: version.into(), previous_status: previous, new_status: target, quality_score: quality, promoted: true, message })
    }

    /// Does a document ingested under `document_version` need to be processed again?
    pub fn reingest_check(&self, document_hash: &str, document_version: &str) -> Result<ReIngestCheck> {
        let check = |action, reason: String, affected: bool| ReIngestCheck { action, reason, affected_documents: if affected { vec![document_hash.to_string()] } else { vec![] }, document_ontology_version: Some(document_version.to_string()) };
        let Some(doc) = self.registry.get(document_version)? else {
            return Ok(check(ReIngestAction::FullReingest, format!("Document's ontology version '{document_version}' not found in registry"), false));
        };
        let latest = match self.registry.latest(Some(Status::Promoted))? {
            Some(l) => l,
            None => match self.registry.latest(None)? {
                Some(l) => l,
                None => return Ok(check(ReIngestAction::NoAction, "No ontology versions available in registry".into(), false)),
            },
        };
        if doc.version == latest.version {
            return Ok(check(ReIngestAction::NoAction, format!("Document already using latest promoted ontology ({})", latest.version), false));
        }
        let (Some(a), Some(b)) = (self.registry.ontology(&doc.version)?, self.registry.ontology(&latest.version)?) else {
            return Ok(check(ReIngestAction::FullReingest, "Unable to load ontology content for comparison".into(), false));
        };
        let names = |o: &Ontology| -> (BTreeSet<String>, BTreeSet<String>) {
            let (classes, triples) = parts(o);
            (classes, triples.into_iter().map(|(_, p, _)| p).collect())
        };
        let ((dc, dr), (lc, lr)) = (names(&a), names(&b));
        let (cr, rr): (Vec<_>, Vec<_>) = (dc.difference(&lc).collect(), dr.difference(&lr).collect());
        let (ca, ra): (Vec<_>, Vec<_>) = (lc.difference(&dc).collect(), lr.difference(&dr).collect());
        Ok(if !cr.is_empty() || !rr.is_empty() {
            check(ReIngestAction::FullReingest, format!("Ontology has removals: {} class(es), {} relation(s) removed. Full re-ingestion required.", cr.len(), rr.len()), true)
        } else if !ca.is_empty() || !ra.is_empty() {
            check(ReIngestAction::PartialReingest, format!("Ontology has additions: {} new class(es), {} new relation(s). Partial re-ingestion recommended.", ca.len(), ra.len()), true)
        } else {
            check(ReIngestAction::NoAction, "No significant ontology changes detected between versions".into(), false)
        })
    }

    /// Write a promoted version to `path`: OWL functional syntax for `.ofn`, RDF/XML otherwise.
    pub fn export(&self, version: &str, path: &std::path::Path) -> Result<ExportInfo> {
        let info = self.registry.get(version)?.ok_or_else(|| ServiceError::NotFound(version.to_string()))?;
        if info.status != Status::Promoted {
            return Err(ServiceError::NotPromoted(info.status.as_str()));
        }
        let o = self.load(version)?;
        let text = if path.extension().is_some_and(|e| e == "ofn") { write(&o) } else { xiar_owl::rdfxml::export(&o).map_err(|e| ServiceError::Export(e.to_string()))? };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| ServiceError::Export(e.to_string()))?;
        }
        std::fs::write(path, &text).map_err(|e| ServiceError::Export(e.to_string()))?;
        let (classes, triples) = parts(&o);
        Ok(ExportInfo { version: version.into(), path: path.to_path_buf(), bytes: text.len(), triples: classes.len() + triples.len() })
    }

    /// Terms related to a query: classes whose name contains (or is contained in) a query word, with their direct
    /// subclasses and superclasses, at most twenty, sorted. Without a promoted ontology there is nothing to add.
    pub fn expand_query(&self, query: &str) -> Result<Expansion> {
        let Some(latest) = self.registry.latest(Some(Status::Promoted))? else { return Ok(Expansion { ontology_version: None, expanded_terms: vec![], original_query: query.into() }) };
        let o = self.load(&latest.version)?;
        let supers = hierarchy(&o);
        let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).filter(|t| t.chars().count() > 2).collect();
        let mut out = BTreeSet::new();
        for class in supers.keys() {
            let lower = class.to_lowercase();
            if terms.iter().any(|t| lower.contains(t.as_str()) || t.contains(lower.as_str())) {
                out.insert(class.clone());
                out.extend(supers[class].iter().cloned());
                out.extend(supers.iter().filter(|(_, s)| s.contains(class)).map(|(c, _)| c.clone()));
            }
        }
        Ok(Expansion { ontology_version: Some(latest.version), expanded_terms: out.into_iter().take(MAX_EXPANDED_TERMS).collect(), original_query: query.into() })
    }

    /// Names of the classes in the promoted ontology, sorted; empty when nothing is promoted.
    pub fn class_names(&self) -> Result<Vec<String>> {
        let Some(latest) = self.registry.latest(Some(Status::Promoted))? else { return Ok(vec![]) };
        let o = self.load(&latest.version)?;
        Ok(o.class_names().iter().map(|c| c.local().to_string()).collect::<BTreeSet<_>>().into_iter().collect())
    }

    /// Classes whose name matches a word of `query` (same rule as [`Self::expand_query`]) together with every class below
    /// them, however deep. Asking for "vehicle" reaches things typed `SportsCar`; asking for "car" does not reach "vehicle".
    pub fn classes_under(&self, query: &str) -> Result<BTreeSet<String>> {
        let Some(latest) = self.registry.latest(Some(Status::Promoted))? else { return Ok(BTreeSet::new()) };
        let supers = hierarchy(&self.load(&latest.version)?);
        let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).filter(|t| t.chars().count() > 2).collect();
        let mut out: BTreeSet<String> = supers.keys().filter(|c| { let l = c.to_lowercase(); terms.iter().any(|t| l.contains(t.as_str()) || t.contains(l.as_str())) }).cloned().collect();
        loop {
            let below: Vec<String> = supers.iter().filter(|(c, s)| !out.contains(*c) && s.iter().any(|x| out.contains(x))).map(|(c, _)| c.clone()).collect();
            if below.is_empty() {
                return Ok(out);
            }
            out.extend(below);
        }
    }

    /// The classes each entity belongs to under the promoted ontology. `entities` pairs a name with the class names it was
    /// asserted to be (unknown class names are ignored). The result lists every named class the reasoner proves for it,
    /// superclasses included, sorted. An entity set that contradicts the ontology is an error, never a silent guess.
    pub fn entity_types(&self, entities: &[(String, Vec<String>)]) -> Result<BTreeMap<String, Vec<String>>> {
        let latest = self.registry.latest(Some(Status::Promoted))?.ok_or(ServiceError::NothingPromoted)?;
        let mut o = self.load(&latest.version)?;
        let by_local: BTreeMap<String, xiar_owl::model::Iri> = o.class_names().into_iter().map(|c| (c.local().to_string(), c)).collect();
        let iri = |name: &str| xiar_owl::model::Iri::new(format!("http://xiar.ai/individual#{}", name.split_whitespace().collect::<Vec<_>>().join("_")));
        for (name, asserted) in entities {
            o.axioms.push(Axiom::DeclareIndividual(iri(name)));
            for c in asserted.iter().filter_map(|c| by_local.get(c)) {
                o.axioms.push(Axiom::ClassAssertion(ClassExpr::Class(c.clone()), iri(name)));
            }
        }
        let reasoner = Reasoner::new(&o);
        let mut out = BTreeMap::new();
        for (name, _) in entities {
            let types = reasoner.types(&iri(name)).map_err(|e| ServiceError::Rejected(e.to_string()))?;
            out.insert(name.clone(), types.iter().map(|c| c.local().to_string()).collect());
        }
        Ok(out)
    }

    /// What the promoted ontology says about each entity: the properties it takes part in and its place in the hierarchy.
    pub fn query_relations(&self, entities: &[String]) -> Result<RelationsResult> {
        let latest = self.registry.latest(Some(Status::Promoted))?.ok_or(ServiceError::NothingPromoted)?;
        let o = self.load(&latest.version)?;
        let (_, triples) = parts(&o);
        let classes: BTreeSet<String> = o.class_names().iter().map(|c| c.local().to_string()).collect();
        let supers = hierarchy(&o);
        let show = |s: &str| if s.is_empty() { "?".to_string() } else { s.to_string() };
        let mut relations = BTreeMap::new();
        for e in entities {
            let needle = e.to_lowercase();
            let mut found: Vec<RelationRef> = Vec::new();
            if !needle.is_empty() {
                for c in classes.iter().filter(|c| c.to_lowercase().contains(&needle)) {
                    for (d, p, r) in &triples {
                        if d == c {
                            found.push(RelationRef { kind: "property_from", context: format!("{p}: {c} -> {}", show(r)) });
                        }
                        if r == c {
                            found.push(RelationRef { kind: "property_to", context: format!("{p}: {} -> {c}", show(d)) });
                        }
                    }
                    for s in supers.get(c).into_iter().flatten() {
                        found.push(RelationRef { kind: "subclass_of", context: format!("{c} is a {s}") });
                    }
                    for (sub, s) in &supers {
                        if s.contains(c) {
                            found.push(RelationRef { kind: "superclass_of", context: format!("{c} is a kind of ... {sub} is a {c}") });
                        }
                    }
                }
                for (d, p, r) in triples.iter().filter(|(_, p, _)| p.to_lowercase().contains(&needle)) {
                    found.push(RelationRef { kind: "property", context: format!("{p}: {} -> {}", show(d), show(r)) });
                }
            }
            found.dedup();
            found.truncate(MAX_RELATIONS_PER_ENTITY);
            relations.insert(e.clone(), found);
        }
        let total = relations.values().map(Vec::len).sum();
        Ok(RelationsResult { ontology_version: latest.version, relations, entity_count: entities.len(), total_relations_found: total })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiar_owl::functional::parse;
    use xiar_prompts::PromptStore;
    use xiar_store::relational::RedbStore;

    fn service() -> OntologyService {
        let reg = Arc::new(Registry::new(Arc::new(RedbStore::in_memory().unwrap())));
        OntologyService::new(reg, Critic::new(None, Arc::new(PromptStore::new(None, crate::BUNDLED))))
    }

    fn class(n: &str) -> CandidateClass {
        CandidateClass { name: n.into(), description: String::new(), source_document: "d".into(), confidence: 0.9, occurrences: 1 }
    }

    fn rel(s: &str, p: &str, o: &str) -> CandidateRelation {
        CandidateRelation { subject_class: s.into(), predicate: p.into(), object_class: o.into(), description: String::new(), source_document: "d".into(), confidence: 0.9, occurrences: 1 }
    }

    const BASE: &str = "Prefix(x:=<http://xiar.ai/ontology#>)\nOntology(<http://xiar.ai/ontology> Declaration(Class(x:Agent)) Declaration(Class(x:Task)) Declaration(Class(x:Tool)) Declaration(Class(x:Plan)) Declaration(Class(x:Goal)) Declaration(ObjectProperty(x:uses)) ObjectPropertyDomain(x:uses x:Agent) ObjectPropertyRange(x:uses x:Tool) Declaration(ObjectProperty(x:has)) ObjectPropertyDomain(x:has x:Agent) ObjectPropertyRange(x:has x:Goal) Declaration(ObjectProperty(x:serves)) ObjectPropertyDomain(x:serves x:Plan) ObjectPropertyRange(x:serves x:Goal))";

    #[test]
    fn the_first_build_starts_from_an_empty_base_made_on_the_spot() {
        let s = service();
        assert!(s.engineer(&OntologyCandidate::default(), None).is_err(), "with no base there is nothing to build on");
        assert!(s.ensure_base().unwrap(), "it made one");
        assert!(!s.ensure_base().unwrap(), "and only once");
        let candidates = OntologyCandidate { classes: vec![class("Vendor"), class("Project")], relations: vec![rel("Project", "usesVendor", "Vendor")], ..OntologyCandidate::default() };
        let made = s.engineer(&candidates, None).unwrap();
        assert_eq!(made.classes_added, 2);
        assert_eq!(s.registry.get(&made.new_version).unwrap().unwrap().status, Status::Draft);
    }

    fn seeded() -> OntologyService {
        let s = service();
        s.registry.register_text(BASE, "system", false, None, Status::Draft).unwrap();
        s
    }

    #[test]
    fn engineering_registers_a_draft_with_its_consistency_proof() {
        let s = seeded();
        let cands = OntologyCandidate { classes: vec![class("customer"), class("tools")], relations: vec![rel("customer", "owns", "task")], ..Default::default() };
        let r = s.engineer(&cands, None).unwrap();
        assert_eq!((r.new_version.as_str(), r.classes_added, r.relations_added, r.conflicts_resolved), ("v2", 1, 1, 1));
        let v = s.registry.get("v2").unwrap().unwrap();
        assert_eq!((v.status, v.promoted_by.as_str()), (Status::Draft, "engineer_agent"));
        assert!(s.registry.consistency("v2").unwrap().unwrap().is_consistent);
        assert!(matches!(service().engineer(&cands, None), Err(ServiceError::NoBase)));
        assert!(matches!(s.engineer(&cands, Some("v9")), Err(ServiceError::NotFound(_))));
    }

    #[test]
    fn an_ontology_that_fails_the_logical_check_is_rejected_when_engineered() {
        let s = service();
        s.registry.register_text("Prefix(x:=<http://xiar.ai/ontology#>)\nOntology(<http://xiar.ai/ontology> Declaration(Class(x:A)) DisjointClasses(x:A x:B) SubClassOf(x:C x:A) SubClassOf(x:C x:B))", "m", false, None, Status::Draft).unwrap();
        let e = s.engineer(&OntologyCandidate::default(), None).unwrap_err();
        assert!(matches!(&e, ServiceError::Rejected(m) if m.contains("Unsatisfiable class: C")), "{e}");
        assert_eq!(s.registry.list(10).unwrap().len(), 1, "nothing was registered");
    }

    #[test]
    fn promotion_walks_draft_validated_promoted_through_the_quality_gate() {
        let s = seeded();
        let r = s.promote("v1", false).unwrap();
        assert!(r.promoted && (r.previous_status, r.new_status) == (Status::Draft, Status::Validated), "{r:?}");
        assert!(r.message.starts_with("Successfully promoted from 'draft' → 'validated' (quality score: 1.00)"));
        let r = s.promote("v1", false).unwrap();
        assert_eq!(r.new_status, Status::Promoted);
        let r = s.promote("v1", false).unwrap();
        assert!(!r.promoted && r.message == "Version is already promoted");
        assert!(matches!(s.promote("v7", false), Err(ServiceError::NotFound(_))));
    }

    #[test]
    fn a_low_score_blocks_promotion_unless_forced_but_inconsistency_always_does() {
        let s = service();
        // a class with a bad name is a structural error
        s.registry.register_text("Prefix(x:=<http://xiar.ai/ontology#>)\nOntology(<http://xiar.ai/ontology> Declaration(Class(x:lowercase)))", "m", false, None, Status::Draft).unwrap();
        let r = s.promote("v1", false).unwrap();
        assert!(!r.promoted && r.message.contains("validation errors"), "{r:?}");
        let r = s.promote("v1", true).unwrap();
        assert!(r.promoted && r.message.ends_with("(forced, validation skipped)"));
        s.registry.register_text("Prefix(x:=<http://xiar.ai/ontology#>)\nOntology(<http://xiar.ai/ontology> DisjointClasses(x:A x:B) SubClassOf(x:C x:A) SubClassOf(x:C x:B) Declaration(Class(x:A)))", "m", false, None, Status::Draft).unwrap();
        let r = s.promote("v2", true).unwrap();
        assert!(!r.promoted && r.message.starts_with("OWL consistency check FAILED"), "even a forced promotion needs a consistent ontology: {r:?}");
        assert_eq!(s.registry.get("v2").unwrap().unwrap().status, Status::Draft);
        let warn_only = {
            let c = "Declaration(Class(x:A)) Declaration(ObjectProperty(x:p)) ObjectPropertyDomain(x:p x:A) ObjectPropertyRange(x:p x:A) Declaration(Class(x:B)) Declaration(ObjectProperty(x:q)) ObjectPropertyDomain(x:q x:B) ObjectPropertyRange(x:q x:B)";
            format!("Prefix(x:=<http://xiar.ai/ontology#>)\nOntology(<http://xiar.ai/ontology> {c})")
        };
        s.registry.register_text(&warn_only, "m", false, None, Status::Draft).unwrap();
        let r = s.promote("v3", false).unwrap();
        assert!(!r.promoted && r.message.contains("below threshold"), "two self-loops are two warnings: {r:?}");
        assert!((r.quality_score - 0.9).abs() < 1e-9);
    }

    #[test]
    fn validation_reports_and_stores_the_logical_check() {
        let s = seeded();
        let v = s.validate("v1").unwrap();
        assert!(v.is_valid && v.quality_score == 1.0);
        assert!(s.registry.consistency("v1").unwrap().unwrap().is_consistent);
        assert!(matches!(s.validate("v5"), Err(ServiceError::NotFound(_))));
    }

    #[test]
    fn re_ingestion_follows_what_changed_between_versions() {
        let s = seeded();
        assert_eq!(s.reingest_check("h", "v9").unwrap().action, ReIngestAction::FullReingest);
        assert_eq!(s.reingest_check("h", "v1").unwrap().action, ReIngestAction::NoAction, "the only version is the latest");
        s.engineer(&OntologyCandidate { classes: vec![class("customer")], ..Default::default() }, None).unwrap();
        s.promote("v2", false).unwrap();
        s.promote("v2", false).unwrap();
        let r = s.reingest_check("hash-1", "v1").unwrap();
        assert_eq!(r.action, ReIngestAction::PartialReingest);
        assert!(r.reason.contains("1 new class(es), 0 new relation(s)") && r.affected_documents == ["hash-1"], "{r:?}");
        assert_eq!(s.reingest_check("h", "v2").unwrap().action, ReIngestAction::NoAction);
        let removed = parse(BASE.replace("Declaration(Class(x:Tool))", "").as_str()).unwrap();
        s.registry.register(&removed, "m", false, None, Status::Draft, None).unwrap();
        s.promote("v3", true).unwrap();
        s.promote("v3", true).unwrap();
        assert_eq!(s.reingest_check("h", "v1").unwrap().action, ReIngestAction::FullReingest, "Tool disappeared");
    }

    #[test]
    fn only_promoted_versions_export_and_the_format_follows_the_extension() {
        let s = seeded();
        let dir = std::env::temp_dir().join(format!("xiar-onto-export-{}", std::process::id()));
        assert!(matches!(s.export("v1", &dir.join("o.owl")), Err(ServiceError::NotPromoted("draft"))));
        s.promote("v1", false).unwrap();
        s.promote("v1", false).unwrap();
        let rdf = s.export("v1", &dir.join("sub").join("o.owl")).unwrap();
        assert!(std::fs::read_to_string(&rdf.path).unwrap().contains("<owl:Class rdf:about=\"http://xiar.ai/ontology#Agent\""));
        assert_eq!(rdf.triples, 5 + 3);
        let ofn = s.export("v1", &dir.join("o.ofn")).unwrap();
        assert_eq!(parse(&std::fs::read_to_string(&ofn.path).unwrap()).unwrap(), s.registry.ontology("v1").unwrap().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_expansion_uses_the_promoted_ontology_and_its_hierarchy() {
        let s = service();
        assert_eq!(s.expand_query("anything").unwrap().ontology_version, None);
        assert!(matches!(s.query_relations(&["x".into()]), Err(ServiceError::NothingPromoted)));
        let text = "Prefix(x:=<http://xiar.ai/ontology#>)\nOntology(<http://xiar.ai/ontology> Declaration(Class(x:Vehicle)) SubClassOf(x:Car x:Vehicle) SubClassOf(x:SportsCar x:Car) Declaration(Class(x:Driver)) Declaration(ObjectProperty(x:drives)) ObjectPropertyDomain(x:drives x:Driver) ObjectPropertyRange(x:drives x:Car))";
        s.registry.register_text(text, "m", false, None, Status::Draft).unwrap();
        s.promote("v1", true).unwrap();
        s.promote("v1", true).unwrap();
        let e = s.expand_query("tell me about my car please").unwrap();
        assert_eq!(e.ontology_version.as_deref(), Some("v1"));
        assert_eq!(e.expanded_terms, ["Car", "SportsCar", "Vehicle"], "the class, its subclass and its superclass");
        assert!(s.expand_query("zebra").unwrap().expanded_terms.is_empty());
        assert!(s.expand_query("is a").unwrap().expanded_terms.is_empty(), "words of two letters are ignored");
        let r = s.query_relations(&["car".into(), "drives".into(), "nothing".into()]).unwrap();
        assert_eq!((r.entity_count, r.ontology_version.as_str()), (3, "v1"));
        let car: Vec<&str> = r.relations["car"].iter().map(|x| x.context.as_str()).collect();
        assert!(car.contains(&"drives: Driver -> Car") && car.contains(&"Car is a Vehicle"), "{car:?}");
        assert_eq!(r.relations["drives"][0].kind, "property");
        assert!(r.relations["nothing"].is_empty());
        assert_eq!(r.total_relations_found, r.relations.values().map(Vec::len).sum::<usize>());
    }

    #[test]
    fn entity_types_include_inferred_superclasses_and_refuse_contradictions() {
        let s = service();
        assert!(matches!(s.entity_types(&[]), Err(ServiceError::NothingPromoted)));
        let text = "Prefix(x:=<http://xiar.ai/ontology#>)
Ontology(<http://xiar.ai/ontology> Declaration(Class(x:Vehicle)) SubClassOf(x:Car x:Vehicle) SubClassOf(x:SportsCar x:Car) Declaration(Class(x:Driver)) DisjointClasses(x:Driver x:Vehicle))";
        s.registry.register_text(text, "m", false, None, Status::Draft).unwrap();
        s.promote("v1", true).unwrap();
        s.promote("v1", true).unwrap();
        let t = s.entity_types(&[("my red car".into(), vec!["SportsCar".into(), "Unknown".into()]), ("Ann".into(), vec!["Driver".into()])]).unwrap();
        assert_eq!(t["my red car"], ["Car", "SportsCar", "Vehicle"]);
        assert_eq!(t["Ann"], ["Driver"]);
        let bad = s.entity_types(&[("Ann".into(), vec!["Driver".into(), "Car".into()])]);
        assert!(matches!(bad, Err(ServiceError::Rejected(_))), "{bad:?}");
        assert_eq!(s.class_names().unwrap(), ["Car", "Driver", "SportsCar", "Vehicle"]);
        assert_eq!(s.classes_under("vehicle").unwrap().into_iter().collect::<Vec<_>>(), ["Car", "SportsCar", "Vehicle"], "everything below, at any depth");
        assert_eq!(s.classes_under("my car").unwrap().into_iter().collect::<Vec<_>>(), ["Car", "SportsCar"], "not its superclass");
        assert!(s.classes_under("zebra").unwrap().is_empty());
    }
}
