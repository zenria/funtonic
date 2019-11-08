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

use thiserror::Error;

const SPACES: &'static str = " \t\r\n";
const SPECIAL_AUTHORIZED_CHARS: &'static str = "-_@#";

fn sp<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, &'a str, E> {
    take_while1(move |c| SPACES.contains(c))(i)
}

fn wildcard<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, char, E> {
    nom::character::complete::char('*')(i)
}

fn field_delimiter<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, char, E> {
    nom::character::complete::char(':')(i)
}

fn pattern<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, &'a str, E> {
    take_while1(|c| is_alphanumeric(c as u8) || SPECIAL_AUTHORIZED_CHARS.contains(c))(i)
}
// `or` or `||`
fn or<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, (), E> {
    value((), alt((tag_no_case("or"), tag("||"))))(i)
}
fn or_separator<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, (), E> {
    value((), tuple((sp, or, sp)))(i)
}

fn or_clause<'a, E: ParseError<&'a str>>(
    i: &'a str,
) -> IResult<&'a str, (RawQuery<'a>, RawQuery<'a>), E> {
    separated_pair(parse_simple_query, or_separator, parse_query)(i)
}

// `and` or `&&`
fn and<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, (), E> {
    value((), alt((tag_no_case("and"), tag("&&"))))(i)
}

fn and_separator<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, (), E> {
    value((), tuple((sp, and, sp)))(i)
}

fn and_clause<'a, E: ParseError<&'a str>>(
    i: &'a str,
) -> IResult<&'a str, (RawQuery<'a>, RawQuery<'a>), E> {
    separated_pair(parse_simple_query, and_separator, parse_query)(i)
}

// field:sub_query
fn field_pattern<'a, E: ParseError<&'a str>>(
    i: &'a str,
) -> IResult<&'a str, (&'a str, char, RawQuery<'a>), E> {
    tuple((pattern, field_delimiter, parse_query))(i)
}
fn parse_simple_query<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
    alt((
        // * wildcard
        map(wildcard, |_| RawQuery::Wildcard),
        // field:_sub_query
        map(field_pattern, |(field, _, sub_query)| {
            RawQuery::FieldPattern(field, Box::new(sub_query))
        }),
        map(pattern, |s| RawQuery::Pattern(s)),
    ))(i)
}
fn parse_query<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
    alt((
        map(and_clause, |(l, r)| RawQuery::And(l.into(), r.into())),
        map(or_clause, |(l, r)| RawQuery::Or(l.into(), r.into())),
        parse_simple_query,
    ))(i)
}

fn parse_raw<'a, E: ParseError<&'a str>>(i: &'a str) -> Result<RawQuery<'a>, Err<E>> {
    let ret = complete(parse_query)(i)?;
    Ok(ret.1)
}

#[derive(Error, Debug, Clone)]
#[error("Unable to parse query {0}")]
pub struct QueryParseError(String);

pub fn parse<'a>(i: &'a str) -> Result<Query<'a>, QueryParseError> {
    let ret = complete::<_, _, VerboseError<&str>, _>(parse_query)(i);
    match ret {
        Err(e) => Err(QueryParseError(format!("{:?}", e))),
        Ok(ret) => Ok(ret.1.into()),
    }
}

/// Raw parsed query with no precedence applied
#[derive(Debug, PartialOrd, PartialEq)]
enum RawQuery<'a> {
    Pattern(&'a str),
    FieldPattern(&'a str, Box<RawQuery<'a>>),
    Wildcard,
    And(Box<RawQuery<'a>>, Box<RawQuery<'a>>),
    Or(Box<RawQuery<'a>>, Box<RawQuery<'a>>),
}

#[derive(Debug, PartialOrd, PartialEq)]
pub enum Query<'a> {
    Pattern(&'a str),
    FieldPattern(&'a str, Box<Query<'a>>),
    Wildcard,
    And(Vec<Query<'a>>),
    Or(Vec<Query<'a>>),
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{and, or, parse, parse_raw, Query, QueryMatcher, RawQuery};
    use nom::error::VerboseError;
    use std::collections::HashMap;

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

    #[test]
    fn test() {
        assert!(and::<VerboseError<&str>>("and").is_ok());
        assert!(and::<VerboseError<&str>>("&&").is_ok());
        assert!(or::<VerboseError<&str>>("or").is_ok());
        assert!(or::<VerboseError<&str>>("||").is_ok());
        assert!(parse_raw::<VerboseError<&str>>("").is_err());
        assert_eq!(
            RawQuery::Wildcard,
            parse_raw::<VerboseError<&str>>("*").unwrap()
        );
        assert_eq!(
            RawQuery::Pattern("coucou_les-amis1234"),
            parse_raw::<VerboseError<&str>>("coucou_les-amis1234").unwrap()
        );
        assert_eq!(
            RawQuery::FieldPattern("field", Box::new(RawQuery::Pattern("pattern"))),
            parse_raw::<VerboseError<&str>>("field:pattern").unwrap()
        );
        assert_eq!(
            RawQuery::FieldPattern("field", Box::new(RawQuery::Wildcard)),
            parse_raw::<VerboseError<&str>>("field:*").unwrap()
        );
        assert_eq!(
            RawQuery::FieldPattern(
                "field",
                Box::new(RawQuery::FieldPattern(
                    "sub_field",
                    Box::new(RawQuery::Pattern("pattern"))
                ))
            ),
            parse_raw::<VerboseError<&str>>("field:sub_field:pattern").unwrap()
        );
        assert_eq!(
            RawQuery::FieldPattern(
                "field",
                Box::new(RawQuery::FieldPattern(
                    "sub_field",
                    Box::new(RawQuery::Wildcard)
                ))
            ),
            parse_raw::<VerboseError<&str>>("field:sub_field:*").unwrap()
        );
        // one lvl
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo and bar").unwrap(),
            RawQuery::And(
                Box::new(RawQuery::Pattern("foo")),
                Box::new(RawQuery::Pattern("bar"))
            ),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo or bar").unwrap(),
            RawQuery::Or(
                Box::new(RawQuery::Pattern("foo")),
                Box::new(RawQuery::Pattern("bar"))
            ),
        );
        // two lvl
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo and bar and yak").unwrap(),
            RawQuery::And(
                Box::new(RawQuery::Pattern("foo")),
                Box::new(RawQuery::And(
                    Box::new(RawQuery::Pattern("bar")),
                    Box::new(RawQuery::Pattern("yak"))
                ))
            ),
        );
    }
}
