// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! RDF/XML **export** (never read): the interchange format other OWL tools understand. Follows the W3C OWL 2 RDF
//! mapping for the constructs in the XIAR profile, with one element per axiom so the output stays easy to diff.

use crate::model::*;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExportError {
    #[error("cannot write `{0}` as an XML element name (needed for a property assertion)")]
    NotAnElementName(String),
}

const NS_RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const NS_OWL: &str = "http://www.w3.org/2002/07/owl#";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// `<ns:local xmlns:ns="namespace" ...attrs>` pieces for an IRI used as an element name.
fn element_name(i: &Iri) -> Result<(String, String), ExportError> {
    let split = i.0.rfind(['#', '/']).map(|p| p + 1).unwrap_or(0);
    let (ns, local) = i.0.split_at(split);
    if ns.is_empty() || !is_ncname(local) {
        return Err(ExportError::NotAnElementName(i.0.clone()));
    }
    Ok((format!("p:{local}"), format!(" xmlns:p=\"{}\"", esc(ns))))
}

fn res(tag: &str, i: &str) -> String {
    format!("<{tag} rdf:resource=\"{}\"/>", esc(i))
}

fn dprop_ref(p: &Iri) -> String {
    format!("<owl:DatatypeProperty rdf:about=\"{}\"/>", esc(&p.0))
}

/// A data range as an element (OWL 2 RDF mapping: `rdfs:Datatype` with `owl:oneOf`, `owl:datatypeComplementOf`,
/// `owl:intersectionOf`, `owl:unionOf` or `owl:onDatatype` + `owl:withRestrictions`).
fn data_range(d: &DataRange) -> String {
    match d {
        DataRange::Top => format!("<rdfs:Datatype rdf:about=\"{RDFS_LITERAL}\"/>"),
        DataRange::Datatype(t) => format!("<rdfs:Datatype rdf:about=\"{}\"/>", t.iri()),
        DataRange::OneOf(v) => {
            // an RDF list of literals, written with rdf:first / rdf:rest
            let mut list = format!("<rdf:Description rdf:about=\"{NS_RDF}nil\"/>");
            for l in v.iter().rev() {
                list = format!("<rdf:Description><rdf:first {}</rdf:first><rdf:rest>{list}</rdf:rest></rdf:Description>", lit(l));
            }
            format!("<rdfs:Datatype><owl:oneOf>{list}</owl:oneOf></rdfs:Datatype>")
        }
        DataRange::Not(x) => format!("<rdfs:Datatype><owl:datatypeComplementOf>{}</owl:datatypeComplementOf></rdfs:Datatype>", data_range(x)),
        DataRange::And(v) => format!("<rdfs:Datatype><owl:intersectionOf rdf:parseType=\"Collection\">{}</owl:intersectionOf></rdfs:Datatype>", v.iter().map(data_range).collect::<String>()),
        DataRange::Or(v) => format!("<rdfs:Datatype><owl:unionOf rdf:parseType=\"Collection\">{}</owl:unionOf></rdfs:Datatype>", v.iter().map(data_range).collect::<String>()),
        DataRange::Facets(t, fs) => {
            let items: String = fs
                .iter()
                .map(|(f, l)| {
                    let local = f.iri().rsplit('#').next().unwrap_or("");
                    format!("<rdf:Description><xsd:{local} {}</xsd:{local}></rdf:Description>", lit(l))
                })
                .collect();
            format!("<rdfs:Datatype><owl:onDatatype rdf:resource=\"{}\"/><owl:withRestrictions rdf:parseType=\"Collection\">{items}</owl:withRestrictions></rdfs:Datatype>", t.iri())
        }
    }
}

fn class_ref(c: &ClassExpr) -> String {
    match c {
        ClassExpr::Thing => "<owl:Class rdf:about=\"http://www.w3.org/2002/07/owl#Thing\"/>".into(),
        ClassExpr::Nothing => "<owl:Class rdf:about=\"http://www.w3.org/2002/07/owl#Nothing\"/>".into(),
        _ => ce_element(c, ""),
    }
}

