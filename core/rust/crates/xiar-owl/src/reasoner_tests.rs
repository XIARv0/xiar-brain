// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
use crate::functional::parse;
use crate::model::*;
use crate::reasoner::*;
use std::collections::BTreeSet;

fn kb(axioms: &str) -> Reasoner {
    let doc = format!("Prefix(ex:=<http://e/#>)\nOntology(<http://e>\n{axioms}\n)");
    Reasoner::new(&parse(&doc).unwrap_or_else(|e| panic!("{e}\n{doc}")))
}

fn c(n: &str) -> ClassExpr {
    ClassExpr::class(&format!("http://e/#{n}"))
}

fn i(n: &str) -> Iri {
    Iri::new(format!("http://e/#{n}"))
}

fn names(s: &BTreeSet<Iri>) -> Vec<&str> {
    s.iter().map(|x| x.local()).collect()
}

#[test]
fn told_hierarchy_is_classified_with_direct_supers() {
    let r = kb("SubClassOf(ex:A ex:B) SubClassOf(ex:B ex:C) SubClassOf(ex:D ex:C) Declaration(Class(ex:E))");
    let k = r.classify().unwrap();
    assert!(k.unsatisfiable.is_empty());
    assert_eq!(names(&k.all_supers[&i("A")]), ["B", "C"]);
    assert_eq!(names(&k.direct_supers[&i("A")]), ["B"]);
    assert_eq!(names(&k.direct_supers[&i("D")]), ["C"]);
    assert!(k.direct_supers[&i("E")].is_empty() && k.direct_supers[&i("C")].is_empty());
}

#[test]
fn unsatisfiable_classes_are_found_including_by_disjointness() {
    let r = kb("SubClassOf(ex:A ex:B) SubClassOf(ex:A ObjectComplementOf(ex:B)) DisjointClasses(ex:P ex:Q) SubClassOf(ex:X ex:P) SubClassOf(ex:X ex:Q) SubClassOf(ex:Y ex:X) Declaration(Class(ex:Fine))");
    let k = r.classify().unwrap();
    assert_eq!(names(&k.unsatisfiable), ["A", "X", "Y"]);
    assert!(!k.all_supers.contains_key(&i("A")), "unsatisfiable classes are reported separately");
    assert!(r.is_satisfiable(&c("Fine")).unwrap());
    assert!(!r.is_satisfiable(&ClassExpr::And(vec![c("P"), c("Q")])).unwrap());
}

