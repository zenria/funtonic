#![allow(unused)]
use nom::branch::alt;
use nom::bytes::complete::{tag, tag_no_case, take_while, take_while1};
use nom::character::is_alphanumeric;
use nom::combinator::{complete, map, value};
use nom::error::{ParseError, VerboseError};
use nom::multi::many1;
use nom::sequence::{separated_pair, tuple};
use nom::{Err, IResult};
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

use crate::parser::{parse_raw, RawQuery};
use thiserror::Error;

mod parser;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

#[derive(Error, Debug, Clone)]
pub enum QueryParseError {
    #[error("Unable to parse query {0}")]
    ParseError(String),
    #[error("Unable to parse query {0}")]
    UnrecognizedInput(String),
}

pub fn parse(i: &str) -> Result<Query, QueryParseError> {
    let ret = parse_raw::<VerboseError<&str>>(i);
    match ret {
        Err(e) => Err(QueryParseError::ParseError(format!("{:?}", e))),
        Ok(ret) => {
            let rest = ret.0;
            let res = ret.1;
            if rest.len() > 0 {
                Err(QueryParseError::UnrecognizedInput(rest.into()))
            } else {
                Ok(res.into())
            }
        }
    }
}

#[derive(Debug, PartialOrd, PartialEq)]
pub enum Query<'a> {
    Pattern(&'a str),
    FieldPattern(&'a str, Box<Query<'a>>),
    Wildcard,
    And(Vec<Query<'a>>),
    Or(Vec<Query<'a>>),
    Not(Box<Query<'a>>),
}

pub trait QueryMatcher {
    fn qmatches(&self, query: &Query) -> bool;
}

pub trait FieldExtractable {
    type Field;

    fn extract_field(&self, field: &str) -> Option<&Self::Field>;
}

impl QueryMatcher for &str {
    fn qmatches(&self, query: &Query) -> bool {
        match query {
            Query::Pattern(p) => p == self,
            Query::FieldPattern(_, _) => false,
            Query::Wildcard => true,
            Query::And(and) => and.iter().all(|q| self.qmatches(q)),
            Query::Or(or) => or.iter().any(|q| self.qmatches(q)),
            Query::Not(not) => !self.qmatches(not),
        }
    }
}

impl QueryMatcher for String {
    fn qmatches(&self, query: &Query) -> bool {
        self.as_str().qmatches(query)
    }
}

impl<V: QueryMatcher> FieldExtractable for HashMap<String, V> {
    type Field = V;

    fn extract_field(&self, field: &str) -> Option<&Self::Field> {
        self.get(field)
    }
}

impl<'a, V: QueryMatcher> FieldExtractable for HashMap<&'a str, V> {
    type Field = V;

    fn extract_field(&self, field: &str) -> Option<&Self::Field> {
        self.get(field)
    }
}

impl<Q: QueryMatcher, F: FieldExtractable<Field = Q>> QueryMatcher for F {
    fn qmatches(&self, query: &Query) -> bool {
        match query {
            Query::Pattern(_) => false,
            Query::FieldPattern(field, q) => self
                .extract_field(field)
                .map(|v| v.qmatches(q))
                .unwrap_or(false),
            Query::Wildcard => true,
            Query::And(and) => and.iter().all(|q| self.qmatches(q)),
            Query::Or(or) => or.iter().any(|q| self.qmatches(q)),
            Query::Not(not) => !self.qmatches(not),
        }
    }
}

impl<Q: QueryMatcher + Debug> QueryMatcher for Vec<Q> {
    fn qmatches(&self, query: &Query) -> bool {
        match query {
            Query::FieldPattern(_, _) => false,
            Query::Wildcard => true,
            Query::And(clauses) => clauses
                .iter()
                .all(|clause| self.iter().any(|item| item.qmatches(clause))),
            // any item that matches the query => OK
            query => self.iter().any(|item| item.qmatches(query)),
        }
    }
}

impl<'a> From<RawQuery<'a>> for Query<'a> {
    fn from(q: RawQuery<'a>) -> Self {
        match q {
            RawQuery::Pattern(p) => Query::Pattern(p),
            RawQuery::Wildcard => Query::Wildcard,
            RawQuery::FieldPattern(f, q) => {
                let q: Query = (*q).into();
                // field(field, and(field_query, and_clauses)) => field(field, field_query) && and_clauses
                // field(field, or(field_query, or_clauses)) => field(field, field_query) || or_clauses

                match q {
                    Query::And(mut clauses) => {
                        // FieldPattern has the higher precedence as anything, extract it
                        // take the first element
                        let first = clauses.drain(0..1).nth(0).unwrap();
                        // move it into a field_pattern clause
                        let field_pattern_query = Query::FieldPattern(f, first.into());
                        // issue an Ans
                        let mut and_clauses = vec![field_pattern_query];
                        and_clauses.append(&mut clauses);
                        Query::And(and_clauses)
                    }
                    Query::Or(mut clauses) => {
                        // FieldPattern has the higher precedence as anything, extract it
                        // take the first element
                        let first = clauses.drain(0..1).nth(0).unwrap();
                        // move it into a field_pattern clause
                        let field_pattern_query = Query::FieldPattern(f, first.into());
                        // issue an Or
                        let mut or_clauses = vec![field_pattern_query];
                        or_clauses.append(&mut clauses);
                        Query::Or(or_clauses)
                    }
                    q => Query::FieldPattern(f, q.into()),
                }
            }
            RawQuery::And(left, right) => {
                let left: Query = (*left).into();
                let right: Query = (*right).into();

                // simplify wildcard
                if let Query::Wildcard = &left {
                    return right;
                }

                // by construction of the parser, left is always a "canonical" query
                match right {
                    Query::Wildcard => left, // simplify wildcard
                    Query::And(mut clauses) => {
                        clauses.insert(0, left);
                        Query::And(clauses)
                    }
                    Query::Or(mut clauses) => {
                        // take the first element
                        let first = clauses.drain(0..1).nth(0).unwrap();
                        // move it into a and clause
                        let and = Query::And(vec![left, first]);
                        // issue an Or
                        let mut or_clauses = vec![and];
                        or_clauses.append(&mut clauses);
                        Query::Or(or_clauses)
                    }
                    simple => Query::And(vec![left, simple]),
                }

                // and(r_simpleQ, r_simpleQ) => simpleQ &&  simpleQ
                // and(r_simpleQ, and(r_simpleQ, r_Q)) => simpleQ && simpleQ && r_Q

                // and(simpleQ, simpleQ or Q) => (simpleQ && simpleQ) || Q
            }
            RawQuery::Or(left, right) => {
                let left: Query = (*left).into();
                let right: Query = (*right).into();

                // simplify wildcard
                if let Query::Wildcard = &left {
                    return Query::Wildcard;
                }
                match right {
                    Query::Wildcard => Query::Wildcard, // simplify wildcard
                    Query::Or(mut clauses) => {
                        // collapse all or clauses
                        clauses.insert(0, left);
                        Query::Or(clauses)
                    }
                    right => Query::Or(vec![left, right]),
                }
            }
            RawQuery::Not(raw_query) => {
                let q: Query = (*raw_query).into();
                Query::Not(Box::new(q))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{parse, QueryMatcher};
    use nom::error::VerboseError;
    use std::collections::HashMap;

    #[test]
    fn test_wrong_query() {
        assert!(parse("prod and (env:foo or bar)").is_err());
    }

    #[test]
    fn test_matches() {
        assert!("prod".qmatches(&parse("prod").unwrap()));
        assert!("prod".qmatches(&parse("*").unwrap()));
        assert!("prod".qmatches(&parse("prod or qa").unwrap()));
        assert!("qa".qmatches(&parse("prod or qa").unwrap()));
        assert!("qa".qmatches(&parse("prod and fuck or qa").unwrap()));
        assert!("qa".qmatches(&parse("prod or fuck or qa").unwrap()));
        assert!("qa".qmatches(&parse("qa or fuck and qa").unwrap()));

        assert!(!"qa".qmatches(&parse("prod").unwrap()));
        assert!(!"qa".qmatches(&parse("prod and qa").unwrap()));
        assert!(!"qa".qmatches(&parse("prod and qa or coucou").unwrap()));
        assert!(!"qa".qmatches(&parse("coucou or prod and qa or coucou").unwrap()));

        assert!(!"qa".qmatches(&parse("not qa").unwrap()));
        assert!(!"qa".qmatches(&parse("!qa").unwrap()));
        assert!("prod".qmatches(&parse("not qa").unwrap()));
        assert!("prod".qmatches(&parse("!qa").unwrap()));
        assert!(!"qa".qmatches(&parse("not  qa").unwrap()));
        assert!(!"qa".qmatches(&parse("! qa").unwrap()));
        assert!("prod".qmatches(&parse("not  qa").unwrap()));
        assert!("prod".qmatches(&parse("! qa").unwrap()));

        // do some more funny tests with maps
        let mut tags = HashMap::new();
        tags.insert("env", "prod");
        tags.insert("location", "Paris");

        assert!(!tags.qmatches(&parse("prod").unwrap()));
        assert!(!tags.qmatches(&parse("env").unwrap()));
        assert!(tags.qmatches(&parse("*").unwrap()));
        assert!(tags.qmatches(&parse("env:prod").unwrap()));
        assert!(tags.qmatches(&parse("env:*").unwrap()));
        assert!(tags.qmatches(&parse("env:qa or *").unwrap()));
        assert!(tags.qmatches(&parse("env:prod or location:anywhere").unwrap()));
        assert!(tags.qmatches(&parse("env:qa or location:Paris").unwrap()));

        // vec ftw!
        let empty: Vec<&'static str> = vec![];
        assert!(!empty.qmatches(&parse("foo").unwrap()));
        // empty still matches wilcard
        assert!(empty.qmatches(&parse("*").unwrap()));
        let non_empty = vec!["foo", "bar", "prod"];
        assert!(non_empty.qmatches(&parse("*").unwrap()));
        assert!(non_empty.qmatches(&parse("foo").unwrap()));
        assert!(non_empty.qmatches(&parse("bar").unwrap()));
        assert!(non_empty.qmatches(&parse("prod").unwrap()));
        assert!(non_empty.qmatches(&parse("prod and bar and foo").unwrap()));
        assert!(non_empty.qmatches(&parse("prod and foo").unwrap()));
        assert!(non_empty.qmatches(&parse("prod or field:bar and foo").unwrap()));
    }
}