/// A class expression as an element; `extra` is spliced in as additional child elements (used to attach axioms to
/// anonymous classes).
fn ce_element(c: &ClassExpr, extra: &str) -> String {
    let list = |tag: &str, v: &[ClassExpr]| {
        let items: String = v.iter().map(class_ref).collect();
        format!("<owl:Class><{tag} rdf:parseType=\"Collection\">{items}</{tag}>{extra}</owl:Class>")
    };
    let restriction = |p: &ObjectProp, body: String| format!("<owl:Restriction><owl:onProperty>{}</owl:onProperty>{body}{extra}</owl:Restriction>", prop_ref(p));
    let card = |kind: &str, n: &u32, p: &ObjectProp, d: &ClassExpr| {
        let int = format!("rdf:datatype=\"http://www.w3.org/2001/XMLSchema#nonNegativeInteger\">{n}");
        if *d == ClassExpr::Thing {
            restriction(p, format!("<owl:{kind}Cardinality {int}</owl:{kind}Cardinality>"))
        } else {
            restriction(p, format!("<owl:{kind}QualifiedCardinality {int}</owl:{kind}QualifiedCardinality><owl:onClass>{}</owl:onClass>", class_ref(d)))
        }
    };
    let drestriction = |p: &Iri, body: String| format!("<owl:Restriction><owl:onProperty>{}</owl:onProperty>{body}{extra}</owl:Restriction>", dprop_ref(p));
    let dcard = |kind: &str, n: &u32, p: &Iri, d: &DataRange| {
        let int = format!("rdf:datatype=\"http://www.w3.org/2001/XMLSchema#nonNegativeInteger\">{n}");
        if *d == DataRange::Top {
            drestriction(p, format!("<owl:{kind}Cardinality {int}</owl:{kind}Cardinality>"))
        } else {
            drestriction(p, format!("<owl:{kind}QualifiedCardinality {int}</owl:{kind}QualifiedCardinality><owl:onDataRange>{}</owl:onDataRange>", data_range(d)))
        }
    };
    match c {
        ClassExpr::Thing => format!("<owl:Class rdf:about=\"{OWL_THING}\">{extra}</owl:Class>"),
        ClassExpr::Nothing => format!("<owl:Class rdf:about=\"{OWL_NOTHING}\">{extra}</owl:Class>"),
        ClassExpr::Class(i) => format!("<owl:Class rdf:about=\"{}\">{extra}</owl:Class>", esc(&i.0)),
        ClassExpr::And(v) => list("owl:intersectionOf", v),
        ClassExpr::Or(v) => list("owl:unionOf", v),
        ClassExpr::Not(x) => format!("<owl:Class><owl:complementOf>{}</owl:complementOf>{extra}</owl:Class>", class_ref(x)),
        ClassExpr::Some(p, d) => restriction(p, format!("<owl:someValuesFrom>{}</owl:someValuesFrom>", class_ref(d))),
        ClassExpr::All(p, d) => restriction(p, format!("<owl:allValuesFrom>{}</owl:allValuesFrom>", class_ref(d))),
        ClassExpr::Min(n, p, d) => card("min", n, p, d),
        ClassExpr::Max(n, p, d) => card("max", n, p, d),
        ClassExpr::Exact(n, p, d) => card("", n, p, d).replace("<owl:Cardinality", "<owl:cardinality").replace("</owl:Cardinality>", "</owl:cardinality>").replace("owl:QualifiedCardinality", "owl:qualifiedCardinality"),
        ClassExpr::OneOf(v) => {
            let items: String = v.iter().map(|i| format!("<owl:NamedIndividual rdf:about=\"{}\"/>", esc(&i.0))).collect();
            format!("<owl:Class><owl:oneOf rdf:parseType=\"Collection\">{items}</owl:oneOf>{extra}</owl:Class>")
        }
        ClassExpr::HasValue(p, i) => restriction(p, res("owl:hasValue", &i.0)),
        ClassExpr::AllState(..) => String::new(), // internal to the reasoner
        ClassExpr::HasSelf(p) => restriction(p, "<owl:hasSelf rdf:datatype=\"http://www.w3.org/2001/XMLSchema#boolean\">true</owl:hasSelf>".to_string()),
        ClassExpr::DataSome(p, d) => drestriction(p, format!("<owl:someValuesFrom>{}</owl:someValuesFrom>", data_range(d))),
        ClassExpr::DataAll(p, d) => drestriction(p, format!("<owl:allValuesFrom>{}</owl:allValuesFrom>", data_range(d))),
        ClassExpr::DataHasValue(p, l) => drestriction(p, format!("<owl:hasValue {}</owl:hasValue>", lit(l))),
        ClassExpr::DataMin(n, p, d) => dcard("min", n, p, d),
        ClassExpr::DataMax(n, p, d) => dcard("max", n, p, d),
        ClassExpr::DataExact(n, p, d) => dcard("", n, p, d).replace("<owl:Cardinality", "<owl:cardinality").replace("</owl:Cardinality>", "</owl:cardinality>").replace("owl:QualifiedCardinality", "owl:qualifiedCardinality"),
    }
}

