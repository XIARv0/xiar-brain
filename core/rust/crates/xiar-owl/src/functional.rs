// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! Reader and writer for OWL 2 Functional-Style Syntax, the canonical on-disk format of XIAR ontologies.
//!
//! The reader is strict: a construct outside the XIAR profile (property chains, asymmetric/reflexive properties, axiom
//! annotations, ...) is an error that names the construct and its position, never silently dropped.

use crate::model::*;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("line {line}: {msg}")]
    Syntax { line: usize, msg: String },
    #[error("line {line}: `{construct}` is outside the XIAR OWL profile ({why})")]
    OutsideProfile { line: usize, construct: String, why: &'static str },
    #[error("line {line}: unknown prefix `{prefix}:`")]
    UnknownPrefix { line: usize, prefix: String },
    #[error("not valid OWL 2 DL: {0}")]
    NotDl(String),
}

type Res<T> = Result<T, ParseError>;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Open,
    /// an opening parenthesis after white space: the start of a bare list, unless a keyword stands before it
    SpOpen,
    Close,
    Eq,
    Iri(String),
    Name(String),
    Str(String),
    Hat,
}

fn lex(src: &str) -> Res<Vec<(Tok, usize)>> {
    let b: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut line = 1;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            c if c.is_whitespace() => i += 1,
            '#' => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            '(' => {
                out.push((if i > 0 && b[i - 1].is_whitespace() { Tok::SpOpen } else { Tok::Open }, line));
                i += 1;
            }
            ')' => {
                out.push((Tok::Close, line));
                i += 1;
            }
            '=' => {
                out.push((Tok::Eq, line));
                i += 1;
            }
            '^' if b.get(i + 1) == Some(&'^') => {
                out.push((Tok::Hat, line));
                i += 2;
            }
            '<' => {
                let start = i + 1;
                let mut j = start;
                while j < b.len() && b[j] != '>' && b[j] != '\n' {
                    j += 1;
                }
                if j >= b.len() || b[j] != '>' {
                    return Err(ParseError::Syntax { line, msg: "unterminated <IRI>".into() });
                }
                if b[start..j].iter().any(|c| c.is_whitespace()) {
                    return Err(ParseError::Syntax { line, msg: "an <IRI> cannot contain spaces".into() });
                }
                out.push((Tok::Iri(b[start..j].iter().collect()), line));
                i = j + 1;
            }
            '"' => {
                let mut s = String::new();
                let mut j = i + 1;
                loop {
                    match b.get(j) {
                        None => return Err(ParseError::Syntax { line, msg: "unterminated string".into() }),
                        Some('"') => break,
                        Some('\\') => match b.get(j + 1) {
                            Some('"') => {
                                s.push('"');
                                j += 2;
                            }
                            Some('\\') => {
                                s.push('\\');
                                j += 2;
                            }
                            Some('n') => {
                                s.push('\n');
                                j += 2;
                            }
                            _ => return Err(ParseError::Syntax { line, msg: "unsupported escape in string".into() }),
                        },
                        Some('\n') => {
                            line += 1;
                            s.push('\n');
                            j += 1;
                        }
                        Some(ch) => {
                            s.push(*ch);
                            j += 1;
                        }
                    }
                }
                out.push((Tok::Str(s), line));
                i = j + 1;
            }
            '@' => return Err(ParseError::OutsideProfile { line, construct: "language tag".into(), why: "plain and xsd-typed literals only" }),
            c if c.is_alphanumeric() || matches!(c, '_' | ':' | '-' | '.' | '+') => {
                let start = i;
                while i < b.len() && (b[i].is_alphanumeric() || matches!(b[i], '_' | ':' | '-' | '.' | '+')) {
                    i += 1;
                }
                out.push((Tok::Name(b[start..i].iter().collect()), line));
            }
            other => return Err(ParseError::Syntax { line, msg: format!("unexpected character {other:?}") }),
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Call(String, Vec<Node>, usize),
    Iri(String, usize),
    Name(String, usize),
    Lit(String, Option<Box<Node>>, usize),
}

impl Node {
    fn line(&self) -> usize {
        match self {
            Node::Call(_, _, l) | Node::Iri(_, l) | Node::Name(_, l) | Node::Lit(_, _, l) => *l,
        }
    }
}

struct P {
    toks: Vec<(Tok, usize)>,
    pos: usize,
}

impl P {
    fn node(&mut self) -> Res<Node> {
        let Some((t, line)) = self.toks.get(self.pos).cloned() else { return Err(ParseError::Syntax { line: self.toks.last().map_or(1, |x| x.1), msg: "unexpected end of input".into() }) };
        self.pos += 1;
        match t {
            Tok::Iri(s) => Ok(Node::Iri(s, line)),
            Tok::Str(s) => {
                if matches!(self.toks.get(self.pos), Some((Tok::Hat, _))) {
                    self.pos += 1;
                    let dt = self.node()?;
                    return Ok(Node::Lit(s, Some(Box::new(dt)), line));
                }
                Ok(Node::Lit(s, None, line))
            }
            Tok::Name(n) => {
                // `Name(` is a call; `Name (` too when the name is a keyword, while an IRI before `(` leaves a list for the next argument
                let call = match self.toks.get(self.pos) {
                    Some((Tok::Open, _)) => true,
                    Some((Tok::SpOpen, _)) => !n.contains(':'),
                    _ => false,
                };
                if call {
                    self.pos += 1;
                    let mut args = Vec::new();
                    loop {
                        match self.toks.get(self.pos) {
                            Some((Tok::Close, _)) => {
                                self.pos += 1;
                                break;
                            }
                            None => return Err(ParseError::Syntax { line, msg: format!("`{n}(` is never closed") }),
                            Some((Tok::Eq, l)) => {
                                // Prefix(name:=<iri>): keep the `=` out of the argument list by merging it into the name
                                let _ = l;
                                self.pos += 1;
                            }
                            _ => args.push(self.node()?),
                        }
                    }
                    Ok(Node::Call(n, args, line))
                } else {
                    Ok(Node::Name(n, line))
                }
            }
            // a bare parenthesised list, as in the key lists of `HasKey`
            Tok::Open | Tok::SpOpen => {
                let mut items = Vec::new();
                loop {
                    match self.toks.get(self.pos) {
                        Some((Tok::Close, _)) => {
                            self.pos += 1;
                            break;
                        }
                        None => return Err(ParseError::Syntax { line, msg: "`(` is never closed".into() }),
                        _ => items.push(self.node()?),
                    }
                }
                Ok(Node::Call(String::new(), items, line))
            }
            Tok::Close | Tok::Eq | Tok::Hat => Err(ParseError::Syntax { line, msg: "unexpected punctuation".into() }),
        }
    }
}

