// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The XIAR OWL profile as a data model. Names follow the W3C OWL 2 Structural Specification (class expressions, object
//! property expressions, axioms), restricted to what XIAR's ontologies use:
//!
//! * classes, object properties and data properties with domain and range, individuals;
//! * `SubClassOf`, `EquivalentClasses`, `DisjointClasses`, `SubObjectPropertyOf`, `EquivalentObjectProperties`,
//!   `InverseObjectProperties`, property characteristics (functional, inverse functional, transitive, symmetric);
//! * existential, universal and cardinality restrictions, intersection, union, complement;
//! * nominals (`ObjectOneOf`, `ObjectHasValue`);
//! * data properties: restrictions (some, all, has-value, cardinalities) over data ranges (datatypes, `DataOneOf`,
//!   complement, intersection, union, numeric facets), `FunctionalDataProperty`, sub- and equivalent data properties;
//! * property characteristics also reflexive, irreflexive, asymmetric, and `DisjointObjectProperties`;
//! * class/property assertions, negative property assertions, same/different individuals, annotations.
//!
//! Anything else is rejected by the reader with a clear error, never silently ignored.

use std::collections::BTreeSet;

/// An IRI in full form (prefixes are expanded by the reader and re-applied by the writer).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Iri(pub String);

impl Iri {
    pub fn new(s: impl Into<String>) -> Self {
        Iri(s.into())
    }

    /// The part after the last `#` or `/`, used for display and naming checks.
    pub fn local(&self) -> &str {
        self.0.rsplit(['#', '/']).next().unwrap_or(&self.0)
    }
}

impl std::fmt::Display for Iri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectProp {
    Named(Iri),
    Inverse(Iri),
}

impl ObjectProp {
    pub fn named(&self) -> &Iri {
        match self {
            ObjectProp::Named(i) | ObjectProp::Inverse(i) => i,
        }
    }

