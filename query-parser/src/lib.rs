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
    fn qmatches(&self, query: &Query) -> MatchResult;
}

#[derive(Debug, PartialEq, Eq)]
pub enum MatchResult {
    /// matches the query
    Match,
    /// do not match the query
    NoMatch,
    /// somethign in the query is rejecting the match (typically a not clause), depending
    /// on the application it can be considered as NoMatch
    Rejected,
}

impl MatchResult {
    pub fn matches(&self) -> bool {
        match self {
            Match => true,
            _ => false,
        }
    }
}

use crate::MatchResult::{Match, NoMatch, Rejected};
use std::ops::BitAnd;
use std::ops::BitOr;
use std::ops::BitXor;
use std::ops::Not;

impl BitXor for MatchResult {
    type Output = MatchResult;

    fn bitxor(self, rhs: Self) -> Self::Output {
        if self == Rejected || rhs == Rejected {
            Rejected
        } else {
            self | rhs
        }
    }
}

impl Not for MatchResult {
    type Output = MatchResult;

    fn not(self) -> Self::Output {
        match self {
            MatchResult::Match => MatchResult::Rejected,
            MatchResult::NoMatch => MatchResult::Match,
            MatchResult::Rejected => MatchResult::Match,
        }
    }
}

impl BitAnd for MatchResult {
    type Output = MatchResult;

    fn bitand(self, rhs: Self) -> Self::Output {
        match self {
            MatchResult::Match => match rhs {
                Match => Match,
                NoMatch => NoMatch,
                MatchResult::Rejected => Rejected,
            },
            MatchResult::NoMatch => match rhs {
                Match => NoMatch,
                NoMatch => NoMatch,
                Rejected => Rejected,
            },
            MatchResult::Rejected => MatchResult::Rejected,
        }
    }
}

impl From<bool> for MatchResult {
    fn from(boolean: bool) -> Self {
        if boolean {
            MatchResult::Match
        } else {
            MatchResult::NoMatch
        }
    }
}

impl BitOr for MatchResult {
    type Output = MatchResult;

    fn bitor(self, rhs: Self) -> Self::Output {
        match self {
            Match => Match,
            NoMatch => rhs,
            Rejected => match rhs {
                Match => Match,
                _ => Rejected,
            },
        }
    }
}

pub trait FieldExtractable {
    type Field;

    fn extract_field(&self, field: &str) -> Option<&Self::Field>;
}

impl QueryMatcher for &str {
    fn qmatches(&self, query: &Query) -> MatchResult {
        match query {
            Query::Pattern(p) => (p == self).into(),
            Query::FieldPattern(_, _) => NoMatch,
            Query::Wildcard => Match,
            Query::And(and) => and.iter().fold(Match, |m, q| m & self.qmatches(q)),
            Query::Or(or) => or.iter().fold(NoMatch, |m, q| m | self.qmatches(q)),
            Query::Not(not) => !self.qmatches(not),
        }
    }
}