struct Ctx {
    prefixes: Vec<(String, String)>,
    /// named datatypes from `DatatypeDefinition`, already expanded
    datatypes: std::cell::RefCell<std::collections::BTreeMap<String, DataRange>>,
}

const DATATYPES: &str = "datatypes: string, boolean, dateTime, decimal, integer and its sub-types, float, double";

const BUILTIN: &[(&str, &str)] = &[("owl", "http://www.w3.org/2002/07/owl#"), ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"), ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"), ("xsd", "http://www.w3.org/2001/XMLSchema#")];

impl Ctx {
    fn expand(&self, n: &Node) -> Res<Iri> {
        match n {
            Node::Iri(s, _) => Ok(Iri(s.clone())),
            Node::Name(s, line) => {
                let Some((p, local)) = s.split_once(':') else { return Err(ParseError::Syntax { line: *line, msg: format!("expected an IRI or prefixed name, found `{s}`") }) };
                let ns = self.prefixes.iter().map(|(a, b)| (a.as_str(), b.as_str())).chain(BUILTIN.iter().copied()).find(|(a, _)| *a == p).map(|(_, b)| b);
                match ns {
                    Some(ns) => Ok(Iri(format!("{ns}{local}"))),
                    None => Err(ParseError::UnknownPrefix { line: *line, prefix: p.to_string() }),
                }
            }
            other => Err(ParseError::Syntax { line: other.line(), msg: "expected an IRI".into() }),
        }
    }

    fn outside<T>(&self, n: &Node, name: &str, why: &'static str) -> Res<T> {
        Err(ParseError::OutsideProfile { line: n.line(), construct: name.to_string(), why })
    }

    fn ope(&self, n: &Node) -> Res<ObjectProp> {
        match n {
            Node::Call(c, a, _) if c == "ObjectInverseOf" && a.len() == 1 => Ok(ObjectProp::Inverse(self.expand(&a[0])?)),
            Node::Call(c, _, _) => self.outside(n, c, "only named and inverse object properties"),
            _ => Ok(ObjectProp::Named(self.expand(n)?)),
        }
    }

    fn count(&self, n: &Node) -> Res<u32> {
        match n {
            Node::Name(s, line) => s.parse::<u32>().map_err(|_| ParseError::Syntax { line: *line, msg: format!("expected a cardinality number, found `{s}`") }),
            other => Err(ParseError::Syntax { line: other.line(), msg: "expected a cardinality number".into() }),
        }
    }

    /// A data range: a datatype, `rdfs:Literal`, or one of the data range constructors.
    fn dr(&self, n: &Node) -> Res<DataRange> {
        match n {
            Node::Call(name, a, line) => match name.as_str() {
                "DataOneOf" => {
                    if a.is_empty() {
                        return Err(ParseError::Syntax { line: *line, msg: "`DataOneOf` needs at least one literal".into() });
                    }
                    Ok(DataRange::OneOf(a.iter().map(|x| self.literal(x)).collect::<Res<Vec<_>>>()?))
                }
                "DataComplementOf" => {
                    if a.len() != 1 {
                        return Err(ParseError::Syntax { line: *line, msg: "`DataComplementOf` takes one data range".into() });
                    }
                    Ok(DataRange::Not(Box::new(self.dr(&a[0])?)))
                }
                "DataIntersectionOf" | "DataUnionOf" => {
                    if a.len() < 2 {
                        return Err(ParseError::Syntax { line: *line, msg: format!("`{name}` needs at least two data ranges") });
                    }
                    let v = a.iter().map(|x| self.dr(x)).collect::<Res<Vec<_>>>()?;
                    Ok(if name == "DataIntersectionOf" { DataRange::And(v) } else { DataRange::Or(v) })
                }
                "DatatypeRestriction" => {
                    if a.len() < 3 || a.len() % 2 == 0 {
                        return Err(ParseError::Syntax { line: *line, msg: "`DatatypeRestriction` takes a datatype and facet/literal pairs".into() });
                    }
                    let base = self.expand(&a[0])?;
                    // a defined datatype that is itself a numeric restriction takes the new facets on top of its own
                    let (dt, mut facets) = match self.datatypes.borrow().get(&base.0) {
                        Some(DataRange::Facets(t, fs)) => (*t, fs.clone()),
                        Some(DataRange::Datatype(t)) => (*t, Vec::new()),
                        Some(_) => return self.outside(n, "DatatypeRestriction", "facets on a defined datatype that is not a numeric restriction"),
                        None => (Datatype::from_iri(&base.0).ok_or(ParseError::OutsideProfile { line: *line, construct: base.0.clone(), why: DATATYPES })?, Vec::new()),
                    };
                    if !dt.is_numeric() {
                        return self.outside(n, "DatatypeRestriction", "facets are supported on numeric datatypes only");
                    }
                    for pair in a[1..].chunks(2) {
                        let f = self.expand(&pair[0])?;
                        let facet = Facet::from_iri(&f.0).ok_or(ParseError::OutsideProfile { line: pair[0].line(), construct: f.0.clone(), why: "facets: minInclusive, maxInclusive, minExclusive, maxExclusive" })?;
                        facets.push((facet, self.literal(&pair[1])?));
                    }
                    Ok(DataRange::Facets(dt, facets))
                }
                other => self.outside(n, other, "not a data range XIAR supports"),
            },
            _ => {
                let i = self.expand(n)?;
                if i.0 == RDFS_LITERAL {
                    return Ok(DataRange::Top);
                }
                if let Some(d) = self.datatypes.borrow().get(&i.0) {
                    return Ok(d.clone());
                }
                Datatype::from_iri(&i.0).map(DataRange::Datatype).ok_or(ParseError::OutsideProfile { line: n.line(), construct: i.0.clone(), why: DATATYPES })
            }
        }
    }