fn prop_ref(p: &ObjectProp) -> String {
    match p {
        ObjectProp::Named(i) => format!("<owl:ObjectProperty rdf:about=\"{}\"/>", esc(&i.0)),
        ObjectProp::Inverse(i) => format!("<rdf:Description><owl:inverseOf rdf:resource=\"{}\"/></rdf:Description>", esc(&i.0)),
    }
}

/// A property as an element carrying extra children (axioms about it).
fn prop_element(p: &ObjectProp, extra: &str) -> String {
    match p {
        ObjectProp::Named(i) => format!("<owl:ObjectProperty rdf:about=\"{}\">{extra}</owl:ObjectProperty>", esc(&i.0)),
        ObjectProp::Inverse(i) => format!("<rdf:Description><owl:inverseOf rdf:resource=\"{}\"/>{extra}</rdf:Description>", esc(&i.0)),
    }
}

fn type_res(t: &str) -> String {
    res("rdf:type", &format!("{NS_OWL}{t}"))
}

fn lit(l: &Literal) -> String {
    format!("rdf:datatype=\"{}\">{}", l.datatype.iri(), esc(&l.lexical))
}

/// Axioms about an inverse property are restated about the named property, so every property axiom has a named subject
/// (tools differ in how well they read axioms attached to an anonymous `owl:inverseOf` node).
fn with_named_subject(a: &Axiom) -> Axiom {
    use ObjectProp::{Inverse, Named};
    match a {
        Axiom::ObjectPropertyDomain(Inverse(i), c) => Axiom::ObjectPropertyRange(Named(i.clone()), c.clone()),
        Axiom::ObjectPropertyRange(Inverse(i), c) => Axiom::ObjectPropertyDomain(Named(i.clone()), c.clone()),
        Axiom::FunctionalObjectProperty(Inverse(i)) => Axiom::InverseFunctionalObjectProperty(Named(i.clone())),
        Axiom::InverseFunctionalObjectProperty(Inverse(i)) => Axiom::FunctionalObjectProperty(Named(i.clone())),
        Axiom::TransitiveObjectProperty(Inverse(i)) => Axiom::TransitiveObjectProperty(Named(i.clone())),
        Axiom::SymmetricObjectProperty(Inverse(i)) => Axiom::SymmetricObjectProperty(Named(i.clone())),
        Axiom::ReflexiveObjectProperty(Inverse(i)) => Axiom::ReflexiveObjectProperty(Named(i.clone())),
        Axiom::IrreflexiveObjectProperty(Inverse(i)) => Axiom::IrreflexiveObjectProperty(Named(i.clone())),
        Axiom::AsymmetricObjectProperty(Inverse(i)) => Axiom::AsymmetricObjectProperty(Named(i.clone())),
        Axiom::SubPropertyChain(w, t @ Inverse(_)) => Axiom::SubPropertyChain(w.iter().rev().map(ObjectProp::inverse).collect(), t.inverse()),
        Axiom::SubObjectPropertyOf(p @ Inverse(_), q) => Axiom::SubObjectPropertyOf(p.inverse(), q.inverse()),
        Axiom::EquivalentObjectProperties(v) if matches!(v.first(), Some(Inverse(_))) => Axiom::EquivalentObjectProperties(v.iter().map(ObjectProp::inverse).collect()),
        other => other.clone(),
    }
}