    pub fn inverse(&self) -> ObjectProp {
        match self {
            ObjectProp::Named(i) => ObjectProp::Inverse(i.clone()),
            ObjectProp::Inverse(i) => ObjectProp::Named(i.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ClassExpr {
    Thing,
    Nothing,
    Class(Iri),
    And(Vec<ClassExpr>),
    Or(Vec<ClassExpr>),
    Not(Box<ClassExpr>),
    Some(ObjectProp, Box<ClassExpr>),
    All(ObjectProp, Box<ClassExpr>),
    Min(u32, ObjectProp, Box<ClassExpr>),
    Max(u32, ObjectProp, Box<ClassExpr>),
    Exact(u32, ObjectProp, Box<ClassExpr>),
    /// `ObjectOneOf`: exactly the listed individuals (at least one)
    OneOf(Vec<Iri>),
    /// `ObjectHasValue`: some `p`-neighbour is the individual
    HasValue(ObjectProp, Iri),
    /// `ObjectHasSelf`: the individual is `p`-related to itself
    HasSelf(ObjectProp),
    /// Internal to the reasoner, never read or written: `∀B^q.C`, the value restriction of a role with property chains in
    /// state `q` of automaton `B` (see [`crate::chains`]).
    AllState(u32, u32, Box<ClassExpr>),
    DataSome(Iri, DataRange),
    DataAll(Iri, DataRange),
    DataHasValue(Iri, Literal),
    DataMin(u32, Iri, DataRange),
    DataMax(u32, Iri, DataRange),
    DataExact(u32, Iri, DataRange),
}

impl ClassExpr {
    pub fn class(iri: &str) -> Self {
        ClassExpr::Class(Iri::new(iri))
    }

    /// Every named class mentioned.
    pub fn classes(&self, out: &mut BTreeSet<Iri>) {
        match self {
            ClassExpr::Class(i) => {
                out.insert(i.clone());
            }
            ClassExpr::And(v) | ClassExpr::Or(v) => v.iter().for_each(|c| c.classes(out)),
            ClassExpr::Not(c) | ClassExpr::Some(_, c) | ClassExpr::All(_, c) | ClassExpr::Min(_, _, c) | ClassExpr::Max(_, _, c) | ClassExpr::Exact(_, _, c) | ClassExpr::AllState(_, _, c) => c.classes(out),
            ClassExpr::HasValue(..)
            | ClassExpr::HasSelf(_)
            | ClassExpr::OneOf(_)
            | ClassExpr::DataSome(..)
            | ClassExpr::DataAll(..)
            | ClassExpr::DataHasValue(..)
            | ClassExpr::DataMin(..)
            | ClassExpr::DataMax(..)
            | ClassExpr::DataExact(..) => {}
            ClassExpr::Thing | ClassExpr::Nothing => {}
        }
    }

    /// Every object property mentioned.
    pub fn properties(&self, out: &mut BTreeSet<Iri>) {
        match self {
            ClassExpr::And(v) | ClassExpr::Or(v) => v.iter().for_each(|c| c.properties(out)),
            ClassExpr::Not(c) => c.properties(out),
            ClassExpr::Some(p, c) | ClassExpr::All(p, c) | ClassExpr::Min(_, p, c) | ClassExpr::Max(_, p, c) | ClassExpr::Exact(_, p, c) => {
                out.insert(p.named().clone());
                c.properties(out);
            }
            ClassExpr::HasValue(p, _) | ClassExpr::HasSelf(p) => {
                out.insert(p.named().clone());
            }
            _ => {}
        }
    }

    /// Every data property mentioned.
    pub fn data_properties(&self, out: &mut BTreeSet<Iri>) {
        match self {
            ClassExpr::And(v) | ClassExpr::Or(v) => v.iter().for_each(|c| c.data_properties(out)),
            ClassExpr::Not(c) | ClassExpr::Some(_, c) | ClassExpr::All(_, c) | ClassExpr::Min(_, _, c) | ClassExpr::Max(_, _, c) | ClassExpr::Exact(_, _, c) => c.data_properties(out),
            ClassExpr::DataSome(p, _) | ClassExpr::DataAll(p, _) | ClassExpr::DataHasValue(p, _) | ClassExpr::DataMin(_, p, _) | ClassExpr::DataMax(_, p, _) | ClassExpr::DataExact(_, p, _) => {
                out.insert(p.clone());
            }
            _ => {}
        }
    }

    /// Every individual named inside (nominals).
    pub fn individuals(&self, out: &mut BTreeSet<Iri>) {
        match self {
            ClassExpr::OneOf(v) => out.extend(v.iter().cloned()),
            ClassExpr::HasValue(_, i) => {
                out.insert(i.clone());
            }
            ClassExpr::And(v) | ClassExpr::Or(v) => v.iter().for_each(|c| c.individuals(out)),
            ClassExpr::Not(c) | ClassExpr::Some(_, c) | ClassExpr::All(_, c) | ClassExpr::Min(_, _, c) | ClassExpr::Max(_, _, c) | ClassExpr::Exact(_, _, c) => c.individuals(out),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Datatype {
    String,
    Boolean,
    DateTime,
    /// all decimal numbers (the integers are among them)
    Decimal,
    Integer,
    NonNegativeInteger,
    PositiveInteger,
    NonPositiveInteger,
    NegativeInteger,
    Long,
    Int,
    Short,
    Byte,
    /// `xsd:float` and `xsd:double` share one value space here (real numbers, disjoint from the decimals)
    Float,
    Double,
}

impl Datatype {
    pub const ALL: [(Datatype, &'static str); 15] = [
        (Datatype::String, "http://www.w3.org/2001/XMLSchema#string"),
        (Datatype::Integer, "http://www.w3.org/2001/XMLSchema#integer"),
        (Datatype::Decimal, "http://www.w3.org/2001/XMLSchema#decimal"),
        (Datatype::Boolean, "http://www.w3.org/2001/XMLSchema#boolean"),
        (Datatype::DateTime, "http://www.w3.org/2001/XMLSchema#dateTime"),
        (Datatype::NonNegativeInteger, "http://www.w3.org/2001/XMLSchema#nonNegativeInteger"),
        (Datatype::PositiveInteger, "http://www.w3.org/2001/XMLSchema#positiveInteger"),
        (Datatype::NonPositiveInteger, "http://www.w3.org/2001/XMLSchema#nonPositiveInteger"),
        (Datatype::NegativeInteger, "http://www.w3.org/2001/XMLSchema#negativeInteger"),
        (Datatype::Long, "http://www.w3.org/2001/XMLSchema#long"),
        (Datatype::Int, "http://www.w3.org/2001/XMLSchema#int"),
        (Datatype::Short, "http://www.w3.org/2001/XMLSchema#short"),
        (Datatype::Byte, "http://www.w3.org/2001/XMLSchema#byte"),
        (Datatype::Float, "http://www.w3.org/2001/XMLSchema#float"),
        (Datatype::Double, "http://www.w3.org/2001/XMLSchema#double"),
    ];

    pub fn iri(&self) -> &'static str {
        Self::ALL.iter().find(|(d, _)| d == self).map(|(_, i)| *i).unwrap_or("")
    }

    pub fn from_iri(i: &str) -> Option<Datatype> {
        Self::ALL.iter().find(|(_, s)| *s == i).map(|(d, _)| *d)
    }

    /// The integer datatypes (`xsd:integer` and everything derived from it here).
    pub fn is_integer(&self) -> bool {
        !matches!(self, Datatype::String | Datatype::Boolean | Datatype::DateTime | Datatype::Decimal | Datatype::Float | Datatype::Double)
    }

    /// Everything numeric: integers, decimals and the real-number types.
    pub fn is_numeric(&self) -> bool {
        !matches!(self, Datatype::String | Datatype::Boolean | Datatype::DateTime)
    }
}

pub const RDFS_LITERAL: &str = "http://www.w3.org/2000/01/rdf-schema#Literal";

/// The constraining facets XIAR supports on numeric datatypes (`xsd:minInclusive` and its three siblings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Facet {
    MinInclusive,
    MaxInclusive,
    MinExclusive,
    MaxExclusive,
}

impl Facet {
    pub const ALL: [(Facet, &'static str); 4] = [
        (Facet::MinInclusive, "http://www.w3.org/2001/XMLSchema#minInclusive"),
        (Facet::MaxInclusive, "http://www.w3.org/2001/XMLSchema#maxInclusive"),
        (Facet::MinExclusive, "http://www.w3.org/2001/XMLSchema#minExclusive"),
        (Facet::MaxExclusive, "http://www.w3.org/2001/XMLSchema#maxExclusive"),
    ];

    pub fn iri(&self) -> &'static str {
        Self::ALL.iter().find(|(f, _)| f == self).map(|(_, i)| *i).unwrap_or("")
    }

    pub fn from_iri(i: &str) -> Option<Facet> {
        Self::ALL.iter().find(|(_, s)| *s == i).map(|(f, _)| *f)
    }
}

/// A data range (OWL 2 `DataRange`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataRange {
    /// `rdfs:Literal`: every data value
    Top,
    Datatype(Datatype),
    OneOf(Vec<Literal>),
    Not(Box<DataRange>),
    And(Vec<DataRange>),
    Or(Vec<DataRange>),
    /// a numeric datatype narrowed by facets
    Facets(Datatype, Vec<(Facet, Literal)>),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Literal {
    pub lexical: String,
    pub datatype: Datatype,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axiom {
    DeclareClass(Iri),
    DeclareObjectProperty(Iri),
    DeclareDataProperty(Iri),
    DeclareIndividual(Iri),
    SubClassOf(ClassExpr, ClassExpr),
    EquivalentClasses(Vec<ClassExpr>),
    DisjointClasses(Vec<ClassExpr>),
    /// `HasKey(C (p1 … pn) (d1 … dm))`: two named instances of `C` that share a named `pi`-neighbour and a `dj` value for
    /// every key property are the same individual
    HasKey(ClassExpr, Vec<ObjectProp>, Vec<Iri>),
    /// `DatatypeDefinition(D range)`: `D` names the data range; the reader replaces every use of `D` by the range
    DatatypeDefinition(Iri, DataRange),
    /// `DisjointUnion(A C1 … Cn)`: `A` is the union of the `Ci`, which are pairwise disjoint
    DisjointUnion(Iri, Vec<ClassExpr>),
    SubObjectPropertyOf(ObjectProp, ObjectProp),
    /// `SubObjectPropertyOf(ObjectPropertyChain(S1 … Sn) R)`: a path of `S1` … `Sn` edges is an `R` edge (`n ≥ 2`)
    SubPropertyChain(Vec<ObjectProp>, ObjectProp),
    EquivalentObjectProperties(Vec<ObjectProp>),
    InverseObjectProperties(Iri, Iri),
    ObjectPropertyDomain(ObjectProp, ClassExpr),
    ObjectPropertyRange(ObjectProp, ClassExpr),
    FunctionalObjectProperty(ObjectProp),
    InverseFunctionalObjectProperty(ObjectProp),
    TransitiveObjectProperty(ObjectProp),
    SymmetricObjectProperty(ObjectProp),
    ReflexiveObjectProperty(ObjectProp),
    IrreflexiveObjectProperty(ObjectProp),
    AsymmetricObjectProperty(ObjectProp),
    DisjointObjectProperties(Vec<ObjectProp>),
    DataPropertyDomain(Iri, ClassExpr),
    DataPropertyRange(Iri, DataRange),
    FunctionalDataProperty(Iri),
    SubDataPropertyOf(Iri, Iri),
    EquivalentDataProperties(Vec<Iri>),
    /// no individual has the same value for two of these data properties
    DisjointDataProperties(Vec<Iri>),
    ClassAssertion(ClassExpr, Iri),
    ObjectPropertyAssertion(ObjectProp, Iri, Iri),
    DataPropertyAssertion(Iri, Iri, Literal),
    /// `NegativeObjectPropertyAssertion(p a b)`: `a` is not `p`-related to `b`
    NegativeObjectPropertyAssertion(ObjectProp, Iri, Iri),
    NegativeDataPropertyAssertion(Iri, Iri, Literal),
    SameIndividual(Vec<Iri>),
    DifferentIndividuals(Vec<Iri>),
    /// annotation property (e.g. `rdfs:label`), the annotated IRI, and the text
    AnnotationAssertion(Iri, Iri, String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ontology {
    pub iri: Option<Iri>,
    pub version_iri: Option<Iri>,
    /// declared prefixes (name without the colon, namespace), kept so output reads like the input
    pub prefixes: Vec<(String, String)>,
    pub axioms: Vec<Axiom>,
}

pub const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
pub const RDFS_COMMENT: &str = "http://www.w3.org/2000/01/rdf-schema#comment";
pub const OWL_THING: &str = "http://www.w3.org/2002/07/owl#Thing";
pub const OWL_NOTHING: &str = "http://www.w3.org/2002/07/owl#Nothing";

impl Axiom {
    /// Every class expression the axiom contains.
    pub fn class_exprs(&self) -> Vec<&ClassExpr> {
        match self {
            Axiom::SubClassOf(a, b) => vec![a, b],
            Axiom::EquivalentClasses(v) | Axiom::DisjointClasses(v) | Axiom::DisjointUnion(_, v) => v.iter().collect(),
            Axiom::HasKey(c, _, _) => vec![c],
            Axiom::ObjectPropertyDomain(_, c) | Axiom::ObjectPropertyRange(_, c) | Axiom::DataPropertyDomain(_, c) | Axiom::ClassAssertion(c, _) => vec![c],
            _ => Vec::new(),
        }
    }
}

impl Ontology {
    /// Does any axiom use a nominal (`ObjectOneOf`, `ObjectHasValue`)?
    pub fn uses_nominals(&self) -> bool {
        self.axioms.iter().any(|a| a.class_exprs().into_iter().any(|c| {
            let mut s = BTreeSet::new();
            c.individuals(&mut s);
            !s.is_empty()
        }))
    }

    /// Named classes declared or mentioned anywhere.
    pub fn class_names(&self) -> BTreeSet<Iri> {
        let mut out = BTreeSet::new();
        let mut add = |c: &ClassExpr| c.classes(&mut out);
        for a in &self.axioms {
            match a {
                Axiom::DeclareClass(i) => {
                    add(&ClassExpr::Class(i.clone()));
                }
                Axiom::SubClassOf(a, b) => {
                    add(a);
                    add(b);
                }
                Axiom::EquivalentClasses(v) | Axiom::DisjointClasses(v) => v.iter().for_each(&mut add),
                Axiom::HasKey(c, _, _) => add(c),
                Axiom::DisjointUnion(a, v) => {
                    add(&ClassExpr::Class(a.clone()));
                    v.iter().for_each(&mut add);
                }
                Axiom::ObjectPropertyDomain(_, c) | Axiom::ObjectPropertyRange(_, c) | Axiom::DataPropertyDomain(_, c) | Axiom::ClassAssertion(c, _) => add(c),
                _ => {}
            }
        }
        out
    }

    pub fn object_property_names(&self) -> BTreeSet<Iri> {
        let mut out = BTreeSet::new();
        for a in &self.axioms {
            match a {
                Axiom::DeclareObjectProperty(i) => {
                    out.insert(i.clone());
                }
                Axiom::HasKey(c, ps, _) => {
                    c.properties(&mut out);
                    out.extend(ps.iter().map(|p| p.named().clone()));
                }
                Axiom::SubPropertyChain(w, t) => {
                    out.extend(w.iter().map(|p| p.named().clone()));
                    out.insert(t.named().clone());
                }
                Axiom::SubClassOf(a, b) => {
                    a.properties(&mut out);
                    b.properties(&mut out);
                }
                Axiom::EquivalentClasses(v) | Axiom::DisjointClasses(v) | Axiom::DisjointUnion(_, v) => v.iter().for_each(|c| c.properties(&mut out)),
                Axiom::SubObjectPropertyOf(p, q) => {
                    out.insert(p.named().clone());
                    out.insert(q.named().clone());
                }
                Axiom::EquivalentObjectProperties(v) => v.iter().for_each(|p| {
                    out.insert(p.named().clone());
                }),
                Axiom::InverseObjectProperties(p, q) => {
                    out.insert(p.clone());
                    out.insert(q.clone());
                }
                Axiom::ObjectPropertyDomain(p, c) | Axiom::ObjectPropertyRange(p, c) => {
                    out.insert(p.named().clone());
                    c.properties(&mut out);
                }
                Axiom::FunctionalObjectProperty(p) | Axiom::InverseFunctionalObjectProperty(p) | Axiom::TransitiveObjectProperty(p) | Axiom::SymmetricObjectProperty(p) | Axiom::ReflexiveObjectProperty(p) | Axiom::IrreflexiveObjectProperty(p) | Axiom::AsymmetricObjectProperty(p) | Axiom::ObjectPropertyAssertion(p, _, _) | Axiom::NegativeObjectPropertyAssertion(p, _, _) => {
                    out.insert(p.named().clone());
                }
                Axiom::DisjointObjectProperties(v) => v.iter().for_each(|p| {
                    out.insert(p.named().clone());
                }),
                Axiom::ClassAssertion(c, _) => c.properties(&mut out),
                _ => {}
            }
        }
        out
    }

    pub fn data_property_names(&self) -> BTreeSet<Iri> {
        let mut out = BTreeSet::new();
        for a in &self.axioms {
            for c in a.class_exprs() {
                c.data_properties(&mut out);
            }
            match a {
                Axiom::DeclareDataProperty(i) | Axiom::DataPropertyDomain(i, _) | Axiom::DataPropertyRange(i, _) | Axiom::FunctionalDataProperty(i) | Axiom::DataPropertyAssertion(i, _, _) | Axiom::NegativeDataPropertyAssertion(i, _, _) => {
                    out.insert(i.clone());
                }
                Axiom::SubDataPropertyOf(p, q) => {
                    out.insert(p.clone());
                    out.insert(q.clone());
                }
                Axiom::EquivalentDataProperties(v) | Axiom::DisjointDataProperties(v) => out.extend(v.iter().cloned()),
                Axiom::HasKey(_, _, ds) => out.extend(ds.iter().cloned()),
                _ => {}
            }
        }
        out
    }

    /// The individuals that class expressions refer to (nominals), as opposed to those that only appear in assertions.
    pub fn nominal_names(&self) -> BTreeSet<Iri> {
        let mut out = BTreeSet::new();
        for a in &self.axioms {
            for c in a.class_exprs() {
                c.individuals(&mut out);
            }
        }
        out
    }

    pub fn individual_names(&self) -> BTreeSet<Iri> {
        let mut out = BTreeSet::new();
        for a in &self.axioms {
            for c in a.class_exprs() {
                c.individuals(&mut out);
            }
            match a {
                Axiom::DeclareIndividual(i) | Axiom::ClassAssertion(_, i) | Axiom::DataPropertyAssertion(_, i, _) | Axiom::NegativeDataPropertyAssertion(_, i, _) => {
                    out.insert(i.clone());
                }
                Axiom::ObjectPropertyAssertion(_, a, b) | Axiom::NegativeObjectPropertyAssertion(_, a, b) => {
                    out.insert(a.clone());
                    out.insert(b.clone());
                }
                Axiom::SameIndividual(v) | Axiom::DifferentIndividuals(v) => out.extend(v.iter().cloned()),
                _ => {}
            }
        }
        out
    }

    /// The rdfs:label of `iri`, if annotated.
    pub fn label(&self, iri: &Iri) -> Option<&str> {
        self.axioms.iter().find_map(|a| match a {
            Axiom::AnnotationAssertion(p, s, t) if p.0 == RDFS_LABEL && s == iri => Some(t.as_str()),
            _ => None,
        })
    }
}