    fn ce(&self, n: &Node) -> Res<ClassExpr> {
        match n {
            Node::Call(name, a, line) => {
                let need = |k: usize| -> Res<()> {
                    if a.len() == k { Ok(()) } else { Err(ParseError::Syntax { line: *line, msg: format!("`{name}` takes {k} argument(s), found {}", a.len()) }) }
                };
                match name.as_str() {
                    "ObjectIntersectionOf" | "ObjectUnionOf" => {
                        if a.len() < 2 {
                            return Err(ParseError::Syntax { line: *line, msg: format!("`{name}` needs at least two classes") });
                        }
                        let v = a.iter().map(|x| self.ce(x)).collect::<Res<Vec<_>>>()?;
                        Ok(if name == "ObjectIntersectionOf" { ClassExpr::And(v) } else { ClassExpr::Or(v) })
                    }
                    "ObjectComplementOf" => {
                        need(1)?;
                        Ok(ClassExpr::Not(Box::new(self.ce(&a[0])?)))
                    }
                    "ObjectSomeValuesFrom" | "ObjectAllValuesFrom" => {
                        need(2)?;
                        let (p, c) = (self.ope(&a[0])?, Box::new(self.ce(&a[1])?));
                        Ok(if name == "ObjectSomeValuesFrom" { ClassExpr::Some(p, c) } else { ClassExpr::All(p, c) })
                    }
                    "ObjectMinCardinality" | "ObjectMaxCardinality" | "ObjectExactCardinality" => {
                        if a.len() != 2 && a.len() != 3 {
                            return Err(ParseError::Syntax { line: *line, msg: format!("`{name}` takes a number, a property and optionally a class") });
                        }
                        let n = self.count(&a[0])?;
                        let p = self.ope(&a[1])?;
                        let c = Box::new(if a.len() == 3 { self.ce(&a[2])? } else { ClassExpr::Thing });
                        Ok(match name.as_str() {
                            "ObjectMinCardinality" => ClassExpr::Min(n, p, c),
                            "ObjectMaxCardinality" => ClassExpr::Max(n, p, c),
                            _ => ClassExpr::Exact(n, p, c),
                        })
                    }
                    "ObjectOneOf" => {
                        if a.is_empty() {
                            return Err(ParseError::Syntax { line: *line, msg: "`ObjectOneOf` needs at least one individual".into() });
                        }
                        let mut v = a.iter().map(|x| self.expand(x)).collect::<Res<Vec<_>>>()?;
                        v.sort();
                        v.dedup();
                        Ok(ClassExpr::OneOf(v))
                    }
                    "ObjectHasValue" => {
                        need(2)?;
                        Ok(ClassExpr::HasValue(self.ope(&a[0])?, self.expand(&a[1])?))
                    }
                    "DataSomeValuesFrom" | "DataAllValuesFrom" => {
                        if a.len() != 2 {
                            return self.outside(n, name, "one data property and one data range");
                        }
                        let (p, d) = (self.expand(&a[0])?, self.dr(&a[1])?);
                        Ok(if name == "DataSomeValuesFrom" { ClassExpr::DataSome(p, d) } else { ClassExpr::DataAll(p, d) })
                    }
                    "DataHasValue" => {
                        need(2)?;
                        Ok(ClassExpr::DataHasValue(self.expand(&a[0])?, self.literal(&a[1])?))
                    }
                    "DataMinCardinality" | "DataMaxCardinality" | "DataExactCardinality" => {
                        if a.len() != 2 && a.len() != 3 {
                            return Err(ParseError::Syntax { line: *line, msg: format!("`{name}` takes a number, a data property and optionally a data range") });
                        }
                        let n = self.count(&a[0])?;
                        let p = self.expand(&a[1])?;
                        let d = if a.len() == 3 { self.dr(&a[2])? } else { DataRange::Top };
                        Ok(match name.as_str() {
                            "DataMinCardinality" => ClassExpr::DataMin(n, p, d),
                            "DataMaxCardinality" => ClassExpr::DataMax(n, p, d),
                            _ => ClassExpr::DataExact(n, p, d),
                        })
                    }
                    "ObjectHasSelf" => {
                        need(1)?;
                        Ok(ClassExpr::HasSelf(self.ope(&a[0])?))
                    }
                    other => self.outside(n, other, "not a class expression XIAR supports"),
                }
            }
            _ => {
                let i = self.expand(n)?;
                Ok(match i.0.as_str() {
                    OWL_THING => ClassExpr::Thing,
                    OWL_NOTHING => ClassExpr::Nothing,
                    _ => ClassExpr::Class(i),
                })
            }
        }
    }

    fn literal(&self, n: &Node) -> Res<Literal> {
        match n {
            Node::Lit(s, dt, line) => {
                let datatype = match dt {
                    None => Datatype::String,
                    Some(d) => {
                        let i = self.expand(d)?;
                        Datatype::from_iri(&i.0).ok_or(ParseError::OutsideProfile { line: *line, construct: i.0.clone(), why: DATATYPES })?
                    }
                };
                Ok(Literal { lexical: s.clone(), datatype })
            }
            other => Err(ParseError::Syntax { line: other.line(), msg: "expected a literal".into() }),
        }
    }