impl QueryMatcher for String {
    fn qmatches(&self, query: &Query) -> MatchResult {
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
    fn qmatches(&self, query: &Query) -> MatchResult {
        match query {
            Query::Pattern(_) => NoMatch,
            Query::FieldPattern(field, q) => self
                .extract_field(field)
                .map(|v| v.qmatches(q))
                .unwrap_or(NoMatch),
            Query::Wildcard => Match,
            Query::And(and) => and.iter().fold(Match, |m, q| m & self.qmatches(q)),
            Query::Or(or) => or.iter().fold(NoMatch, |m, q| m | self.qmatches(q)),
            Query::Not(not) => !self.qmatches(not),
        }
    }
}

impl<Q: QueryMatcher> QueryMatcher for &[Q] {
    fn qmatches(&self, query: &Query) -> MatchResult {
        match query {
            Query::Wildcard => Match,
            Query::And(clauses) => clauses.iter().fold(Match, |m, q| {
                // all clauses must match at least one item
                m & self.iter().fold(NoMatch, |m, item| m ^ item.qmatches(q))
            }),
            Query::Or(clauses) => clauses.iter().fold(NoMatch, |m, q| {
                // any clause must match at least one item
                m | self.iter().fold(NoMatch, |m, item| m | item.qmatches(q))
            }),

            Query::Not(_) => self.iter().fold(Match, |m, item| m & item.qmatches(query)),
            Query::Pattern(_) | Query::FieldPattern(_, _) => self
                .iter()
                .fold(NoMatch, |m, item| m | item.qmatches(query)),
        }
    }
}

impl<Q: QueryMatcher> QueryMatcher for Vec<Q> {
    fn qmatches(&self, query: &Query) -> MatchResult {
        self.as_slice().qmatches(query)
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
    use crate::MatchResult::{Match, NoMatch, Rejected};
    use crate::{parse, QueryMatcher};
    use nom::error::VerboseError;
    use std::collections::HashMap;

    #[test]
    fn test_wrong_query() {
        assert!(parse("prod and (env:foo or bar)").is_err());
    }

    #[test]
    fn test_matches() {
        assert_eq!("prod".qmatches(&parse("prod").unwrap()), Match);
        assert_eq!("prod".qmatches(&parse("*").unwrap()), Match);
        assert_eq!("prod".qmatches(&parse("prod or qa").unwrap()), Match);
        assert_eq!("qa".qmatches(&parse("prod or qa").unwrap()), Match);
        assert_eq!("qa".qmatches(&parse("prod and fuck or qa").unwrap()), Match);
        assert_eq!("qa".qmatches(&parse("prod or fuck or qa").unwrap()), Match);
        assert_eq!("qa".qmatches(&parse("qa or fuck and qa").unwrap()), Match);

        assert_eq!("qa".qmatches(&parse("prod").unwrap()), NoMatch);
        assert_eq!("qa".qmatches(&parse("prod and qa").unwrap()), NoMatch);
        assert_eq!(
            "qa".qmatches(&parse("prod and qa or coucou").unwrap()),
            NoMatch
        );
        assert_eq!(
            "qa".qmatches(&parse("coucou or prod and qa or coucou").unwrap()),
            NoMatch
        );

        assert_eq!("qa".qmatches(&parse("not qa").unwrap()), Rejected);
        assert_eq!("qa".qmatches(&parse("!qa").unwrap()), Rejected);
        assert_eq!("prod".qmatches(&parse("not qa").unwrap()), Match);
        assert_eq!("prod".qmatches(&parse("!qa").unwrap()), Match);
        assert_eq!("qa".qmatches(&parse("not  qa").unwrap()), Rejected);
        assert_eq!("qa".qmatches(&parse("! qa").unwrap()), Rejected);
        assert_eq!("prod".qmatches(&parse("not  qa").unwrap()), Match);
        assert_eq!("prod".qmatches(&parse("! qa").unwrap()), Match);

        // do some more funny tests with maps
        let mut tags = HashMap::new();
        tags.insert("env", "prod");
        tags.insert("location", "Paris");

        assert_eq!(tags.qmatches(&parse("prod").unwrap()), NoMatch);
        assert_eq!(tags.qmatches(&parse("env").unwrap()), NoMatch);
        assert_eq!(tags.qmatches(&parse("*").unwrap()), Match);
        assert_eq!(tags.qmatches(&parse("env:prod").unwrap()), Match);
        assert_eq!(tags.qmatches(&parse("env:*").unwrap()), Match);
        assert_eq!(tags.qmatches(&parse("env:qa or *").unwrap()), Match);
        assert_eq!(
            tags.qmatches(&parse("env:prod or location:anywhere").unwrap()),
            Match
        );
        assert_eq!(
            tags.qmatches(&parse("env:qa or location:Paris").unwrap()),
            Match
        );

        // vec ftw!
        let empty: Vec<&'static str> = vec![];
        assert_eq!(empty.qmatches(&parse("foo").unwrap()), NoMatch);
        // empty still matches wilcard
        assert_eq!(empty.qmatches(&parse("*").unwrap()), Match);
        let non_empty = vec!["foo", "bar", "prod"];
        assert_eq!(non_empty.qmatches(&parse("*").unwrap()), Match);
        assert_eq!(non_empty.qmatches(&parse("foo").unwrap()), Match);
        assert_eq!(non_empty.qmatches(&parse("bar").unwrap()), Match);
        assert_eq!(non_empty.qmatches(&parse("prod").unwrap()), Match);
        assert_eq!(non_empty.qmatches(&parse("!prod").unwrap()), Rejected);
        assert_eq!(
            non_empty.qmatches(&parse("prod and bar and foo").unwrap()),
            Match
        );
        assert_eq!(non_empty.qmatches(&parse("prod and foo").unwrap()), Match);
        assert_eq!(
            non_empty.qmatches(&parse("prod or field:bar and foo").unwrap()),
            Match
        );
        assert_eq!(
            non_empty.qmatches(&parse("prod and !prod").unwrap()),
            Rejected
        );
        assert_eq!(non_empty.qmatches(&parse("prod or !prod").unwrap()), Match);
    }
}