fn axiom(a: &Axiom) -> Result<String, ExportError> {
    let a = &with_named_subject(a);
    Ok(match a {
        Axiom::DeclareClass(i) => format!("<owl:Class rdf:about=\"{}\"/>", esc(&i.0)),
        Axiom::DeclareObjectProperty(i) => format!("<owl:ObjectProperty rdf:about=\"{}\"/>", esc(&i.0)),
        Axiom::DeclareDataProperty(i) => format!("<owl:DatatypeProperty rdf:about=\"{}\"/>", esc(&i.0)),
        Axiom::DeclareIndividual(i) => format!("<owl:NamedIndividual rdf:about=\"{}\"/>", esc(&i.0)),
        Axiom::SubClassOf(x, y) => ce_element(x, &format!("<rdfs:subClassOf>{}</rdfs:subClassOf>", class_ref(y))),
        Axiom::EquivalentClasses(v) => v[1..].iter().map(|y| ce_element(&v[0], &format!("<owl:equivalentClass>{}</owl:equivalentClass>", class_ref(y)))).collect(),
        Axiom::DisjointClasses(v) => {
            let mut out = String::new();
            for i in 0..v.len() {
                for j in (i + 1)..v.len() {
                    out.push_str(&ce_element(&v[i], &format!("<owl:disjointWith>{}</owl:disjointWith>", class_ref(&v[j]))));
                }
            }
            out
        }
        Axiom::HasKey(c, ps, ds) => {
            let items: String = ps.iter().map(prop_ref).chain(ds.iter().map(dprop_ref)).collect();
            ce_element(c, &format!("<owl:hasKey rdf:parseType=\"Collection\">{items}</owl:hasKey>"))
        }
        Axiom::DatatypeDefinition(n, d) => format!("<rdfs:Datatype rdf:about=\"{}\"><owl:equivalentClass>{}</owl:equivalentClass></rdfs:Datatype>", esc(&n.0), data_range(d)),
        Axiom::DisjointUnion(c, v) => {
            let items: String = v.iter().map(class_ref).collect();
            format!("<owl:Class rdf:about=\"{}\"><owl:disjointUnionOf rdf:parseType=\"Collection\">{items}</owl:disjointUnionOf></owl:Class>", esc(&c.0))
        }
        Axiom::SubPropertyChain(w, t) => {
            let items: String = w.iter().map(prop_ref).collect();
            prop_element(t, &format!("<owl:propertyChainAxiom rdf:parseType=\"Collection\">{items}</owl:propertyChainAxiom>"))
        }
        Axiom::SubObjectPropertyOf(p, q) => prop_element(p, &format!("<rdfs:subPropertyOf>{}</rdfs:subPropertyOf>", prop_ref(q))),
        Axiom::EquivalentObjectProperties(v) => v[1..].iter().map(|q| prop_element(&v[0], &format!("<owl:equivalentProperty>{}</owl:equivalentProperty>", prop_ref(q)))).collect(),
        Axiom::InverseObjectProperties(p, q) => prop_element(&ObjectProp::Named(p.clone()), &res("owl:inverseOf", &q.0)),
        Axiom::ObjectPropertyDomain(p, c) => prop_element(p, &format!("<rdfs:domain>{}</rdfs:domain>", class_ref(c))),
        Axiom::ObjectPropertyRange(p, c) => prop_element(p, &format!("<rdfs:range>{}</rdfs:range>", class_ref(c))),
        Axiom::FunctionalObjectProperty(p) => prop_element(p, &type_res("FunctionalProperty")),
        Axiom::InverseFunctionalObjectProperty(p) => prop_element(p, &type_res("InverseFunctionalProperty")),
        Axiom::TransitiveObjectProperty(p) => prop_element(p, &type_res("TransitiveProperty")),
        Axiom::SymmetricObjectProperty(p) => prop_element(p, &type_res("SymmetricProperty")),
        Axiom::ReflexiveObjectProperty(p) => prop_element(p, &type_res("ReflexiveProperty")),
        Axiom::IrreflexiveObjectProperty(p) => prop_element(p, &type_res("IrreflexiveProperty")),
        Axiom::AsymmetricObjectProperty(p) => prop_element(p, &type_res("AsymmetricProperty")),
        Axiom::DisjointObjectProperties(v) => {
            let mut out = String::new();
            for i in 0..v.len() {
                for j in (i + 1)..v.len() {
                    out.push_str(&prop_element(&v[i], &format!("<owl:propertyDisjointWith>{}</owl:propertyDisjointWith>", prop_ref(&v[j]))));
                }
            }
            out
        }
        Axiom::NegativeObjectPropertyAssertion(p, a, b) => {
            let (s, o) = match p {
                ObjectProp::Named(_) => (a, b),
                ObjectProp::Inverse(_) => (b, a),
            };
            format!("<owl:NegativePropertyAssertion>{}{}{}</owl:NegativePropertyAssertion>", res("owl:sourceIndividual", &s.0), res("owl:assertionProperty", &p.named().0), res("owl:targetIndividual", &o.0))
        }
        Axiom::NegativeDataPropertyAssertion(p, a, l) => {
            format!("<owl:NegativePropertyAssertion>{}{}<owl:targetValue {}</owl:targetValue></owl:NegativePropertyAssertion>", res("owl:sourceIndividual", &a.0), res("owl:assertionProperty", &p.0), lit(l))
        }
        Axiom::DataPropertyDomain(p, c) => format!("<owl:DatatypeProperty rdf:about=\"{}\"><rdfs:domain>{}</rdfs:domain></owl:DatatypeProperty>", esc(&p.0), class_ref(c)),
        Axiom::DataPropertyRange(p, d) => format!("<owl:DatatypeProperty rdf:about=\"{}\"><rdfs:range>{}</rdfs:range></owl:DatatypeProperty>", esc(&p.0), data_range(d)),
        Axiom::FunctionalDataProperty(p) => format!("<owl:DatatypeProperty rdf:about=\"{}\">{}</owl:DatatypeProperty>", esc(&p.0), type_res("FunctionalProperty")),
        Axiom::SubDataPropertyOf(p, q) => format!("<owl:DatatypeProperty rdf:about=\"{}\">{}</owl:DatatypeProperty>", esc(&p.0), res("rdfs:subPropertyOf", &q.0)),
        Axiom::DisjointDataProperties(v) => {
            let mut out = String::new();
            for i in 0..v.len() {
                for j in (i + 1)..v.len() {
                    out.push_str(&format!("<owl:DatatypeProperty rdf:about=\"{}\">{}</owl:DatatypeProperty>", esc(&v[i].0), res("owl:propertyDisjointWith", &v[j].0)));
                }
            }
            out
        }
        Axiom::EquivalentDataProperties(v) => v[1..].iter().map(|q| format!("<owl:DatatypeProperty rdf:about=\"{}\">{}</owl:DatatypeProperty>", esc(&v[0].0), res("owl:equivalentProperty", &q.0))).collect(),
        Axiom::ClassAssertion(c, i) => match c {
            ClassExpr::Class(n) => format!("<owl:NamedIndividual rdf:about=\"{}\">{}</owl:NamedIndividual>", esc(&i.0), res("rdf:type", &n.0)),
            other => format!("<owl:NamedIndividual rdf:about=\"{}\"><rdf:type>{}</rdf:type></owl:NamedIndividual>", esc(&i.0), class_ref(other)),
        },
        Axiom::ObjectPropertyAssertion(p, a, b) => {
            let (subject, name, object) = match p {
                ObjectProp::Named(n) => (a, n, b),
                ObjectProp::Inverse(n) => (b, n, a),
            };
            let (tag, ns) = element_name(name)?;
            format!("<rdf:Description rdf:about=\"{}\"><{tag}{ns} rdf:resource=\"{}\"/></rdf:Description>", esc(&subject.0), esc(&object.0))
        }
        Axiom::DataPropertyAssertion(p, a, l) => {
            let (tag, ns) = element_name(p)?;
            format!("<rdf:Description rdf:about=\"{}\"><{tag}{ns} {}</{tag}></rdf:Description>", esc(&a.0), lit(l))
        }
        Axiom::SameIndividual(v) => v[1..].iter().map(|y| format!("<rdf:Description rdf:about=\"{}\">{}</rdf:Description>", esc(&v[0].0), res("owl:sameAs", &y.0))).collect(),
        Axiom::DifferentIndividuals(v) => {
            let mut out = String::new();
            for i in 0..v.len() {
                for j in (i + 1)..v.len() {
                    out.push_str(&format!("<rdf:Description rdf:about=\"{}\">{}</rdf:Description>", esc(&v[i].0), res("owl:differentFrom", &v[j].0)));
                }
            }
            out
        }
        Axiom::AnnotationAssertion(p, s, t) => {
            let (tag, ns) = element_name(p)?;
            format!("<rdf:Description rdf:about=\"{}\"><{tag}{ns}>{}</{tag}></rdf:Description>", esc(&s.0), esc(t))
        }
    })
}