    fn axiom(&self, n: &Node) -> Res<Option<Axiom>> {
        let Node::Call(name, a, line) = n else { return Err(ParseError::Syntax { line: n.line(), msg: "expected an axiom".into() }) };
        let line = *line;
        let arity = |k: usize| -> Res<()> {
            if a.len() == k { Ok(()) } else { Err(ParseError::Syntax { line, msg: format!("`{name}` takes {k} argument(s), found {}", a.len()) }) }
        };
        if a.iter().any(|x| matches!(x, Node::Call(c, _, _) if c == "Annotation")) {
            return self.outside(n, "Annotation", "axiom annotations are not supported (use AnnotationAssertion)");
        }
        let ces = |v: &[Node]| v.iter().map(|x| self.ce(x)).collect::<Res<Vec<_>>>();
        Ok(Some(match name.as_str() {
            "Declaration" => {
                arity(1)?;
                match &a[0] {
                    Node::Call(k, x, _) if x.len() == 1 => match k.as_str() {
                        "Class" => Axiom::DeclareClass(self.expand(&x[0])?),
                        "ObjectProperty" => Axiom::DeclareObjectProperty(self.expand(&x[0])?),
                        "DataProperty" => Axiom::DeclareDataProperty(self.expand(&x[0])?),
                        "NamedIndividual" => Axiom::DeclareIndividual(self.expand(&x[0])?),
                        "AnnotationProperty" | "Datatype" => return Ok(None),
                        other => return self.outside(n, other, "declaration kind"),
                    },
                    other => return Err(ParseError::Syntax { line: other.line(), msg: "malformed declaration".into() }),
                }
            }
            "SubClassOf" => {
                arity(2)?;
                Axiom::SubClassOf(self.ce(&a[0])?, self.ce(&a[1])?)
            }
            "EquivalentClasses" | "DisjointClasses" => {
                if a.len() < 2 {
                    return Err(ParseError::Syntax { line, msg: format!("`{name}` needs at least two classes") });
                }
                let v = ces(a)?;
                if name == "EquivalentClasses" { Axiom::EquivalentClasses(v) } else { Axiom::DisjointClasses(v) }
            }
            "DisjointUnion" => {
                if a.len() < 3 {
                    return Err(ParseError::Syntax { line, msg: "`DisjointUnion` needs a class and at least two disjuncts".into() });
                }
                Axiom::DisjointUnion(self.expand(&a[0])?, ces(&a[1..])?)
            }
            "SubObjectPropertyOf" => {
                arity(2)?;
                if let Node::Call(c, chain, _) = &a[0] {
                    if c == "ObjectPropertyChain" {
                        if chain.len() < 2 {
                            return Err(ParseError::Syntax { line, msg: "`ObjectPropertyChain` needs at least two properties".into() });
                        }
                        let w = chain.iter().map(|x| self.ope(x)).collect::<Res<Vec<_>>>()?;
                        return Ok(Some(Axiom::SubPropertyChain(w, self.ope(&a[1])?)));
                    }
                }
                Axiom::SubObjectPropertyOf(self.ope(&a[0])?, self.ope(&a[1])?)
            }
            "EquivalentObjectProperties" => {
                if a.len() < 2 {
                    return Err(ParseError::Syntax { line, msg: "`EquivalentObjectProperties` needs at least two properties".into() });
                }
                Axiom::EquivalentObjectProperties(a.iter().map(|x| self.ope(x)).collect::<Res<_>>()?)
            }
            "InverseObjectProperties" => {
                arity(2)?;
                match (self.ope(&a[0])?, self.ope(&a[1])?) {
                    (ObjectProp::Named(p), ObjectProp::Named(q)) => Axiom::InverseObjectProperties(p, q),
                    _ => return self.outside(n, "InverseObjectProperties", "named properties only"),
                }
            }
            "ObjectPropertyDomain" => {
                arity(2)?;
                Axiom::ObjectPropertyDomain(self.ope(&a[0])?, self.ce(&a[1])?)
            }
            "ObjectPropertyRange" => {
                arity(2)?;
                Axiom::ObjectPropertyRange(self.ope(&a[0])?, self.ce(&a[1])?)
            }
            "FunctionalObjectProperty" => {
                arity(1)?;
                Axiom::FunctionalObjectProperty(self.ope(&a[0])?)
            }
            "InverseFunctionalObjectProperty" => {
                arity(1)?;
                Axiom::InverseFunctionalObjectProperty(self.ope(&a[0])?)
            }
            "TransitiveObjectProperty" => {
                arity(1)?;
                Axiom::TransitiveObjectProperty(self.ope(&a[0])?)
            }
            "SymmetricObjectProperty" => {
                arity(1)?;
                Axiom::SymmetricObjectProperty(self.ope(&a[0])?)
            }
            "ReflexiveObjectProperty" => {
                arity(1)?;
                Axiom::ReflexiveObjectProperty(self.ope(&a[0])?)
            }
            "IrreflexiveObjectProperty" => {
                arity(1)?;
                Axiom::IrreflexiveObjectProperty(self.ope(&a[0])?)
            }
            "AsymmetricObjectProperty" => {
                arity(1)?;
                Axiom::AsymmetricObjectProperty(self.ope(&a[0])?)
            }
            "DisjointObjectProperties" => {
                if a.len() < 2 {
                    return Err(ParseError::Syntax { line, msg: "`DisjointObjectProperties` needs at least two properties".into() });
                }
                Axiom::DisjointObjectProperties(a.iter().map(|x| self.ope(x)).collect::<Res<_>>()?)
            }
            "NegativeObjectPropertyAssertion" => {
                arity(3)?;
                Axiom::NegativeObjectPropertyAssertion(self.ope(&a[0])?, self.expand(&a[1])?, self.expand(&a[2])?)
            }
            "NegativeDataPropertyAssertion" => {
                arity(3)?;
                Axiom::NegativeDataPropertyAssertion(self.expand(&a[0])?, self.expand(&a[1])?, self.literal(&a[2])?)
            }
            "FunctionalDataProperty" => {
                arity(1)?;
                Axiom::FunctionalDataProperty(self.expand(&a[0])?)
            }
            "SubDataPropertyOf" => {
                arity(2)?;
                Axiom::SubDataPropertyOf(self.expand(&a[0])?, self.expand(&a[1])?)
            }
            "HasKey" => {
                let (Some(Node::Call(l1, ops, _)), Some(Node::Call(l2, dps, _))) = (a.get(1), a.get(2)) else {
                    return Err(ParseError::Syntax { line, msg: "`HasKey` is `HasKey(class (object properties) (data properties))`".into() });
                };
                if a.len() != 3 || !l1.is_empty() || !l2.is_empty() || ops.len() + dps.len() == 0 {
                    return Err(ParseError::Syntax { line, msg: "`HasKey` is `HasKey(class (object properties) (data properties))` with at least one key property".into() });
                }
                Axiom::HasKey(self.ce(&a[0])?, ops.iter().map(|x| self.ope(x)).collect::<Res<_>>()?, dps.iter().map(|x| self.expand(x)).collect::<Res<_>>()?)
            }
            "DisjointDataProperties" => {
                if a.len() < 2 {
                    return Err(ParseError::Syntax { line, msg: "`DisjointDataProperties` needs at least two properties".into() });
                }
                Axiom::DisjointDataProperties(a.iter().map(|x| self.expand(x)).collect::<Res<_>>()?)
            }
            "EquivalentDataProperties" => {
                if a.len() < 2 {
                    return Err(ParseError::Syntax { line, msg: "`EquivalentDataProperties` needs at least two properties".into() });
                }
                Axiom::EquivalentDataProperties(a.iter().map(|x| self.expand(x)).collect::<Res<_>>()?)
            }
            "DatatypeDefinition" => {
                arity(2)?;
                Axiom::DatatypeDefinition(self.expand(&a[0])?, self.dr(&a[1])?)
            }
            "DataPropertyDomain" => {
                arity(2)?;
                Axiom::DataPropertyDomain(self.expand(&a[0])?, self.ce(&a[1])?)
            }
            "DataPropertyRange" => {
                arity(2)?;
                Axiom::DataPropertyRange(self.expand(&a[0])?, self.dr(&a[1])?)
            }
            "ClassAssertion" => {
                arity(2)?;
                Axiom::ClassAssertion(self.ce(&a[0])?, self.expand(&a[1])?)
            }
            "ObjectPropertyAssertion" => {
                arity(3)?;
                Axiom::ObjectPropertyAssertion(self.ope(&a[0])?, self.expand(&a[1])?, self.expand(&a[2])?)
            }
            "DataPropertyAssertion" => {
                arity(3)?;
                Axiom::DataPropertyAssertion(self.expand(&a[0])?, self.expand(&a[1])?, self.literal(&a[2])?)
            }
            "SameIndividual" | "DifferentIndividuals" => {
                if a.len() < 2 {
                    return Err(ParseError::Syntax { line, msg: format!("`{name}` needs at least two individuals") });
                }
                let v = a.iter().map(|x| self.expand(x)).collect::<Res<Vec<_>>>()?;
                if name == "SameIndividual" { Axiom::SameIndividual(v) } else { Axiom::DifferentIndividuals(v) }
            }
            "AnnotationAssertion" => {
                arity(3)?;
                match &a[2] {
                    Node::Lit(..) => Axiom::AnnotationAssertion(self.expand(&a[0])?, self.expand(&a[1])?, self.literal(&a[2])?.lexical),
                    other => return self.outside(other, "AnnotationAssertion", "text values only"),
                }
            }
            other => return self.outside(n, other, "unknown or unsupported axiom"),
        }))
    }
}