#[test]
fn definitions_classify_by_their_meaning() {
    let r = kb("EquivalentClasses(ex:Parent ObjectSomeValuesFrom(ex:hasChild ex:Person))
                SubClassOf(ex:Mother ObjectIntersectionOf(ex:Woman ObjectSomeValuesFrom(ex:hasChild ex:Person)))
                SubClassOf(ex:Woman ex:Person)
                SubClassOf(ex:Father ObjectSomeValuesFrom(ex:hasChild ex:Person))");
    let k = r.classify().unwrap();
    assert!(k.all_supers[&i("Mother")].contains(&i("Parent")) && k.all_supers[&i("Father")].contains(&i("Parent")));
    assert!(!k.all_supers[&i("Parent")].contains(&i("Mother")));
    assert_eq!(names(&k.direct_supers[&i("Mother")]), ["Parent", "Woman"]);
}

#[test]
fn domain_and_range_drive_inference() {
    let r = kb("ObjectPropertyDomain(ex:hasChild ex:Parent) ObjectPropertyRange(ex:hasChild ex:Person)
                EquivalentClasses(ex:Owner ObjectSomeValuesFrom(ex:hasChild owl:Thing)) SubClassOf(ex:Parent ex:Person)");
    let k = r.classify().unwrap();
    assert!(k.all_supers[&i("Owner")].contains(&i("Parent")), "exists hasChild.Thing is within Parent by the domain");
    assert!(r.subsumes(&c("Person"), &c("Owner")).unwrap());
    assert!(r.subsumes(&ClassExpr::All(ObjectProp::Named(i("hasChild")), Box::new(c("Person"))), &ClassExpr::Thing).unwrap(), "the range makes all-hasChild-Person universal");
}

#[test]
fn universal_and_existential_restrictions_interact() {
    let r = kb("DisjointClasses(ex:B ex:C) SubClassOf(ex:A ObjectAllValuesFrom(ex:r ex:B)) SubClassOf(ex:A ObjectSomeValuesFrom(ex:r ex:C)) SubClassOf(ex:Ok ObjectSomeValuesFrom(ex:r ex:B))");
    assert!(!r.is_satisfiable(&c("A")).unwrap());
    assert!(r.is_satisfiable(&c("Ok")).unwrap());
}

#[test]
fn number_restrictions_count_and_merge() {
    let r = kb("DisjointClasses(ex:B ex:C)
                SubClassOf(ex:Many ObjectMinCardinality(3 ex:r)) SubClassOf(ex:Many ObjectMaxCardinality(2 ex:r))
                SubClassOf(ex:Tight ObjectMaxCardinality(1 ex:r)) SubClassOf(ex:Tight ObjectSomeValuesFrom(ex:r ex:B)) SubClassOf(ex:Tight ObjectSomeValuesFrom(ex:r ex:C))
                SubClassOf(ex:Merge ObjectMaxCardinality(1 ex:r)) SubClassOf(ex:Merge ObjectSomeValuesFrom(ex:r ex:B)) SubClassOf(ex:Merge ObjectSomeValuesFrom(ex:r ex:D))
                SubClassOf(ex:Exact ObjectExactCardinality(2 ex:r ex:B)) SubClassOf(ex:Exact ObjectSomeValuesFrom(ex:r ex:C))
                SubClassOf(ex:Zero ObjectMaxCardinality(0 ex:r)) SubClassOf(ex:Zero ObjectSomeValuesFrom(ex:r owl:Thing))");
    assert!(!r.is_satisfiable(&c("Many")).unwrap(), "at least 3 and at most 2");
    assert!(!r.is_satisfiable(&c("Tight")).unwrap(), "one filler cannot be both B and C");
    assert!(r.is_satisfiable(&c("Merge")).unwrap(), "B and D may be the same filler");
    assert!(r.is_satisfiable(&c("Exact")).unwrap(), "exactly 2 B fillers, and a C filler that is not a B");
    assert!(!r.is_satisfiable(&c("Zero")).unwrap());
}

#[test]
fn inverse_roles_propagate_backwards() {
    let r = kb("SubClassOf(ex:A ObjectSomeValuesFrom(ex:r ex:B)) SubClassOf(ex:B ObjectAllValuesFrom(ObjectInverseOf(ex:r) ex:C)) InverseObjectProperties(ex:r ex:s) SubClassOf(ex:D ObjectSomeValuesFrom(ex:s ex:E)) SubClassOf(ex:E ObjectAllValuesFrom(ex:r ex:F))");
    assert!(r.subsumes(&c("C"), &c("A")).unwrap(), "A has an r-filler B whose r-predecessors are C, and A is one");
    assert!(r.subsumes(&c("F"), &c("D")).unwrap(), "D's s-filler E has D as an r-filler, so D is F");
    assert!(!r.subsumes(&c("C"), &c("D")).unwrap());
}

#[test]
fn transitive_and_hierarchical_roles() {
    let r = kb("TransitiveObjectProperty(ex:anc) SubObjectPropertyOf(ex:parent ex:anc)
                SubClassOf(ex:T ObjectSomeValuesFrom(ex:parent ObjectSomeValuesFrom(ex:parent ex:Old)))
                SubClassOf(ex:T ObjectAllValuesFrom(ex:anc ObjectComplementOf(ex:Old)))
                SubClassOf(ex:H ObjectSomeValuesFrom(ex:parent ex:Old)) SubClassOf(ex:H ObjectAllValuesFrom(ex:anc ObjectComplementOf(ex:Old)))
                SubClassOf(ex:NoTrans ObjectSomeValuesFrom(ex:link ObjectSomeValuesFrom(ex:link ex:Old))) SubClassOf(ex:NoTrans ObjectAllValuesFrom(ex:link ObjectComplementOf(ex:Old)))");
    assert!(!r.is_satisfiable(&c("T")).unwrap(), "transitivity carries the universal restriction two steps down");
    assert!(!r.is_satisfiable(&c("H")).unwrap(), "parent is a sub-role of anc");
    assert!(r.is_satisfiable(&c("NoTrans")).unwrap(), "without transitivity the second step is unconstrained");
}

#[test]
fn functional_and_inverse_functional_roles() {
    let r = kb("DisjointClasses(ex:B ex:C) FunctionalObjectProperty(ex:f) InverseFunctionalObjectProperty(ex:g)
                SubClassOf(ex:F ObjectSomeValuesFrom(ex:f ex:B)) SubClassOf(ex:F ObjectSomeValuesFrom(ex:f ex:C))
                SubClassOf(ex:G ObjectSomeValuesFrom(ObjectInverseOf(ex:g) ex:B)) SubClassOf(ex:G ObjectSomeValuesFrom(ObjectInverseOf(ex:g) ex:C))");
    assert!(!r.is_satisfiable(&c("F")).unwrap());
    assert!(!r.is_satisfiable(&c("G")).unwrap());
}

#[test]
fn equivalent_classes_are_grouped_and_reduced() {
    let r = kb("EquivalentClasses(ex:A ex:B) SubClassOf(ex:B ex:C) SubClassOf(ex:D ex:A)");
    let k = r.classify().unwrap();
    assert_eq!(k.equivalents.len(), 1);
    assert_eq!(names(&k.equivalents[0]), ["A", "B"]);
    assert_eq!(names(&k.direct_supers[&i("D")]), ["A"], "equivalent classes appear once, as their first member");
    assert_eq!(names(&k.direct_supers[&i("A")]), ["C"]);
}

#[test]
fn cyclic_definitions_terminate_by_blocking() {
    let r = kb("SubClassOf(ex:Person ObjectSomeValuesFrom(ex:hasParent ex:Person))
                SubClassOf(ex:N ObjectSomeValuesFrom(ex:next ex:N)) SubClassOf(ex:N ObjectMaxCardinality(1 ObjectInverseOf(ex:next))) SubClassOf(ex:N ObjectMinCardinality(1 ObjectInverseOf(ex:next)))
                TransitiveObjectProperty(ex:anc) SubClassOf(ex:Q ObjectSomeValuesFrom(ex:anc ex:Q))");
    assert!(r.is_satisfiable(&c("Person")).unwrap());
    assert!(r.is_satisfiable(&c("N")).unwrap(), "an infinite chain is a model; pairwise blocking finds a finite one");
    assert!(r.is_satisfiable(&c("Q")).unwrap());
    assert!(r.is_consistent().unwrap());
}

#[test]
fn disjunctions_backtrack() {
    let r = kb("SubClassOf(ex:A ObjectUnionOf(ex:B ex:C)) SubClassOf(ex:B ex:X) SubClassOf(ex:C ex:X) SubClassOf(ex:D ObjectUnionOf(ex:E ex:F)) DisjointClasses(ex:E ex:G) DisjointClasses(ex:F ex:G) SubClassOf(ex:H ex:D) SubClassOf(ex:H ex:G)");
    assert!(r.subsumes(&c("X"), &c("A")).unwrap(), "each branch ends in X");
    assert!(!r.subsumes(&c("B"), &c("A")).unwrap());
    assert!(!r.is_satisfiable(&c("H")).unwrap(), "both branches of D clash with G");
    let k = r.classify().unwrap();
    assert!(k.all_supers[&i("A")].contains(&i("X")));
    assert!(!k.all_supers[&i("A")].contains(&i("B")));
}

#[test]
fn abox_consistency_and_instance_inference() {
    let r = kb("ObjectPropertyDomain(ex:hasChild ex:Parent) ObjectPropertyRange(ex:hasChild ex:Person) SubClassOf(ex:Parent ex:Person)
                ClassAssertion(ex:Woman ex:alice) SubClassOf(ex:Woman ex:Person) ObjectPropertyAssertion(ex:hasChild ex:alice ex:bob)
                EquivalentClasses(ex:Mother ObjectIntersectionOf(ex:Woman ex:Parent))");
    assert!(r.is_consistent().unwrap());
    assert!(r.instance_of(&i("alice"), &c("Parent")).unwrap(), "domain");
    assert!(r.instance_of(&i("alice"), &c("Mother")).unwrap(), "definition");
    assert!(r.instance_of(&i("bob"), &c("Person")).unwrap(), "range");
    assert!(!r.instance_of(&i("bob"), &c("Parent")).unwrap());
    assert_eq!(names(&r.types(&i("alice")).unwrap()), ["Mother", "Parent", "Person", "Woman"]);
}

#[test]
fn abox_inconsistencies_are_detected() {
    assert!(!kb("DisjointClasses(ex:A ex:B) ClassAssertion(ex:A ex:x) ClassAssertion(ex:B ex:x)").is_consistent().unwrap());
    assert!(!kb("SubClassOf(ex:A owl:Nothing) ClassAssertion(ex:A ex:x)").is_consistent().unwrap());
    assert!(!kb("SameIndividual(ex:x ex:y) DifferentIndividuals(ex:x ex:y)").is_consistent().unwrap());
    assert!(!kb("ClassAssertion(ObjectMaxCardinality(1 ex:r) ex:x) ObjectPropertyAssertion(ex:r ex:x ex:a) ObjectPropertyAssertion(ex:r ex:x ex:b) DifferentIndividuals(ex:a ex:b)").is_consistent().unwrap());
    assert!(kb("ClassAssertion(ObjectMaxCardinality(1 ex:r) ex:x) ObjectPropertyAssertion(ex:r ex:x ex:a) ObjectPropertyAssertion(ex:r ex:x ex:b)").is_consistent().unwrap(), "no unique-name assumption: a and b are merged");
    let r = kb("FunctionalObjectProperty(ex:r) ObjectPropertyAssertion(ex:r ex:x ex:a) ObjectPropertyAssertion(ex:r ex:x ex:b) ClassAssertion(ex:A ex:a) ClassAssertion(ex:B ex:b) DisjointClasses(ex:A ex:B)");
    assert!(!r.is_consistent().unwrap(), "a functional role forces a = b, but they are in disjoint classes");
    assert_eq!(r.classify().unwrap_err(), ReasonError::Inconsistent);
    assert_eq!(r.instance_of(&i("x"), &c("A")).unwrap_err(), ReasonError::Inconsistent);
}

#[test]
fn same_individual_merges_their_knowledge() {
    let r = kb("SameIndividual(ex:a ex:b) ClassAssertion(ex:A ex:a) ClassAssertion(ex:B ex:b) SubClassOf(ex:A ex:Top1)");
    assert!(r.instance_of(&i("b"), &c("A")).unwrap());
    assert!(r.instance_of(&i("a"), &c("B")).unwrap());
    assert!(r.instance_of(&i("b"), &c("Top1")).unwrap());
}

#[test]
fn nnf_and_negation_are_exact() {
    let a = c("A");
    let r = ObjectProp::Named(i("r"));
    assert_eq!(neg(&neg(&a)), a);
    assert_eq!(neg(&ClassExpr::Some(r.clone(), Box::new(a.clone()))), ClassExpr::All(r.clone(), Box::new(ClassExpr::Not(Box::new(a.clone())))));
    assert_eq!(neg(&ClassExpr::Min(2, r.clone(), Box::new(a.clone()))), ClassExpr::Max(1, r.clone(), Box::new(a.clone())));
    assert_eq!(neg(&ClassExpr::Max(1, r.clone(), Box::new(a.clone()))), ClassExpr::Min(2, r.clone(), Box::new(a.clone())));
    assert_eq!(neg(&ClassExpr::Min(0, r.clone(), Box::new(a.clone()))), ClassExpr::Nothing);
    assert_eq!(nnf(&ClassExpr::Min(0, r, Box::new(a))), ClassExpr::Thing);
}

#[test]
fn an_ontology_without_individuals_is_inconsistent_when_owl_thing_is_unsatisfiable() {
    let r = kb("SubClassOf(owl:Thing owl:Nothing)");
    assert!(!r.is_consistent().unwrap(), "the domain is non-empty, so a contradictory TBox has no model");
    assert_eq!(r.classify().unwrap_err(), ReasonError::Inconsistent);
    let r = kb("SubClassOf(ex:A owl:Nothing)");
    assert!(r.is_consistent().unwrap(), "an unsatisfiable class alone does not make the ontology inconsistent");
    assert!(kb("Declaration(Class(ex:A))").is_consistent().unwrap());
}

#[test]
fn transitive_roles_may_not_appear_in_cardinality_restrictions_or_functional_axioms() {
    let doc = |a: &str| format!("Prefix(ex:=<http://e/#>)\nOntology(<http://e>\n{a}\n)");
    let bad = |a: &str| matches!(parse(&doc(a)), Err(crate::functional::ParseError::NotDl(_)));
    assert!(bad("TransitiveObjectProperty(ex:r) SubClassOf(ex:A ObjectMaxCardinality(1 ex:r))"));
    assert!(bad("TransitiveObjectProperty(ex:r) FunctionalObjectProperty(ex:r)"));
    assert!(bad("TransitiveObjectProperty(ex:r) SubObjectPropertyOf(ex:r ex:s) SubClassOf(ex:A ObjectMinCardinality(2 ex:s))"), "a sub-property makes it non-simple");
    assert!(bad("TransitiveObjectProperty(ex:r) InverseObjectProperties(ex:r ex:s) SubClassOf(ex:A ObjectExactCardinality(1 ObjectInverseOf(ex:s)))"), "so does an inverse");
    assert!(!bad("TransitiveObjectProperty(ex:r) SubClassOf(ex:A ObjectSomeValuesFrom(ex:r ex:B)) SubClassOf(ex:A ObjectMaxCardinality(1 ex:s))"), "other roles are fine");
    assert!(!bad("SubObjectPropertyOf(ex:r ex:s) TransitiveObjectProperty(ex:s) SubClassOf(ex:A ObjectMaxCardinality(1 ex:r))"), "a sub-role of a transitive role is still simple");
}

#[test]
fn classes_equivalent_to_thing_are_reported_apart_from_the_hierarchy() {
    let r = kb("SubClassOf(owl:Thing ex:All) SubClassOf(ex:A ex:B) Declaration(Class(ex:C))");
    let k = r.classify().unwrap();
    assert_eq!(names(&k.top_equivalents), ["All"]);
    assert!(k.all_supers[&i("All")].is_empty() && !k.all_supers[&i("A")].contains(&i("All")));
    assert_eq!(names(&k.all_supers[&i("A")]), ["B"]);
}

// ------------------------------------------------------------------ nominals
#[test]
fn a_nominal_class_has_exactly_its_members() {
    // blue must be red or green, and cannot be either
    let r = kb("EquivalentClasses(ex:Colour ObjectOneOf(ex:red ex:green)) ClassAssertion(ex:Colour ex:blue) DifferentIndividuals(ex:red ex:green ex:blue)");
    assert!(!r.is_consistent().unwrap());
    // without the distinctness, blue may simply be one of them
    let r = kb("EquivalentClasses(ex:Colour ObjectOneOf(ex:red ex:green)) ClassAssertion(ex:Colour ex:blue)");
    assert!(r.is_consistent().unwrap());
}

#[test]
fn nominals_interact_with_the_facts_about_individuals() {
    // A has at most the individual a, and a is a B: so A is below B
    let r = kb("SubClassOf(ex:A ObjectOneOf(ex:a)) ClassAssertion(ex:B ex:a)");
    assert!(r.subsumes(&c("B"), &c("A")).unwrap());
    assert!(!r.subsumes(&c("A"), &c("B")).unwrap());
    // two different single-member nominal classes cannot overlap; without the distinctness they can
    let r = kb("SubClassOf(ex:A ObjectOneOf(ex:a)) SubClassOf(ex:B ObjectOneOf(ex:b)) DifferentIndividuals(ex:a ex:b)");
    assert!(!r.is_satisfiable(&ClassExpr::And(vec![c("A"), c("B")])).unwrap());
    assert!(r.is_satisfiable(&c("A")).unwrap());
    let r = kb("SubClassOf(ex:A ObjectOneOf(ex:a)) SubClassOf(ex:B ObjectOneOf(ex:b))");
    assert!(r.is_satisfiable(&ClassExpr::And(vec![c("A"), c("B")])).unwrap(), "a and b may be the same individual");
}

#[test]
fn a_class_with_a_nominal_on_the_left_applies_to_that_individual() {
    let r = kb("SubClassOf(ObjectOneOf(ex:a ex:b) ex:P) Declaration(NamedIndividual(ex:a)) Declaration(NamedIndividual(ex:b)) Declaration(NamedIndividual(ex:c)) Declaration(Class(ex:P))");
    assert_eq!(names(&r.types(&i("a")).unwrap()), ["P"]);
    assert_eq!(names(&r.types(&i("b")).unwrap()), ["P"]);
    assert!(r.types(&i("c")).unwrap().is_empty(), "c is neither a nor b");
}

#[test]
fn has_value_is_an_existential_over_a_nominal() {
    let r = kb("SubClassOf(ex:Rose ObjectHasValue(ex:hasColour ex:red)) ClassAssertion(ex:Warm ex:red) ClassAssertion(ex:Rose ex:r1)");
    assert!(r.subsumes(&ClassExpr::Some(ObjectProp::Named(i("hasColour")), Box::new(c("Warm"))), &c("Rose")).unwrap());
    assert!(r.instance_of(&i("r1"), &ClassExpr::HasValue(ObjectProp::Named(i("hasColour")), i("red"))).unwrap());
    assert!(!r.instance_of(&i("r1"), &ClassExpr::HasValue(ObjectProp::Named(i("hasColour")), i("blue"))).unwrap());
    // a functional colour cannot be both red and the different individual blue
    let r = kb("SubClassOf(ex:Rose ObjectHasValue(ex:hasColour ex:red)) FunctionalObjectProperty(ex:hasColour) ClassAssertion(ex:Rose ex:r1) ObjectPropertyAssertion(ex:hasColour ex:r1 ex:blue) DifferentIndividuals(ex:red ex:blue)");
    assert!(!r.is_consistent().unwrap());
}

#[test]
fn at_most_restrictions_on_a_nominal_cover_the_nodes_that_point_at_it() {
    // two different B-kinds of A each point at the individual a, which allows at most one A pointing at it
    let base = "SubClassOf(ex:A ObjectSomeValuesFrom(ex:t ObjectOneOf(ex:a))) SubClassOf(ex:D ObjectIntersectionOf(ObjectSomeValuesFrom(ex:s ObjectIntersectionOf(ex:A ex:B1)) ObjectSomeValuesFrom(ex:s ObjectIntersectionOf(ex:A ex:B2))))
                ClassAssertion(ObjectMaxCardinality(1 ObjectInverseOf(ex:t) ex:A) ex:a)";
    let r = kb(&format!("{base} DisjointClasses(ex:B1 ex:B2)"));
    assert!(!r.is_satisfiable(&c("D")).unwrap(), "the two A's would be one element that is both B1 and B2");
    let r = kb(base);
    assert!(r.is_satisfiable(&c("D")).unwrap(), "without the disjointness they are the same element");
    // inverse-functional roles at a nominal are the same thing
    let r = kb("SubClassOf(ex:A ObjectSomeValuesFrom(ex:t ObjectOneOf(ex:a))) InverseFunctionalObjectProperty(ex:t) DisjointClasses(ex:B1 ex:B2)
                SubClassOf(ex:D ObjectIntersectionOf(ObjectSomeValuesFrom(ex:s ObjectIntersectionOf(ex:A ex:B1)) ObjectSomeValuesFrom(ex:s ObjectIntersectionOf(ex:A ex:B2))))");
    assert!(!r.is_satisfiable(&c("D")).unwrap());
}

#[test]
fn a_chain_of_nodes_pointing_at_a_nominal_is_not_copied_by_blocking() {
    // every A points at a and has an A successor; at most one A may point at a. The whole chain is one element that is its own
    // successor. Blocking alone would copy the first node and give a two A neighbours.
    let r = kb("SubClassOf(ex:A ObjectIntersectionOf(ObjectSomeValuesFrom(ex:r ex:A) ObjectSomeValuesFrom(ex:t ObjectOneOf(ex:a))))
                ClassAssertion(ObjectMaxCardinality(1 ObjectInverseOf(ex:t) ex:A) ex:a)");
    assert!(r.is_satisfiable(&c("A")).unwrap());
    // two alternating, disjoint kinds cannot be one element
    let r = kb("SubClassOf(ex:A0 ObjectIntersectionOf(ObjectSomeValuesFrom(ex:r ex:A1) ObjectSomeValuesFrom(ex:t ObjectOneOf(ex:a))))
                SubClassOf(ex:A1 ObjectIntersectionOf(ObjectSomeValuesFrom(ex:r ex:A0) ObjectSomeValuesFrom(ex:t ObjectOneOf(ex:a))))
                DisjointClasses(ex:A0 ex:A1)
                ClassAssertion(ObjectMaxCardinality(1 ObjectInverseOf(ex:t) ObjectUnionOf(ex:A0 ex:A1)) ex:a)");
    assert!(!r.is_satisfiable(&c("A0")).unwrap());
    assert!(!r.is_satisfiable(&c("A1")).unwrap());
}

// ------------------------------------------------------------------ data properties
const ADULT: &str = "DatatypeRestriction(xsd:integer xsd:minInclusive \"18\"^^xsd:integer)";
const MINOR: &str = "DatatypeRestriction(xsd:integer xsd:maxExclusive \"18\"^^xsd:integer)";

#[test]
fn data_restrictions_contradict_through_their_ranges() {
    let r = kb(&format!("SubClassOf(ex:Adult DataSomeValuesFrom(ex:age {ADULT})) SubClassOf(ex:Minor DataAllValuesFrom(ex:age {MINOR})) SubClassOf(ex:Odd DataSomeValuesFrom(ex:age xsd:string)) SubClassOf(ex:Odd DataAllValuesFrom(ex:age xsd:integer))"));
    assert!(r.is_satisfiable(&c("Adult")).unwrap() && r.is_satisfiable(&c("Minor")).unwrap());
    assert!(!r.is_satisfiable(&ClassExpr::And(vec![c("Adult"), c("Minor")])).unwrap(), "some age >= 18 but every age < 18");
    assert!(!r.is_satisfiable(&c("Odd")).unwrap(), "no value is both a string and an integer");
}

#[test]
fn functional_data_properties_and_cardinalities_count_values() {
    let r = kb("FunctionalDataProperty(ex:age) SubClassOf(ex:Twice DataMinCardinality(2 ex:age)) SubClassOf(ex:Once DataExactCardinality(1 ex:age xsd:integer))");
    assert!(!r.is_satisfiable(&c("Twice")).unwrap());
    assert!(r.is_satisfiable(&c("Once")).unwrap());
    let r = kb("SubClassOf(ex:Few DataMaxCardinality(2 ex:tag)) SubClassOf(ex:Many DataMinCardinality(3 ex:tag))");
    assert!(!r.is_satisfiable(&ClassExpr::And(vec![c("Few"), c("Many")])).unwrap());
    assert!(r.subsumes(&ClassExpr::Not(Box::new(c("Many"))), &c("Few")).unwrap(), "at most 2 tags excludes at least 3");
}

#[test]
fn asserted_data_values_are_checked_against_ranges_and_counts() {
    let doc = |extra: &str| format!("DataPropertyRange(ex:age xsd:integer) ClassAssertion(ex:P ex:a) {extra}");
    assert!(kb(&doc("DataPropertyAssertion(ex:age ex:a \"30\"^^xsd:integer)")).is_consistent().unwrap());
    assert!(!kb(&doc("DataPropertyAssertion(ex:age ex:a \"thirty\")")).is_consistent().unwrap(), "a string is outside the range");
    let fun = |extra: &str| format!("FunctionalDataProperty(ex:code) {extra}");
    assert!(kb(&fun("DataPropertyAssertion(ex:code ex:x \"A1\") DataPropertyAssertion(ex:code ex:x \"A1\")")).is_consistent().unwrap(), "the same value twice is one value");
    assert!(!kb(&fun("DataPropertyAssertion(ex:code ex:x \"A1\") DataPropertyAssertion(ex:code ex:x \"B2\")")).is_consistent().unwrap());
    assert!(!kb(&fun("ClassAssertion(DataHasValue(ex:code \"A1\") ex:x) DataPropertyAssertion(ex:code ex:x \"B2\")")).is_consistent().unwrap());
    assert!(kb("ClassAssertion(DataHasValue(ex:code \"A1\") ex:x) DataPropertyAssertion(ex:code ex:x \"B2\")").is_consistent().unwrap(), "two codes are fine when the property is not functional");
}

#[test]
fn data_domains_and_defined_classes_classify_individuals() {
    let r = kb(&format!(
        "DataPropertyDomain(ex:age ex:Person) EquivalentClasses(ex:Adult DataSomeValuesFrom(ex:age {ADULT}))
         DataPropertyAssertion(ex:age ex:a \"30\"^^xsd:integer) DataPropertyAssertion(ex:age ex:b \"7\"^^xsd:integer)"
    ));
    assert_eq!(names(&r.types(&i("a")).unwrap()), ["Adult", "Person"]);
    assert_eq!(names(&r.types(&i("b")).unwrap()), ["Person"], "7 is below 18");
    let k = r.classify().unwrap();
    assert!(k.unsatisfiable.is_empty());
}

#[test]
fn data_property_hierarchies_follow_the_sub_property_in_data_restrictions() {
    // a birthYear is a year: a restriction on the super-property is met by a value of the sub-property
    let r = kb("SubDataPropertyOf(ex:birthYear ex:year) SubClassOf(ex:A DataSomeValuesFrom(ex:year xsd:integer))");
    assert!(r.is_consistent().unwrap());
    assert!(r.is_satisfiable(&c("A")).unwrap());
    // ranges and domains follow the hierarchy
    let r = kb("SubDataPropertyOf(ex:birthYear ex:year) DataPropertyRange(ex:year xsd:integer) DataPropertyAssertion(ex:birthYear ex:a \"x\")");
    assert!(!r.is_consistent().unwrap(), "a birthYear is a year, and years are integers");
    // at most one year, but a birthYear and a deathYear are both years, so two different values clash
    let r = kb("SubDataPropertyOf(ex:birthYear ex:year) SubDataPropertyOf(ex:deathYear ex:year) FunctionalDataProperty(ex:year)
                DataPropertyAssertion(ex:birthYear ex:a \"1900\"^^xsd:integer) DataPropertyAssertion(ex:deathYear ex:a \"1980\"^^xsd:integer)");
    assert!(!r.is_consistent().unwrap());
    // ... while the same value may serve both, and unrelated properties do not interact
    let r = kb("SubDataPropertyOf(ex:birthYear ex:year) SubDataPropertyOf(ex:deathYear ex:year) FunctionalDataProperty(ex:year)
                DataPropertyAssertion(ex:birthYear ex:a \"1900\"^^xsd:integer) DataPropertyAssertion(ex:deathYear ex:a \"1900\"^^xsd:integer)");
    assert!(r.is_consistent().unwrap());
    let r = kb("FunctionalDataProperty(ex:p) DataPropertyAssertion(ex:p ex:a \"1\"^^xsd:integer) DataPropertyAssertion(ex:q ex:a \"2\"^^xsd:integer)");
    assert!(r.is_consistent().unwrap());
    // two values wanted on the super-property, one allowed on each sub-property: fits (one value of each)
    let r = kb("SubDataPropertyOf(ex:p1 ex:p) SubDataPropertyOf(ex:p2 ex:p) FunctionalDataProperty(ex:p1) FunctionalDataProperty(ex:p2)
                SubClassOf(ex:A DataMinCardinality(2 ex:p xsd:integer))");
    assert!(r.is_satisfiable(&c("A")).unwrap());
    // ... but not when the super-property is functional
    let r = kb("SubDataPropertyOf(ex:p1 ex:p) SubDataPropertyOf(ex:p2 ex:p) FunctionalDataProperty(ex:p)
                SubClassOf(ex:A DataMinCardinality(2 ex:p xsd:integer))");
    assert!(!r.is_satisfiable(&c("A")).unwrap());
    // a value of a sub-property is a value of every super-property, so the ranges of both apply to it
    let r = kb("SubDataPropertyOf(ex:q ex:p1) SubDataPropertyOf(ex:q ex:p2) SubClassOf(ex:A DataSomeValuesFrom(ex:q xsd:integer))
                SubClassOf(ex:A DataAllValuesFrom(ex:p2 xsd:string))");
    assert!(!r.is_satisfiable(&c("A")).unwrap(), "the q value is a p2 value, which must be a string");
    // ... but a value given directly to the super-property p1 need not be one of q
    let r = kb("SubDataPropertyOf(ex:q ex:p1) SubDataPropertyOf(ex:q ex:p2) SubClassOf(ex:A DataSomeValuesFrom(ex:p1 xsd:integer))
                SubClassOf(ex:A DataAllValuesFrom(ex:p2 xsd:string))");
    assert!(r.is_satisfiable(&c("A")).unwrap());
    // one value of the sub-property can meet minimums on two super-properties, but a maximum on either still counts it
    let r = kb("SubDataPropertyOf(ex:q ex:p1) SubDataPropertyOf(ex:q ex:p2) SubClassOf(ex:A DataMinCardinality(2 ex:p1 xsd:integer))
                SubClassOf(ex:A DataMinCardinality(2 ex:p2 xsd:integer)) SubClassOf(ex:A DataMaxCardinality(2 ex:q xsd:integer))
                SubClassOf(ex:A DataMaxCardinality(3 ex:p1 xsd:integer))");
    assert!(r.is_satisfiable(&c("A")).unwrap());
}

#[test]
fn irreflexive_and_asymmetric_properties_forbid_loops_and_two_way_edges() {
    let r = kb("IrreflexiveObjectProperty(ex:p) ObjectPropertyAssertion(ex:p ex:a ex:a)");
    assert!(!r.is_consistent().unwrap());
    let r = kb("IrreflexiveObjectProperty(ex:p) ObjectPropertyAssertion(ex:p ex:a ex:b)");
    assert!(r.is_consistent().unwrap());
    // a sub-property's loop is a loop of the super-property
    let r = kb("IrreflexiveObjectProperty(ex:p) SubObjectPropertyOf(ex:q ex:p) ObjectPropertyAssertion(ex:q ex:a ex:a)");
    assert!(!r.is_consistent().unwrap());
    // an inverse loop is a loop too
    let r = kb("IrreflexiveObjectProperty(ex:p) ObjectPropertyAssertion(ObjectInverseOf(ex:p) ex:a ex:a)");
    assert!(!r.is_consistent().unwrap());
    // forcing a loop through a nominal or a merge
    let r = kb("IrreflexiveObjectProperty(ex:p) ObjectPropertyAssertion(ex:p ex:a ex:b) SameIndividual(ex:a ex:b)");
    assert!(!r.is_consistent().unwrap());
    // generated: an A has a p-neighbour that is an A, a B, or itself; irreflexive removes the loop only via the model
    let r = kb("IrreflexiveObjectProperty(ex:p) SubClassOf(ex:A ObjectSomeValuesFrom(ex:p ex:A)) SubClassOf(ex:A ObjectMaxCardinality(1 ex:p))");
    assert!(r.is_satisfiable(&c("A")).unwrap(), "an infinite chain is fine");
    let r = kb("IrreflexiveObjectProperty(ex:p) SubClassOf(ex:A ObjectHasValue(ex:p ex:a)) SubClassOf(ex:A ObjectOneOf(ex:a))");
    assert!(!r.is_satisfiable(&c("A")).unwrap(), "the only A is a, and a would be its own p-neighbour");

    let r = kb("AsymmetricObjectProperty(ex:p) ObjectPropertyAssertion(ex:p ex:a ex:b) ObjectPropertyAssertion(ex:p ex:b ex:a)");
    assert!(!r.is_consistent().unwrap());
    let r = kb("AsymmetricObjectProperty(ex:p) ObjectPropertyAssertion(ex:p ex:a ex:b) ObjectPropertyAssertion(ex:p ex:b ex:c)");
    assert!(r.is_consistent().unwrap());
    let r = kb("AsymmetricObjectProperty(ex:p) ObjectPropertyAssertion(ex:p ex:a ex:a)");
    assert!(!r.is_consistent().unwrap(), "asymmetric implies irreflexive");
    let r = kb("AsymmetricObjectProperty(ex:p) SymmetricObjectProperty(ex:p) ObjectPropertyAssertion(ex:p ex:a ex:b)");
    assert!(!r.is_consistent().unwrap());
}

#[test]
fn reflexive_properties_loop_on_every_individual() {
    let r = kb("ReflexiveObjectProperty(ex:p) Declaration(NamedIndividual(ex:a)) SubClassOf(ex:N ObjectHasValue(ex:p ex:a))");
    assert!(r.is_consistent().unwrap());
    assert!(r.instance_of(&i("a"), &ClassExpr::Some(ObjectProp::Named(i("p")), Box::new(ClassExpr::Thing))).unwrap());
    assert!(r.instance_of(&i("a"), &ClassExpr::HasValue(ObjectProp::Named(i("p")), i("a"))).unwrap());
    // every A is its own p-neighbour, so an A with no p-neighbour at all cannot exist
    let r = kb("ReflexiveObjectProperty(ex:p) SubClassOf(ex:A ObjectAllValuesFrom(ex:p ex:B))");
    let k = r.classify().unwrap();
    assert!(k.all_supers[&i("A")].contains(&i("B")), "{:?}", k.all_supers);
    // reflexive meets irreflexive
    let r = kb("ReflexiveObjectProperty(ex:p) IrreflexiveObjectProperty(ex:p) Declaration(NamedIndividual(ex:a))");
    assert!(!r.is_consistent().unwrap());
    // a reflexive property counts its own loop: at most one p-neighbour, and a second named one must be the individual itself
    let r = kb("ReflexiveObjectProperty(ex:p) ObjectPropertyAssertion(ex:p ex:a ex:b) DifferentIndividuals(ex:a ex:b) SubClassOf(ex:T ObjectMaxCardinality(1 ex:p)) ClassAssertion(ex:T ex:a)");
    assert!(!r.is_consistent().unwrap());
    // the loop reaches super-properties and the domain/range axioms
    let r = kb("ReflexiveObjectProperty(ex:p) SubObjectPropertyOf(ex:p ex:q) ObjectPropertyDomain(ex:q ex:D) Declaration(NamedIndividual(ex:a))");
    assert!(r.instance_of(&i("a"), &c("D")).unwrap());
}

#[test]
fn disjoint_properties_cannot_join_the_same_pair() {
    let r = kb("DisjointObjectProperties(ex:p ex:q) ObjectPropertyAssertion(ex:p ex:a ex:b) ObjectPropertyAssertion(ex:q ex:a ex:b)");
    assert!(!r.is_consistent().unwrap());
    let r = kb("DisjointObjectProperties(ex:p ex:q) ObjectPropertyAssertion(ex:p ex:a ex:b) ObjectPropertyAssertion(ex:q ex:b ex:a)");
    assert!(r.is_consistent().unwrap());
    // inverse use: q(a,b) written through the inverse as inv(q)(b,a) is the same pair as p(a,b)
    let r = kb("DisjointObjectProperties(ex:p ex:q) ObjectPropertyAssertion(ex:p ex:a ex:b) ObjectPropertyAssertion(ObjectInverseOf(ex:q) ex:b ex:a)");
    assert!(!r.is_consistent().unwrap());
    // sub-properties inherit the disjointness
    let r = kb("DisjointObjectProperties(ex:p ex:q) SubObjectPropertyOf(ex:p1 ex:p) SubObjectPropertyOf(ex:q1 ex:q) ObjectPropertyAssertion(ex:p1 ex:a ex:b) ObjectPropertyAssertion(ex:q1 ex:a ex:b)");
    assert!(!r.is_consistent().unwrap());
    // generated neighbours: a p-neighbour that is also a q-neighbour is forbidden, two distinct ones are fine
    let r = kb("DisjointObjectProperties(ex:p ex:q) SubClassOf(ex:A ObjectSomeValuesFrom(ex:p ex:X)) SubClassOf(ex:A ObjectSomeValuesFrom(ex:q ex:X)) SubClassOf(ex:A ObjectMaxCardinality(1 ex:p))");
    assert!(r.is_satisfiable(&c("A")).unwrap());
    let r = kb("DisjointObjectProperties(ex:p ex:q) SubClassOf(ex:A ObjectSomeValuesFrom(ex:p ex:X)) SubClassOf(ex:A ObjectAllValuesFrom(ex:p ex:Y)) SubClassOf(ex:A ObjectExactCardinality(1 ex:r)) SubObjectPropertyOf(ex:p ex:r) SubObjectPropertyOf(ex:q ex:r) SubClassOf(ex:A ObjectSomeValuesFrom(ex:q ex:Y))");
    assert!(!r.is_satisfiable(&c("A")).unwrap(), "one r-neighbour would have to be both a p- and a q-neighbour");
    let r = kb("DisjointObjectProperties(ex:p ex:q) SubClassOf(ex:A ObjectSomeValuesFrom(ex:p ex:X)) SubClassOf(ex:A ObjectSomeValuesFrom(ex:q ex:Y)) SubObjectPropertyOf(ex:p ex:r) SubObjectPropertyOf(ex:q ex:r) SubClassOf(ex:A ObjectMaxCardinality(2 ex:r))");
    assert!(r.is_satisfiable(&c("A")).unwrap(), "two r-neighbours are fine");
}

#[test]
fn negative_property_assertions_exclude_the_pair() {
    let r = kb("NegativeObjectPropertyAssertion(ex:p ex:a ex:b) ObjectPropertyAssertion(ex:p ex:a ex:b)");
    assert!(!r.is_consistent().unwrap());
    let r = kb("NegativeObjectPropertyAssertion(ex:p ex:a ex:b) ObjectPropertyAssertion(ex:p ex:a ex:c) Declaration(NamedIndividual(ex:b))");
    assert!(r.is_consistent().unwrap());
    // the same individual under another name still counts
    let r = kb("NegativeObjectPropertyAssertion(ex:p ex:a ex:b) ObjectPropertyAssertion(ex:p ex:a ex:c) SameIndividual(ex:b ex:c)");
    assert!(!r.is_consistent().unwrap());
    // a sub-property's edge is a p edge
    let r = kb("NegativeObjectPropertyAssertion(ex:p ex:a ex:b) SubObjectPropertyOf(ex:q ex:p) ObjectPropertyAssertion(ex:q ex:a ex:b)");
    assert!(!r.is_consistent().unwrap());
    // through an inverse
    let r = kb("NegativeObjectPropertyAssertion(ObjectInverseOf(ex:p) ex:a ex:b) ObjectPropertyAssertion(ex:p ex:b ex:a)");
    assert!(!r.is_consistent().unwrap());

    let r = kb("NegativeDataPropertyAssertion(ex:d ex:a \"1\"^^xsd:integer) DataPropertyAssertion(ex:d ex:a \"1\"^^xsd:integer)");
    assert!(!r.is_consistent().unwrap());
    let r = kb("NegativeDataPropertyAssertion(ex:d ex:a \"1\"^^xsd:integer) DataPropertyAssertion(ex:d ex:a \"2\"^^xsd:integer)");
    assert!(r.is_consistent().unwrap());
}

#[test]
fn a_disjoint_union_is_a_union_of_pairwise_disjoint_classes() {
    let r = kb("DisjointUnion(ex:A ex:B ex:C) SubClassOf(ex:B ex:Top) SubClassOf(ex:C ex:Top) Declaration(Class(ex:D))");
    let k = r.classify().unwrap();
    assert!(k.all_supers[&i("B")].contains(&i("A")), "each disjunct is a subclass of the union");
    assert!(!r.is_satisfiable(&ClassExpr::And(vec![c("B"), c("C")])).unwrap(), "disjuncts are disjoint");
    assert!(!r.is_satisfiable(&ClassExpr::And(vec![c("A"), ClassExpr::Not(Box::new(c("B"))), ClassExpr::Not(Box::new(c("C")))])).unwrap(), "the union has no other members");
    let r = kb("DisjointUnion(ex:A ex:B ex:C) ClassAssertion(ex:A ex:x) ClassAssertion(ObjectComplementOf(ex:B) ex:x)");
    assert!(r.instance_of(&i("x"), &c("C")).unwrap());
    assert!(parse_roundtrip("DisjointUnion(ex:A ex:B ex:C)"));
}

fn parse_roundtrip(axiom: &str) -> bool {
    let doc = format!("Prefix(ex:=<http://e/#>)\nOntology(<http://e>\n{axiom}\n)");
    let o = parse(&doc).unwrap();
    parse(&crate::functional::write(&o)).unwrap() == o && crate::rdfxml::export(&o).is_ok()
}

#[test]
fn self_restrictions_make_an_individual_its_own_neighbour() {
    // ∃p.Self on a class: members loop, so they are their own p-neighbours
    let r = kb("SubClassOf(ex:Narcissist ObjectHasSelf(ex:loves)) SubClassOf(ex:Narcissist ObjectAllValuesFrom(ex:loves ex:Happy))");
    let k = r.classify().unwrap();
    assert!(k.all_supers[&i("Narcissist")].contains(&i("Happy")), "{:?}", k.all_supers);
    // a loop forbidden by irreflexivity, by a negated Self, and by asymmetry
    assert!(!kb("SubClassOf(ex:A ObjectHasSelf(ex:p)) IrreflexiveObjectProperty(ex:p)").is_satisfiable(&c("A")).unwrap());
    assert!(!kb("SubClassOf(ex:A ObjectHasSelf(ex:p)) SubClassOf(ex:A ObjectComplementOf(ObjectHasSelf(ex:p)))").is_satisfiable(&c("A")).unwrap());
    assert!(!kb("SubClassOf(ex:A ObjectHasSelf(ex:p)) AsymmetricObjectProperty(ex:p)").is_satisfiable(&c("A")).unwrap());
    // ¬Self forbids asserted loops, also through a sub-property
    assert!(!kb("SubClassOf(ex:A ObjectComplementOf(ObjectHasSelf(ex:p))) SubObjectPropertyOf(ex:q ex:p) ClassAssertion(ex:A ex:a) ObjectPropertyAssertion(ex:q ex:a ex:a)").is_consistent().unwrap());
    assert!(kb("SubClassOf(ex:A ObjectComplementOf(ObjectHasSelf(ex:p))) ClassAssertion(ex:A ex:a) ObjectPropertyAssertion(ex:p ex:a ex:b)").is_consistent().unwrap());
    // an asserted loop makes the individual an instance of Self; a class defined by Self classifies it
    let r = kb("EquivalentClasses(ex:Selfish ObjectHasSelf(ex:p)) ObjectPropertyAssertion(ex:p ex:a ex:a) ObjectPropertyAssertion(ex:p ex:b ex:c)");
    assert!(r.instance_of(&i("a"), &c("Selfish")).unwrap());
    assert!(!r.instance_of(&i("b"), &c("Selfish")).unwrap());
    // Self implies a merged neighbour: a unique p-neighbour that loops must be the individual itself
    let r = kb("SubClassOf(ex:A ObjectHasSelf(ex:p)) SubClassOf(ex:A ObjectMaxCardinality(1 ex:p)) SubClassOf(ex:A ObjectSomeValuesFrom(ex:p ex:B))");
    let k = r.classify().unwrap();
    assert!(k.all_supers[&i("A")].contains(&i("B")), "the only p-neighbour is A itself, so A is a B");
    // the role must be simple
    assert!(parse("Prefix(ex:=<http://e/#>)\nOntology(<http://e> TransitiveObjectProperty(ex:p) SubClassOf(ex:A ObjectHasSelf(ex:p)))").is_err());
    assert!(parse_roundtrip("SubClassOf(ex:A ObjectHasSelf(ObjectInverseOf(ex:p)))"));
}

#[test]
fn property_chains_carry_value_restrictions_along_their_paths() {
    let base = "SubObjectPropertyOf(ObjectPropertyChain(ex:hasParent ex:hasBrother) ex:hasUncle)
                ObjectPropertyAssertion(ex:hasParent ex:a ex:b) ObjectPropertyAssertion(ex:hasBrother ex:b ex:c) Declaration(NamedIndividual(ex:d))";
    // a hasParent b, b hasBrother c, so a hasUncle c, and everything a's uncles must be applies to c
    let r = kb(&format!("{base} ClassAssertion(ObjectAllValuesFrom(ex:hasUncle ex:Nice) ex:a)"));
    assert!(r.instance_of(&i("c"), &c("Nice")).unwrap());
    assert!(!r.instance_of(&i("d"), &c("Nice")).unwrap());
    assert!(!r.instance_of(&i("b"), &c("Nice")).unwrap());
    let r = kb(&format!("{base} ClassAssertion(ObjectAllValuesFrom(ex:hasUncle ex:Nice) ex:a) ClassAssertion(ObjectComplementOf(ex:Nice) ex:c)"));
    assert!(!r.is_consistent().unwrap());
    // through the inverse: c's inverse-hasUncle neighbours include a
    let r = kb(&format!("{base} ClassAssertion(ObjectAllValuesFrom(ObjectInverseOf(ex:hasUncle) ex:Nephew) ex:c)"));
    assert!(r.instance_of(&i("a"), &c("Nephew")).unwrap());
    // the domain of the chain's target applies to the start of a path
    let r = kb(&format!("{base} ObjectPropertyDomain(ex:hasUncle ex:Person) ObjectPropertyRange(ex:hasUncle ex:Male)"));
    assert!(r.instance_of(&i("a"), &c("Person")).unwrap());
    assert!(r.instance_of(&i("c"), &c("Male")).unwrap());
    assert!(!r.instance_of(&i("b"), &c("Person")).unwrap());
    // a sub-property of the target takes part as well
    let r = kb(&format!("{base} SubObjectPropertyOf(ex:hasUncle ex:hasRelative) ClassAssertion(ObjectAllValuesFrom(ex:hasRelative ex:Kin) ex:a)"));
    assert!(r.instance_of(&i("c"), &c("Kin")).unwrap());
    // and a sub-property of a chain member: hasFather ⊑ hasParent
    let r = kb("SubObjectPropertyOf(ObjectPropertyChain(ex:hasParent ex:hasBrother) ex:hasUncle) SubObjectPropertyOf(ex:hasFather ex:hasParent) ObjectPropertyAssertion(ex:hasFather ex:a ex:b) ObjectPropertyAssertion(ex:hasBrother ex:b ex:c) ClassAssertion(ObjectAllValuesFrom(ex:hasUncle ex:Nice) ex:a)");
    assert!(r.instance_of(&i("c"), &c("Nice")).unwrap());
}

#[test]
fn recursive_chains_extend_a_property_along_another() {
    // r s ⊑ r: a path r then any number of s is an r path
    let r = kb("SubObjectPropertyOf(ObjectPropertyChain(ex:r ex:s) ex:r) ObjectPropertyAssertion(ex:r ex:a ex:b) ObjectPropertyAssertion(ex:s ex:b ex:c) ObjectPropertyAssertion(ex:s ex:c ex:d)
                ObjectPropertyAssertion(ex:s ex:a ex:e) ClassAssertion(ObjectAllValuesFrom(ex:r ex:X) ex:a)");
    assert!(r.instance_of(&i("b"), &c("X")).unwrap());
    assert!(r.instance_of(&i("c"), &c("X")).unwrap());
    assert!(r.instance_of(&i("d"), &c("X")).unwrap());
    assert!(!r.instance_of(&i("e"), &c("X")).unwrap(), "an s edge first is not an r path");
    // s r ⊑ r: any number of s, then r
    let r = kb("SubObjectPropertyOf(ObjectPropertyChain(ex:s ex:r) ex:r) ObjectPropertyAssertion(ex:s ex:a ex:b) ObjectPropertyAssertion(ex:s ex:b ex:c) ObjectPropertyAssertion(ex:r ex:c ex:d)
                ObjectPropertyAssertion(ex:r ex:a ex:f) ObjectPropertyAssertion(ex:s ex:d ex:g) ClassAssertion(ObjectAllValuesFrom(ex:r ex:X) ex:a)");
    assert!(r.instance_of(&i("d"), &c("X")).unwrap());
    assert!(r.instance_of(&i("f"), &c("X")).unwrap());
    assert!(!r.instance_of(&i("g"), &c("X")).unwrap());
    assert!(!r.instance_of(&i("b"), &c("X")).unwrap());
    // r r ⊑ r written as a chain is transitivity
    let r = kb("SubObjectPropertyOf(ObjectPropertyChain(ex:r ex:r) ex:r) ObjectPropertyAssertion(ex:r ex:a ex:b) ObjectPropertyAssertion(ex:r ex:b ex:c) ClassAssertion(ObjectAllValuesFrom(ex:r ex:X) ex:a)");
    assert!(r.instance_of(&i("c"), &c("X")).unwrap());
    // a chain of three, with a transitive middle
    let r = kb("SubObjectPropertyOf(ObjectPropertyChain(ex:p ex:t ex:q) ex:u) TransitiveObjectProperty(ex:t) ObjectPropertyAssertion(ex:p ex:a ex:b) ObjectPropertyAssertion(ex:t ex:b ex:c) ObjectPropertyAssertion(ex:t ex:c ex:d) ObjectPropertyAssertion(ex:q ex:d ex:e) ClassAssertion(ObjectAllValuesFrom(ex:u ex:X) ex:a)");
    assert!(r.instance_of(&i("e"), &c("X")).unwrap());
    assert!(!r.instance_of(&i("d"), &c("X")).unwrap());
}

#[test]
fn property_chains_must_be_regular_and_their_targets_simple_nowhere() {
    let parse_err = |axioms: &str| parse(&format!("Prefix(ex:=<http://e/#>)\nOntology(<http://e> {axioms})")).unwrap_err().to_string();
    // r s r ⊑ r is not regular
    assert!(parse_err("SubObjectPropertyOf(ObjectPropertyChain(ex:r ex:s ex:r) ex:r)").contains("regular"));
    // r r ⊑ s together with s ⊑ r goes round in a circle
    assert!(parse_err("SubObjectPropertyOf(ObjectPropertyChain(ex:r ex:r) ex:s) SubObjectPropertyOf(ex:s ex:r)").contains("regular"));
    // two chains that need each other
    assert!(parse_err("SubObjectPropertyOf(ObjectPropertyChain(ex:r ex:t) ex:s) SubObjectPropertyOf(ObjectPropertyChain(ex:s ex:t) ex:r)").contains("regular"));
    // a chain target is not simple: no cardinality, no functionality
    assert!(parse_err("SubObjectPropertyOf(ObjectPropertyChain(ex:p ex:q) ex:u) FunctionalObjectProperty(ex:u)").contains("cannot be used"));
    assert!(parse_err("SubObjectPropertyOf(ObjectPropertyChain(ex:p ex:q) ex:u) SubClassOf(ex:A ObjectMaxCardinality(1 ex:u))").contains("cannot be used"));
    assert!(parse_err("SubObjectPropertyOf(ObjectPropertyChain(ex:p ex:q) ex:u) SubObjectPropertyOf(ex:u ex:v) IrreflexiveObjectProperty(ex:v)").contains("cannot be used"));
    // but its chain members stay simple, and chain targets are fine in ∀ and ∃
    assert!(parse("Prefix(ex:=<http://e/#>)\nOntology(<http://e> SubObjectPropertyOf(ObjectPropertyChain(ex:p ex:q) ex:u) FunctionalObjectProperty(ex:p) SubClassOf(ex:A ObjectSomeValuesFrom(ex:u ex:B)))").is_ok());
    assert!(parse_roundtrip("SubObjectPropertyOf(ObjectPropertyChain(ex:p ObjectInverseOf(ex:q)) ex:u)"));
    assert!(parse_roundtrip("SubObjectPropertyOf(ObjectPropertyChain(ex:p ex:q) ObjectInverseOf(ex:u))"));
}

#[test]
fn a_property_disjoint_with_itself_has_no_edges() {
    assert!(!kb("DisjointObjectProperties(ex:p ex:p) ObjectPropertyAssertion(ex:p ex:a ex:b)").is_consistent().unwrap());
    assert!(kb("DisjointObjectProperties(ex:p ex:p) Declaration(NamedIndividual(ex:a))").is_consistent().unwrap());
    assert!(!kb("DisjointObjectProperties(ex:p ex:p) ReflexiveObjectProperty(ex:p) Declaration(NamedIndividual(ex:a))").is_consistent().unwrap());
}

#[test]
fn disjoint_data_properties_share_no_value() {
    let one = "\"1\"^^xsd:integer";
    // the same asserted value for both
    let r = kb(&format!("DisjointDataProperties(ex:p ex:q) DataPropertyAssertion(ex:p ex:a {one}) DataPropertyAssertion(ex:q ex:a {one})"));
    assert!(!r.is_consistent().unwrap());
    let r = kb(&format!("DisjointDataProperties(ex:p ex:q) DataPropertyAssertion(ex:p ex:a {one}) DataPropertyAssertion(ex:q ex:a \"2\"^^xsd:integer)"));
    assert!(r.is_consistent().unwrap());
    // one individual's values for another property do not matter
    let r = kb(&format!("DisjointDataProperties(ex:p ex:q) DataPropertyAssertion(ex:p ex:a {one}) DataPropertyAssertion(ex:q ex:b {one})"));
    assert!(r.is_consistent().unwrap());
    // a value both properties are forced to take
    let r = kb(&format!("DisjointDataProperties(ex:p ex:q) SubClassOf(ex:A DataHasValue(ex:p {one})) SubClassOf(ex:A DataHasValue(ex:q {one}))"));
    assert!(!r.is_satisfiable(&c("A")).unwrap());
    // ... and one they can avoid
    let r = kb("DisjointDataProperties(ex:p ex:q) SubClassOf(ex:A DataSomeValuesFrom(ex:p xsd:integer)) SubClassOf(ex:A DataSomeValuesFrom(ex:q xsd:integer))");
    assert!(r.is_satisfiable(&c("A")).unwrap());
    // but not when only one integer exists in range for both
    let r = kb(&format!("DisjointDataProperties(ex:p ex:q) SubClassOf(ex:A DataAllValuesFrom(ex:p DataOneOf({one}))) SubClassOf(ex:A DataAllValuesFrom(ex:q DataOneOf({one}))) SubClassOf(ex:A DataSomeValuesFrom(ex:p xsd:integer)) SubClassOf(ex:A DataSomeValuesFrom(ex:q xsd:integer))"));
    assert!(!r.is_satisfiable(&c("A")).unwrap());
    // sub-properties inherit the disjointness
    let r = kb(&format!("DisjointDataProperties(ex:p ex:q) SubDataPropertyOf(ex:p1 ex:p) SubDataPropertyOf(ex:q1 ex:q) DataPropertyAssertion(ex:p1 ex:a {one}) DataPropertyAssertion(ex:q1 ex:a {one})"));
    assert!(!r.is_consistent().unwrap());
    // a property disjoint with itself has no values
    let r = kb(&format!("DisjointDataProperties(ex:p ex:p) DataPropertyAssertion(ex:p ex:a {one})"));
    assert!(!r.is_consistent().unwrap());
    assert!(parse_roundtrip("DisjointDataProperties(ex:p ex:q ex:r)"));
}

#[test]
fn keys_identify_named_instances_that_agree_on_every_key_property() {
    let people = "HasKey(ex:Person (ex:hasPassport) ()) ClassAssertion(ex:Person ex:a) ClassAssertion(ex:Person ex:b)
                  ObjectPropertyAssertion(ex:hasPassport ex:a ex:p1) ObjectPropertyAssertion(ex:hasPassport ex:b ex:p1)";
    // the same passport: a and b are one, so they cannot be declared different
    assert!(!kb(&format!("{people} DifferentIndividuals(ex:a ex:b)")).is_consistent().unwrap());
    // ... and what is said about one is said about the other
    let r = kb(&format!("{people} ClassAssertion(ex:Red ex:a)"));
    assert!(r.instance_of(&i("b"), &c("Red")).unwrap());
    // not instances of the class: no identification
    let r = kb("HasKey(ex:Person (ex:hasPassport) ()) ClassAssertion(ex:Person ex:a) ObjectPropertyAssertion(ex:hasPassport ex:a ex:p1) ObjectPropertyAssertion(ex:hasPassport ex:b ex:p1) DifferentIndividuals(ex:a ex:b)");
    assert!(r.is_consistent().unwrap());
    // b is a Person unless it is something else, and the something else is a kind of Person: the key bites either way
    let r = kb(&format!("{people} ClassAssertion(ex:Red ex:a) ClassAssertion(ObjectComplementOf(ex:Red) ex:b)"));
    assert!(!r.is_consistent().unwrap());
    // different passports, or a passport that is not a named individual's neighbour
    let r = kb("HasKey(ex:Person (ex:hasPassport) ()) ClassAssertion(ex:Person ex:a) ClassAssertion(ex:Person ex:b) ObjectPropertyAssertion(ex:hasPassport ex:a ex:p1) ObjectPropertyAssertion(ex:hasPassport ex:b ex:p2) DifferentIndividuals(ex:a ex:b)");
    assert!(r.is_consistent().unwrap());
    let r = kb("HasKey(ex:Person (ex:hasPassport) ()) ClassAssertion(ex:Person ex:a) ClassAssertion(ex:Person ex:b) ClassAssertion(ObjectSomeValuesFrom(ex:hasPassport ex:Doc) ex:a) ClassAssertion(ObjectSomeValuesFrom(ex:hasPassport ex:Doc) ex:b) DifferentIndividuals(ex:a ex:b)");
    assert!(r.is_consistent().unwrap(), "an unnamed passport is not shared");
    // data keys, and two key properties at once
    let one = "\"1\"^^xsd:integer";
    let r = kb(&format!("HasKey(ex:Person () (ex:ssn)) ClassAssertion(ex:Person ex:a) ClassAssertion(ex:Person ex:b) DataPropertyAssertion(ex:ssn ex:a {one}) DataPropertyAssertion(ex:ssn ex:b {one}) DifferentIndividuals(ex:a ex:b)"));
    assert!(!r.is_consistent().unwrap());
    let r = kb(&format!("HasKey(ex:Person (ex:hasPassport) (ex:ssn)) ClassAssertion(ex:Person ex:a) ClassAssertion(ex:Person ex:b) DataPropertyAssertion(ex:ssn ex:a {one}) DataPropertyAssertion(ex:ssn ex:b {one}) ObjectPropertyAssertion(ex:hasPassport ex:a ex:p1) DifferentIndividuals(ex:a ex:b)"));
    assert!(r.is_consistent().unwrap(), "only the data key agrees");
    let r = kb(&format!("HasKey(ex:Person (ex:hasPassport) (ex:ssn)) ClassAssertion(ex:Person ex:a) ClassAssertion(ex:Person ex:b) DataPropertyAssertion(ex:ssn ex:a {one}) DataPropertyAssertion(ex:ssn ex:b {one}) ObjectPropertyAssertion(ex:hasPassport ex:a ex:p1) ObjectPropertyAssertion(ex:hasPassport ex:b ex:p1) DifferentIndividuals(ex:a ex:b)"));
    assert!(!r.is_consistent().unwrap());
    // a sub-property counts for the key property
    let r = kb("HasKey(ex:Person (ex:hasId) ()) SubObjectPropertyOf(ex:hasPassport ex:hasId) ClassAssertion(ex:Person ex:a) ClassAssertion(ex:Person ex:b) ObjectPropertyAssertion(ex:hasPassport ex:a ex:p1) ObjectPropertyAssertion(ex:hasId ex:b ex:p1) DifferentIndividuals(ex:a ex:b)");
    assert!(!r.is_consistent().unwrap());
    assert!(parse_roundtrip("HasKey(ex:Person (ex:hasPassport ObjectInverseOf(ex:p)) (ex:ssn))"));
    assert!(parse_roundtrip("HasKey(ObjectUnionOf(ex:A ex:B) () (ex:ssn))"));
}

#[test]
fn named_datatypes_stand_for_their_ranges() {
    let defs = "DatatypeDefinition(ex:Adult DatatypeRestriction(xsd:integer xsd:minInclusive \"18\"^^xsd:integer)) DatatypeDefinition(ex:Senior DatatypeRestriction(ex:Adult xsd:minInclusive \"65\"^^xsd:integer))";
    let r = kb(&format!("{defs} DataPropertyRange(ex:age ex:Adult) DataPropertyAssertion(ex:age ex:a \"20\"^^xsd:integer)"));
    assert!(r.is_consistent().unwrap());
    let r = kb(&format!("{defs} DataPropertyRange(ex:age ex:Adult) DataPropertyAssertion(ex:age ex:a \"17\"^^xsd:integer)"));
    assert!(!r.is_consistent().unwrap());
}

#[test]
fn datatype_definitions_may_come_late_and_may_not_go_round() {
    let p = |axioms: &str| parse(&format!("Prefix(ex:=<http://e/#>)\nOntology(<http://e> {axioms})"));
    assert!(p("DataPropertyRange(ex:age ex:Adult) DatatypeDefinition(ex:Adult DataComplementOf(xsd:string))").is_ok());
    assert!(p("DatatypeDefinition(ex:A ex:B) DatatypeDefinition(ex:B ex:A)").is_err());
    assert!(parse_roundtrip("DatatypeDefinition(ex:Adult DataIntersectionOf(xsd:integer DataComplementOf(DataOneOf(\"1\"^^xsd:integer))))"));
}

#[test]
fn restricting_a_defined_datatype_keeps_both_sets_of_facets() {
    let defs = "DatatypeDefinition(ex:Adult DatatypeRestriction(xsd:integer xsd:minInclusive \"18\"^^xsd:integer)) DatatypeDefinition(ex:Senior DatatypeRestriction(ex:Adult xsd:minInclusive \"65\"^^xsd:integer)) DataPropertyRange(ex:age ex:Senior)";
    assert!(kb(&format!("{defs} DataPropertyAssertion(ex:age ex:a \"70\"^^xsd:integer)")).is_consistent().unwrap());
    assert!(!kb(&format!("{defs} DataPropertyAssertion(ex:age ex:a \"30\"^^xsd:integer)")).is_consistent().unwrap());
    assert!(!kb(&format!("{defs} DataPropertyAssertion(ex:age ex:a \"10\"^^xsd:integer)")).is_consistent().unwrap());
}

#[test]
fn a_memory_budget_stops_the_search_with_a_budget_error_never_a_verdict() {
    let axioms = "ClassAssertion(ex:A ex:a) SubClassOf(ex:A ObjectSomeValuesFrom(ex:r ex:B)) SubClassOf(ex:B ObjectUnionOf(ex:C ex:D))";
    let doc = format!("Prefix(ex:=<http://e/#>)
Ontology(<http://e>
{axioms}
)");
    let o = parse(&doc).unwrap();
    assert_eq!(Reasoner::new(&o).is_consistent(), Ok(true), "the default budget is ample for a small ontology");
    assert_eq!(Reasoner::new(&o).with_memory_budget(1).is_consistent(), Err(ReasonError::Budget("memory")), "one cell is not enough, and that is an error, not 'inconsistent'");
}


#[test]
fn a_huge_at_least_count_is_refused_before_its_nodes_are_built() {
    // `ObjectMinCardinality(60000 ...)` would make 60000 nodes and 1.8 billion pairwise inequalities inside ONE step, where no
    // between-step check can run (W3C consistent907 did this, to 13 GB)
    let doc = "Prefix(ex:=<http://e/#>)
Ontology(<http://e>
ClassAssertion(ObjectMinCardinality(60000 ex:r owl:Thing) ex:a)
)";
    let o = parse(doc).unwrap();
    let t = std::time::Instant::now();
    let got = Reasoner::new(&o).is_consistent();
    assert!(matches!(got, Err(ReasonError::Budget("nodes")) | Err(ReasonError::Budget("memory"))), "{got:?}");
    assert!(t.elapsed().as_secs() < 5, "refused up front, not after building");
    assert_eq!(Reasoner::new(&o).with_budget(100_000, 100_000).with_memory_budget(1_000_000).is_consistent(), Err(ReasonError::Budget("memory")), "the inequalities alone are over a small memory budget");
}