/// Render the ontology as RDF/XML.
pub fn export(o: &Ontology) -> Result<String, ExportError> {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str(&format!("<rdf:RDF xmlns:rdf=\"{NS_RDF}\" xmlns:rdfs=\"http://www.w3.org/2000/01/rdf-schema#\" xmlns:owl=\"{NS_OWL}\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema#\">\n"));
    match &o.iri {
        Some(i) => {
            s.push_str(&format!("  <owl:Ontology rdf:about=\"{}\">", esc(&i.0)));
            if let Some(v) = &o.version_iri {
                s.push_str(&res("owl:versionIRI", &v.0));
            }
            s.push_str("</owl:Ontology>\n");
        }
        None => s.push_str("  <owl:Ontology/>\n"),
    }
    for a in &o.axioms {
        s.push_str("  ");
        s.push_str(&axiom(a)?);
        s.push('\n');
    }
    s.push_str("</rdf:RDF>\n");
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functional::parse;

    fn export_of(axioms: &str) -> String {
        let doc = format!("Prefix(ex:=<http://e/#>)\nOntology(<http://e>\n{axioms}\n)");
        export(&parse(&doc).unwrap()).unwrap()
    }

    #[test]
    fn the_document_is_wrapped_and_the_ontology_declared() {
        let x = export_of("Declaration(Class(ex:A))");
        assert!(x.starts_with("<?xml") && x.contains("<owl:Ontology rdf:about=\"http://e\">") && x.contains("<owl:Class rdf:about=\"http://e/#A\"/>") && x.trim_end().ends_with("</rdf:RDF>"));
    }

    #[test]
    fn class_expressions_follow_the_owl_rdf_mapping() {
        let x = export_of("SubClassOf(ex:A ObjectIntersectionOf(ex:B ObjectSomeValuesFrom(ex:r ex:C) ObjectAllValuesFrom(ObjectInverseOf(ex:r) ex:D)))
                           SubClassOf(ex:A ObjectMinCardinality(2 ex:r ex:B)) SubClassOf(ex:A ObjectMaxCardinality(1 ex:r)) SubClassOf(ex:A ObjectExactCardinality(3 ex:r ex:B))
                           SubClassOf(ex:A ObjectComplementOf(ex:B)) SubClassOf(ex:A ObjectUnionOf(ex:B ex:C))");
        for needle in [
            "<rdfs:subClassOf><owl:Class><owl:intersectionOf rdf:parseType=\"Collection\">",
            "<owl:someValuesFrom><owl:Class rdf:about=\"http://e/#C\"></owl:Class></owl:someValuesFrom>",
            "<owl:onProperty><rdf:Description><owl:inverseOf rdf:resource=\"http://e/#r\"/></rdf:Description></owl:onProperty>",
            "<owl:minQualifiedCardinality rdf:datatype=\"http://www.w3.org/2001/XMLSchema#nonNegativeInteger\">2</owl:minQualifiedCardinality><owl:onClass>",
            "<owl:maxCardinality rdf:datatype=\"http://www.w3.org/2001/XMLSchema#nonNegativeInteger\">1</owl:maxCardinality>",
            "<owl:qualifiedCardinality rdf:datatype=\"http://www.w3.org/2001/XMLSchema#nonNegativeInteger\">3</owl:qualifiedCardinality>",
            "<owl:complementOf>",
            "<owl:unionOf rdf:parseType=\"Collection\">",
        ] {
            assert!(x.contains(needle), "missing {needle}\n{x}");
        }
    }

    #[test]
    fn property_axioms_and_individuals_export() {
        let x = export_of("InverseObjectProperties(ex:p ex:q) TransitiveObjectProperty(ex:p) FunctionalObjectProperty(ObjectInverseOf(ex:g))
                           ObjectPropertyDomain(ex:p ex:A) SubObjectPropertyOf(ex:p ex:q)
                           ClassAssertion(ex:A ex:a) ObjectPropertyAssertion(ex:p ex:a ex:b) ObjectPropertyAssertion(ObjectInverseOf(ex:p) ex:a ex:b)
                           DataPropertyAssertion(ex:age ex:a \"3\"^^xsd:integer) SameIndividual(ex:a ex:c) DifferentIndividuals(ex:a ex:b ex:d)
                           AnnotationAssertion(rdfs:label ex:A \"A & B\")");
        for needle in [
            "<owl:inverseOf rdf:resource=\"http://e/#q\"/>",
            "<rdf:type rdf:resource=\"http://www.w3.org/2002/07/owl#TransitiveProperty\"/>",
            "<owl:ObjectProperty rdf:about=\"http://e/#g\"><rdf:type rdf:resource=\"http://www.w3.org/2002/07/owl#InverseFunctionalProperty\"/></owl:ObjectProperty>",
            "<rdfs:domain>",
            "<rdf:Description rdf:about=\"http://e/#a\"><p:p xmlns:p=\"http://e/#\" rdf:resource=\"http://e/#b\"/></rdf:Description>",
            "<rdf:Description rdf:about=\"http://e/#b\"><p:p xmlns:p=\"http://e/#\" rdf:resource=\"http://e/#a\"/></rdf:Description>",
            "rdf:datatype=\"http://www.w3.org/2001/XMLSchema#integer\">3</p:age>",
            "owl:sameAs",
            "<owl:differentFrom rdf:resource=\"http://e/#d\"/>",
            "A &amp; B",
        ] {
            assert!(x.contains(needle), "missing {needle}\n{x}");
        }
        assert_eq!(x.matches("<owl:differentFrom").count(), 3, "three individuals give three pairs");
    }

    #[test]
    fn nominals_and_data_restrictions_follow_the_owl_rdf_mapping() {
        let x = export_of("EquivalentClasses(ex:Colour ObjectOneOf(ex:red ex:green)) SubClassOf(ex:Rose ObjectHasValue(ex:hasColour ex:red))
                           SubClassOf(ex:Adult DataSomeValuesFrom(ex:age DatatypeRestriction(xsd:integer xsd:minInclusive \"18\"^^xsd:integer)))
                           SubClassOf(ex:P DataMaxCardinality(1 ex:age)) SubClassOf(ex:Q DataMinCardinality(2 ex:tag xsd:string))
                           SubClassOf(ex:F DataHasValue(ex:code \"A1\")) DataPropertyRange(ex:size DataUnionOf(DataOneOf(\"s\" \"l\") DataComplementOf(xsd:boolean)))
                           FunctionalDataProperty(ex:code) SubDataPropertyOf(ex:a ex:b)");
        for needle in [
            "<owl:oneOf rdf:parseType=\"Collection\"><owl:NamedIndividual rdf:about=\"http://e/#green\"/>",
            "<owl:hasValue rdf:resource=\"http://e/#red\"/>",
            "<owl:onProperty><owl:DatatypeProperty rdf:about=\"http://e/#age\"/></owl:onProperty><owl:someValuesFrom><rdfs:Datatype><owl:onDatatype rdf:resource=\"http://www.w3.org/2001/XMLSchema#integer\"/>",
            "<xsd:minInclusive rdf:datatype=\"http://www.w3.org/2001/XMLSchema#integer\">18</xsd:minInclusive>",
            "<owl:maxCardinality rdf:datatype=\"http://www.w3.org/2001/XMLSchema#nonNegativeInteger\">1</owl:maxCardinality>",
            "<owl:minQualifiedCardinality rdf:datatype=\"http://www.w3.org/2001/XMLSchema#nonNegativeInteger\">2</owl:minQualifiedCardinality><owl:onDataRange>",
            "<owl:hasValue rdf:datatype=\"http://www.w3.org/2001/XMLSchema#string\">A1</owl:hasValue>",
            "<owl:datatypeComplementOf><rdfs:Datatype rdf:about=\"http://www.w3.org/2001/XMLSchema#boolean\"/></owl:datatypeComplementOf>",
            "<rdf:first rdf:datatype=\"http://www.w3.org/2001/XMLSchema#string\">s</rdf:first>",
            "<rdfs:subPropertyOf rdf:resource=\"http://e/#b\"/>",
        ] {
            assert!(x.contains(needle), "missing {needle}\n{x}");
        }
    }

    #[test]
    fn a_property_iri_that_is_not_an_xml_name_is_a_clear_error() {
        let o = parse("Ontology(<http://e> ObjectPropertyAssertion(<http://e/1p> <http://e/a> <http://e/b>))").unwrap();
        assert_eq!(export(&o).unwrap_err(), ExportError::NotAnElementName("http://e/1p".into()));
    }
}