/// Parse an ontology document. Prefix declarations may appear before `Ontology(...)`.
pub fn parse(src: &str) -> Result<Ontology, ParseError> {
    let toks = lex(src)?;
    let mut p = P { toks, pos: 0 };
    let mut nodes = Vec::new();
    while p.pos < p.toks.len() {
        nodes.push(p.node()?);
    }
    let mut ctx = Ctx { prefixes: Vec::new(), datatypes: Default::default() };
    let mut onto = Ontology::default();
    let mut body: Option<&Vec<Node>> = None;
    for n in &nodes {
        match n {
            Node::Call(c, args, line) if c == "Prefix" => {
                // Prefix(ex:=<iri>) lexes as Name("ex:"), Iri
                match args.as_slice() {
                    [Node::Name(name, _), Node::Iri(ns, _)] => {
                        let key = name.trim_end_matches(':').to_string();
                        ctx.prefixes.push((key.clone(), ns.clone()));
                        onto.prefixes.push((key, ns.clone()));
                    }
                    _ => return Err(ParseError::Syntax { line: *line, msg: "malformed Prefix declaration".into() }),
                }
            }
            Node::Call(c, args, _) if c == "Ontology" => body = Some(args),
            other => return Err(ParseError::Syntax { line: other.line(), msg: "expected Prefix(...) or Ontology(...)".into() }),
        }
    }
    let Some(args) = body else { return Err(ParseError::Syntax { line: 1, msg: "no Ontology(...) found".into() }) };
    let mut it = args.iter().peekable();
    let mut leading = 0;
    while let Some(n @ (Node::Iri(..) | Node::Name(..))) = it.peek().copied() {
        let i = ctx.expand(n)?;
        match leading {
            0 => onto.iri = Some(i),
            1 => onto.version_iri = Some(i),
            _ => break,
        }
        leading += 1;
        it.next();
    }
    // named datatypes first, so a definition may be used before it is stated; each must reduce to known datatypes
    let items: Vec<&Node> = it.collect();
    let mut pending: Vec<&Node> = items.iter().copied().filter(|n| matches!(n, Node::Call(c, a, _) if c == "DatatypeDefinition" && a.len() == 2)).collect();
    while !pending.is_empty() {
        let before = pending.len();
        let mut next = Vec::new();
        for n in pending {
            let Node::Call(_, a, _) = n else { continue };
            match ctx.dr(&a[1]) {
                Ok(d) => {
                    let name = ctx.expand(&a[0])?;
                    ctx.datatypes.borrow_mut().insert(name.0, d);
                }
                Err(_) => next.push(n),
            }
        }
        if next.len() == before {
            // no progress: report the first problem (an unknown datatype, or definitions that need each other)
            let Node::Call(_, a, _) = next[0] else { unreachable!() };
            ctx.dr(&a[1])?;
        }
        pending = next;
    }
    for n in items {
        if matches!(n, Node::Call(c, _, _) if c == "Import") {
            return Err(ParseError::OutsideProfile { line: n.line(), construct: "Import".into(), why: "an ontology must be self-contained" });
        }
        if matches!(n, Node::Call(c, _, _) if c == "Annotation") {
            return Err(ParseError::OutsideProfile { line: n.line(), construct: "Annotation".into(), why: "ontology annotations are not supported" });
        }
        if let Some(ax) = ctx.axiom(n)? {
            onto.axioms.push(ax);
        }
    }
    crate::reasoner::check_simple_roles(&onto).map_err(ParseError::NotDl)?;
    Ok(onto)
}

// ------------------------------------------------------------------ writer
struct W<'a> {
    prefixes: &'a [(String, String)],
}

impl W<'_> {
    fn iri(&self, i: &Iri) -> String {
        let all = self.prefixes.iter().map(|(a, b)| (a.as_str(), b.as_str())).chain(BUILTIN.iter().copied());
        let mut best: Option<(&str, &str)> = None;
        for (p, ns) in all {
            if let Some(local) = i.0.strip_prefix(ns) {
                let ok = !local.is_empty() && local.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-')) && !local.starts_with('-');
                if ok && best.is_none_or(|(_, b)| ns.len() > b.len()) {
                    best = Some((p, ns));
                }
            }
        }
        match best {
            Some((p, ns)) => format!("{p}:{}", &i.0[ns.len()..]),
            None => format!("<{}>", i.0),
        }
    }

    fn ope(&self, p: &ObjectProp) -> String {
        match p {
            ObjectProp::Named(i) => self.iri(i),
            ObjectProp::Inverse(i) => format!("ObjectInverseOf({})", self.iri(i)),
        }
    }

    fn dr(&self, d: &DataRange) -> String {
        let list = |v: &[DataRange]| v.iter().map(|x| self.dr(x)).collect::<Vec<_>>().join(" ");
        match d {
            DataRange::Top => "rdfs:Literal".into(),
            DataRange::Datatype(t) => self.iri(&Iri(t.iri().into())),
            DataRange::OneOf(v) => format!("DataOneOf({})", v.iter().map(|l| self.lit(l)).collect::<Vec<_>>().join(" ")),
            DataRange::Not(x) => format!("DataComplementOf({})", self.dr(x)),
            DataRange::And(v) => format!("DataIntersectionOf({})", list(v)),
            DataRange::Or(v) => format!("DataUnionOf({})", list(v)),
            DataRange::Facets(t, fs) => format!(
                "DatatypeRestriction({} {})",
                self.iri(&Iri(t.iri().into())),
                fs.iter().map(|(f, l)| format!("{} {}", self.iri(&Iri(f.iri().into())), self.lit(l))).collect::<Vec<_>>().join(" ")
            ),
        }
    }

    fn ce(&self, c: &ClassExpr) -> String {
        let list = |v: &[ClassExpr]| v.iter().map(|x| self.ce(x)).collect::<Vec<_>>().join(" ");
        let card = |name: &str, n: &u32, p: &ObjectProp, c: &ClassExpr| {
            if *c == ClassExpr::Thing { format!("{name}({n} {})", self.ope(p)) } else { format!("{name}({n} {} {})", self.ope(p), self.ce(c)) }
        };
        let dcard = |name: &str, n: &u32, p: &Iri, d: &DataRange| {
            if *d == DataRange::Top { format!("{name}({n} {})", self.iri(p)) } else { format!("{name}({n} {} {})", self.iri(p), self.dr(d)) }
        };
        match c {
            ClassExpr::Thing => "owl:Thing".into(),
            ClassExpr::Nothing => "owl:Nothing".into(),
            ClassExpr::Class(i) => self.iri(i),
            ClassExpr::And(v) => format!("ObjectIntersectionOf({})", list(v)),
            ClassExpr::Or(v) => format!("ObjectUnionOf({})", list(v)),
            ClassExpr::Not(x) => format!("ObjectComplementOf({})", self.ce(x)),
            ClassExpr::Some(p, x) => format!("ObjectSomeValuesFrom({} {})", self.ope(p), self.ce(x)),
            ClassExpr::All(p, x) => format!("ObjectAllValuesFrom({} {})", self.ope(p), self.ce(x)),
            ClassExpr::Min(n, p, x) => card("ObjectMinCardinality", n, p, x),
            ClassExpr::Max(n, p, x) => card("ObjectMaxCardinality", n, p, x),
            ClassExpr::Exact(n, p, x) => card("ObjectExactCardinality", n, p, x),
            ClassExpr::OneOf(v) => format!("ObjectOneOf({})", v.iter().map(|i| self.iri(i)).collect::<Vec<_>>().join(" ")),
            ClassExpr::HasValue(p, i) => format!("ObjectHasValue({} {})", self.ope(p), self.iri(i)),
            ClassExpr::HasSelf(p) => format!("ObjectHasSelf({})", self.ope(p)),
            ClassExpr::AllState(..) => String::new(), // internal to the reasoner
            ClassExpr::DataSome(p, d) => format!("DataSomeValuesFrom({} {})", self.iri(p), self.dr(d)),
            ClassExpr::DataAll(p, d) => format!("DataAllValuesFrom({} {})", self.iri(p), self.dr(d)),
            ClassExpr::DataHasValue(p, l) => format!("DataHasValue({} {})", self.iri(p), self.lit(l)),
            ClassExpr::DataMin(n, p, d) => dcard("DataMinCardinality", n, p, d),
            ClassExpr::DataMax(n, p, d) => dcard("DataMaxCardinality", n, p, d),
            ClassExpr::DataExact(n, p, d) => dcard("DataExactCardinality", n, p, d),
        }
    }

    fn lit(&self, l: &Literal) -> String {
        let esc = l.lexical.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        if l.datatype == Datatype::String { format!("\"{esc}\"") } else { format!("\"{esc}\"^^{}", self.iri(&Iri(l.datatype.iri().into()))) }
    }

    fn axiom(&self, a: &Axiom) -> String {
        let ids = |v: &[Iri]| v.iter().map(|i| self.iri(i)).collect::<Vec<_>>().join(" ");
        match a {
            Axiom::DeclareClass(i) => format!("Declaration(Class({}))", self.iri(i)),
            Axiom::DeclareObjectProperty(i) => format!("Declaration(ObjectProperty({}))", self.iri(i)),
            Axiom::DeclareDataProperty(i) => format!("Declaration(DataProperty({}))", self.iri(i)),
            Axiom::DeclareIndividual(i) => format!("Declaration(NamedIndividual({}))", self.iri(i)),
            Axiom::SubClassOf(x, y) => format!("SubClassOf({} {})", self.ce(x), self.ce(y)),
            Axiom::EquivalentClasses(v) => format!("EquivalentClasses({})", v.iter().map(|x| self.ce(x)).collect::<Vec<_>>().join(" ")),
            Axiom::DisjointClasses(v) => format!("DisjointClasses({})", v.iter().map(|x| self.ce(x)).collect::<Vec<_>>().join(" ")),
            Axiom::DisjointUnion(c, v) => format!("DisjointUnion({} {})", self.iri(c), v.iter().map(|x| self.ce(x)).collect::<Vec<_>>().join(" ")),
            Axiom::SubPropertyChain(w, t) => format!("SubObjectPropertyOf(ObjectPropertyChain({}) {})", w.iter().map(|x| self.ope(x)).collect::<Vec<_>>().join(" "), self.ope(t)),
            Axiom::SubObjectPropertyOf(p, q) => format!("SubObjectPropertyOf({} {})", self.ope(p), self.ope(q)),
            Axiom::EquivalentObjectProperties(v) => format!("EquivalentObjectProperties({})", v.iter().map(|x| self.ope(x)).collect::<Vec<_>>().join(" ")),
            Axiom::InverseObjectProperties(p, q) => format!("InverseObjectProperties({} {})", self.iri(p), self.iri(q)),
            Axiom::ObjectPropertyDomain(p, c) => format!("ObjectPropertyDomain({} {})", self.ope(p), self.ce(c)),
            Axiom::ObjectPropertyRange(p, c) => format!("ObjectPropertyRange({} {})", self.ope(p), self.ce(c)),
            Axiom::FunctionalObjectProperty(p) => format!("FunctionalObjectProperty({})", self.ope(p)),
            Axiom::InverseFunctionalObjectProperty(p) => format!("InverseFunctionalObjectProperty({})", self.ope(p)),
            Axiom::TransitiveObjectProperty(p) => format!("TransitiveObjectProperty({})", self.ope(p)),
            Axiom::SymmetricObjectProperty(p) => format!("SymmetricObjectProperty({})", self.ope(p)),
            Axiom::ReflexiveObjectProperty(p) => format!("ReflexiveObjectProperty({})", self.ope(p)),
            Axiom::IrreflexiveObjectProperty(p) => format!("IrreflexiveObjectProperty({})", self.ope(p)),
            Axiom::AsymmetricObjectProperty(p) => format!("AsymmetricObjectProperty({})", self.ope(p)),
            Axiom::DisjointObjectProperties(v) => format!("DisjointObjectProperties({})", v.iter().map(|x| self.ope(x)).collect::<Vec<_>>().join(" ")),
            Axiom::NegativeObjectPropertyAssertion(p, a, b) => format!("NegativeObjectPropertyAssertion({} {} {})", self.ope(p), self.iri(a), self.iri(b)),
            Axiom::NegativeDataPropertyAssertion(p, a, l) => format!("NegativeDataPropertyAssertion({} {} {})", self.iri(p), self.iri(a), self.lit(l)),
            Axiom::DataPropertyDomain(p, c) => format!("DataPropertyDomain({} {})", self.iri(p), self.ce(c)),
            Axiom::DataPropertyRange(p, d) => format!("DataPropertyRange({} {})", self.iri(p), self.dr(d)),
            Axiom::FunctionalDataProperty(p) => format!("FunctionalDataProperty({})", self.iri(p)),
            Axiom::SubDataPropertyOf(p, q) => format!("SubDataPropertyOf({} {})", self.iri(p), self.iri(q)),
            Axiom::EquivalentDataProperties(v) => format!("EquivalentDataProperties({})", ids(v)),
            Axiom::DisjointDataProperties(v) => format!("DisjointDataProperties({})", ids(v)),
            Axiom::DatatypeDefinition(n, d) => format!("DatatypeDefinition({} {})", self.iri(n), self.dr(d)),
            Axiom::HasKey(c, ps, ds) => format!("HasKey({} ({}) ({}))", self.ce(c), ps.iter().map(|x| self.ope(x)).collect::<Vec<_>>().join(" "), ids(ds)),
            Axiom::ClassAssertion(c, i) => format!("ClassAssertion({} {})", self.ce(c), self.iri(i)),
            Axiom::ObjectPropertyAssertion(p, a, b) => format!("ObjectPropertyAssertion({} {} {})", self.ope(p), self.iri(a), self.iri(b)),
            Axiom::DataPropertyAssertion(p, a, l) => format!("DataPropertyAssertion({} {} {})", self.iri(p), self.iri(a), self.lit(l)),
            Axiom::SameIndividual(v) => format!("SameIndividual({})", ids(v)),
            Axiom::DifferentIndividuals(v) => format!("DifferentIndividuals({})", ids(v)),
            Axiom::AnnotationAssertion(p, s, t) => format!("AnnotationAssertion({} {} {})", self.iri(p), self.iri(s), self.lit(&Literal { lexical: t.clone(), datatype: Datatype::String })),
        }
    }
}

/// Render an ontology as functional syntax. `parse(write(o))` equals `o`.
pub fn write(o: &Ontology) -> String {
    let w = W { prefixes: &o.prefixes };
    let mut s = String::new();
    for (p, ns) in &o.prefixes {
        s.push_str(&format!("Prefix({p}:=<{ns}>)\n"));
    }
    s.push_str("\nOntology(");
    if let Some(i) = &o.iri {
        s.push_str(&format!("<{}>", i.0));
        if let Some(v) = &o.version_iri {
            s.push_str(&format!(" <{}>", v.0));
        }
    }
    s.push('\n');
    for a in &o.axioms {
        s.push_str("  ");
        s.push_str(&w.axiom(a));
        s.push('\n');
    }
    s.push_str(")\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# a small family ontology
Prefix(ex:=<http://example.org/xiar#>)
Prefix(rdfs:=<http://www.w3.org/2000/01/rdf-schema#>)

Ontology(<http://example.org/xiar> <http://example.org/xiar/1>
  Declaration(Class(ex:Person))
  Declaration(ObjectProperty(ex:hasParent))
  Declaration(DataProperty(ex:age))
  Declaration(NamedIndividual(ex:alice))
  AnnotationAssertion(rdfs:label ex:Person "Person \"human\"")
  SubClassOf(ex:Mother ex:Parent)
  EquivalentClasses(ex:Parent ObjectSomeValuesFrom(ex:hasChild owl:Thing))
  DisjointClasses(ex:Person ex:Place)
  SubObjectPropertyOf(ex:hasParent ex:hasAncestor)
  InverseObjectProperties(ex:hasParent ex:hasChild)
  ObjectPropertyDomain(ex:hasParent ex:Person)
  ObjectPropertyRange(ex:hasParent ObjectUnionOf(ex:Person ex:Pet))
  TransitiveObjectProperty(ex:hasAncestor)
  FunctionalObjectProperty(ObjectInverseOf(ex:hasMother))
  SubClassOf(ex:Person ObjectMaxCardinality(2 ex:hasParent))
  SubClassOf(ex:Pet ObjectMinCardinality(1 ex:hasOwner ex:Person))
  SubClassOf(ex:Orphan ObjectAllValuesFrom(ex:hasParent ex:Deceased))
  SubClassOf(ex:Alien ObjectComplementOf(ex:Person))
  DataPropertyDomain(ex:age ex:Person)
  DataPropertyRange(ex:age xsd:integer)
  ClassAssertion(ex:Person ex:alice)
  ObjectPropertyAssertion(ex:hasParent ex:alice ex:bob)
  DataPropertyAssertion(ex:age ex:alice "30"^^xsd:integer)
  SameIndividual(ex:alice ex:alicia)
  DifferentIndividuals(ex:alice ex:bob)
)
"#;

    #[test]
    fn a_full_profile_document_parses() {
        let o = parse(SAMPLE).unwrap();
        assert_eq!(o.iri, Some(Iri::new("http://example.org/xiar")));
        assert_eq!(o.version_iri, Some(Iri::new("http://example.org/xiar/1")));
        assert_eq!(o.axioms.len(), 25);
        assert!(o.axioms.contains(&Axiom::SubClassOf(ClassExpr::class("http://example.org/xiar#Mother"), ClassExpr::class("http://example.org/xiar#Parent"))));
        assert_eq!(o.label(&Iri::new("http://example.org/xiar#Person")), Some("Person \"human\""));
        assert!(o.class_names().contains(&Iri::new("http://example.org/xiar#Orphan")));
        assert!(o.object_property_names().contains(&Iri::new("http://example.org/xiar#hasOwner")));
        assert!(o.individual_names().contains(&Iri::new("http://example.org/xiar#alicia")));
    }

    #[test]
    fn writing_then_parsing_gives_the_same_ontology() {
        let o = parse(SAMPLE).unwrap();
        let text = write(&o);
        assert!(text.contains("SubClassOf(ex:Mother ex:Parent)") && text.contains("ObjectMaxCardinality(2 ex:hasParent)"), "{text}");
        assert!(text.contains("\"30\"^^xsd:integer"));
        assert_eq!(parse(&text).unwrap(), o);
        assert_eq!(write(&parse(&text).unwrap()), text, "the writer is stable");
    }

    #[test]
    fn constructs_outside_the_profile_are_rejected_by_name_and_line() {
        for (src, construct) in [
            ("SubClassOf(Annotation(rdfs:comment \"x\") ex:A ex:B)", "Annotation"),
            ("ClassAssertion(ex:A ex:a)\n  DataPropertyRange(ex:d xsd:duration)", "http://www.w3.org/2001/XMLSchema#duration"),
            ("DataPropertyRange(ex:d DatatypeRestriction(xsd:string xsd:minInclusive \"a\"))", "DatatypeRestriction"),
            ("DataPropertyRange(ex:d DataSomeValuesFrom(ex:d xsd:string))", "DataSomeValuesFrom"),
            ("Import(<http://other>)", "Import"),
        ] {
            let doc = format!("Prefix(ex:=<http://e/#>)\nOntology(<http://e>\n  {src}\n)");
            match parse(&doc) {
                Err(ParseError::OutsideProfile { construct: c, line, .. }) => {
                    assert_eq!(c, construct, "{src}");
                    assert!(line >= 3, "{src}: line {line}");
                }
                other => panic!("{src}: expected a profile error, got {other:?}"),
            }
        }
    }

    const WIDE: &str = r#"
Prefix(ex:=<http://example.org/xiar#>)
Ontology(<http://example.org/wide>
  Declaration(DataProperty(ex:age))
  SubClassOf(ex:Adult DataSomeValuesFrom(ex:age DatatypeRestriction(xsd:integer xsd:minInclusive "18"^^xsd:integer)))
  SubClassOf(ex:Minor DataAllValuesFrom(ex:age DataComplementOf(DatatypeRestriction(xsd:integer xsd:minInclusive "18"^^xsd:integer))))
  EquivalentClasses(ex:Colour ObjectOneOf(ex:red ex:green ex:blue))
  SubClassOf(ex:Rose ObjectHasValue(ex:hasColour ex:red))
  SubClassOf(ex:Person DataMaxCardinality(1 ex:age))
  SubClassOf(ex:Tagged DataMinCardinality(2 ex:tag xsd:string))
  SubClassOf(ex:Fixed DataHasValue(ex:code "A1"))
  DataPropertyRange(ex:size DataUnionOf(DataOneOf("small" "large") DatatypeRestriction(xsd:decimal xsd:minExclusive "0.5"^^xsd:decimal xsd:maxInclusive "9"^^xsd:decimal)))
  DataPropertyRange(ex:any rdfs:Literal)
  FunctionalDataProperty(ex:code)
  SubDataPropertyOf(ex:birthYear ex:year)
  EquivalentDataProperties(ex:yr ex:year)
  DataPropertyAssertion(ex:ratio ex:a "1.5"^^xsd:double)
)
"#;

    #[test]
    fn nominals_and_data_restrictions_parse_and_round_trip() {
        let o = parse(WIDE).unwrap();
        assert_eq!(o.axioms.len(), 14);
        let colours = ClassExpr::OneOf(vec![Iri::new("http://example.org/xiar#blue"), Iri::new("http://example.org/xiar#green"), Iri::new("http://example.org/xiar#red")]);
        assert!(o.axioms.iter().any(|a| matches!(a, Axiom::EquivalentClasses(v) if v[1] == colours)), "members are sorted so equal sets compare equal");
        assert!(o.individual_names().contains(&Iri::new("http://example.org/xiar#red")), "nominal members are individuals");
        assert!(o.data_property_names().contains(&Iri::new("http://example.org/xiar#tag")));
        let text = write(&o);
        assert!(text.contains("ObjectOneOf(ex:blue ex:green ex:red)") && text.contains("DatatypeRestriction(xsd:integer xsd:minInclusive \"18\"^^xsd:integer)"), "{text}");
        assert!(text.contains("DataMinCardinality(2 ex:tag xsd:string)") && text.contains("DataMaxCardinality(1 ex:age)") && text.contains("DataPropertyRange(ex:any rdfs:Literal)"), "{text}");
        assert_eq!(parse(&text).unwrap(), o);
        assert_eq!(write(&parse(&text).unwrap()), text, "the writer is stable");
    }

    #[test]
    fn syntax_errors_are_clear_and_located() {
        assert!(matches!(parse("Ontology(<http://e> SubClassOf(ex:A ex:B)"), Err(ParseError::Syntax { .. })), "unclosed");
        assert!(matches!(parse("Ontology(<http://e> SubClassOf(zz:A zz:B))"), Err(ParseError::UnknownPrefix { prefix, .. }) if prefix == "zz"));
        assert!(matches!(parse("Prefix(ex:=<http://e/#>)\nOntology(<http://e> SubClassOf(ex:A))"), Err(ParseError::Syntax { .. })), "wrong arity");
        assert!(matches!(parse("Prefix(ex:=<http://e/#>)\nOntology(<http://e> ObjectPropertyDomain(ex:p ObjectUnionOf(ex:A)))"), Err(ParseError::Syntax { .. })), "union of one");
        assert!(matches!(parse("Prefix(ex:=<http://e/#>)"), Err(ParseError::Syntax { .. })), "no ontology");
        assert!(matches!(parse("Ontology(<http://e> AnnotationAssertion(rdfs:label <http://e#a> \"x\"@en))"), Err(ParseError::OutsideProfile { .. })), "language tags");
        let e = parse("Ontology(<http://e>\n\n  SubClassOf(owl:Thing %)\n)").unwrap_err();
        assert!(matches!(e, ParseError::Syntax { line: 3, .. }), "{e:?}");
    }

    #[test]
    fn thing_and_nothing_and_unqualified_cardinality_normalise() {
        let o = parse("Prefix(ex:=<http://e/#>)\nOntology(<http://e> SubClassOf(owl:Nothing owl:Thing) SubClassOf(ex:A ObjectMinCardinality(1 ex:p)))").unwrap();
        assert_eq!(o.axioms[0], Axiom::SubClassOf(ClassExpr::Nothing, ClassExpr::Thing));
        assert_eq!(o.axioms[1], Axiom::SubClassOf(ClassExpr::class("http://e/#A"), ClassExpr::Min(1, ObjectProp::Named(Iri::new("http://e/#p")), Box::new(ClassExpr::Thing))));
        assert!(write(&o).contains("ObjectMinCardinality(1 ex:p)"));
    }

    #[test]
    fn an_ontology_without_an_iri_and_iris_that_cannot_be_prefixed_roundtrip() {
        let o = parse("Ontology(SubClassOf(<http://a/x y> <http://b/c>))");
        assert!(o.is_err(), "spaces are not allowed inside names here");
        let o = parse("Ontology(SubClassOf(<http://a/X> <http://b/c>))").unwrap();
        assert_eq!(o.iri, None);
        let text = write(&o);
        assert!(text.contains("<http://a/X>"));
        assert_eq!(parse(&text).unwrap(), o);
    }
}
