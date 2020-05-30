use nom::branch::alt;
use nom::bytes::complete::{tag, tag_no_case, take_while, take_while1};
use nom::character::complete::char;
use nom::character::is_alphanumeric;
use nom::combinator::{complete, map, value};
use nom::error::ParseError;
use nom::sequence::{pair, separated_pair, tuple};
use nom::{Err, IResult};

/// Raw parsed query with no precedence applied
#[derive(Debug, PartialOrd, PartialEq)]
pub enum RawQuery<'a> {
    Pattern(&'a str),
    FieldPattern(&'a str, Box<RawQuery<'a>>),
    Wildcard,
    And(Box<RawQuery<'a>>, Box<RawQuery<'a>>),
    Or(Box<RawQuery<'a>>, Box<RawQuery<'a>>),
    Not(Box<RawQuery<'a>>),
}

pub fn parse_raw<'a, E: ParseError<&'a str>>(
    i: &'a str,
) -> Result<(&'a str, RawQuery<'a>), Err<E>> {
    parse_query(i)
}

const SPACES: &'static str = " \t\r\n";
const SPECIAL_AUTHORIZED_CHARS: &'static str = "-_@#.";

fn sp<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, &'a str, E> {
    take_while1(move |c| SPACES.contains(c))(i)
}

fn maybe_sp<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, &'a str, E> {
    take_while(move |c| SPACES.contains(c))(i)
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
fn comma<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, (), E> {
    value(
        (),
        tuple((maybe_sp, nom::character::complete::char(','), maybe_sp)),
    )(i)
}

fn or_separator<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, (), E> {
    value((), tuple((sp, or, sp)))(i)
}

fn or_clause<'a, E: ParseError<&'a str>>(
    i: &'a str,
) -> IResult<&'a str, (RawQuery<'a>, RawQuery<'a>), E> {
    separated_pair(parse_simple_query, alt((comma, or_separator)), parse_query)(i)
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

// not
fn one_shot_not<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, (), E> {
    value((), alt((tag_no_case("not "), tag("!"))))(i)
}

fn simple_not_clause<'a, E: ParseError<&'a str>>(i: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
    map(pair(one_shot_not, parse_simple_query), |(_, query)| query)(i)
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
        map(simple_not_clause, |q| RawQuery::Not(q.into())),
        parse_simple_query,
    ))(i)
}

#[cfg(test)]
mod test {
    use crate::parser::{and, comma, one_shot_not, or, parse_raw, simple_not_clause, RawQuery};
    use nom::error::VerboseError;

    #[test]
    fn test() {
        assert!(and::<VerboseError<&str>>("and").is_ok());
        assert!(and::<VerboseError<&str>>("&&").is_ok());
        assert!(or::<VerboseError<&str>>("or").is_ok());
        assert!(or::<VerboseError<&str>>("||").is_ok());
        assert!(comma::<VerboseError<&str>>(",").is_ok());
        assert!(comma::<VerboseError<&str>>(" ,").is_ok());
        assert!(comma::<VerboseError<&str>>(", ").is_ok());
        assert!(comma::<VerboseError<&str>>(" , ").is_ok());
        assert!(comma::<VerboseError<&str>>("  ,").is_ok());
        assert!(comma::<VerboseError<&str>>(",  ").is_ok());
        assert!(comma::<VerboseError<&str>>("  ,  ").is_ok());
        assert!(parse_raw::<VerboseError<&str>>("").is_err());
        assert_eq!(
            parse_raw::<VerboseError<&str>>("*").unwrap().1,
            RawQuery::Wildcard,
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("coucou_les-amis1234")
                .unwrap()
                .1,
            RawQuery::Pattern("coucou_les-amis1234"),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("field:pattern").unwrap().1,
            RawQuery::FieldPattern("field", Box::new(RawQuery::Pattern("pattern"))),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("field:*").unwrap().1,
            RawQuery::FieldPattern("field", Box::new(RawQuery::Wildcard)),
        );

        assert_eq!(
            parse_raw::<VerboseError<&str>>("field:sub_field:pattern")
                .unwrap()
                .1,
            RawQuery::FieldPattern(
                "field",
                Box::new(RawQuery::FieldPattern(
                    "sub_field",
                    Box::new(RawQuery::Pattern("pattern"))
                ))
            ),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("field:sub_field:*")
                .unwrap()
                .1,
            RawQuery::FieldPattern(
                "field",
                Box::new(RawQuery::FieldPattern(
                    "sub_field",
                    Box::new(RawQuery::Wildcard)
                ))
            ),
        );
        // one lvl
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo and bar").unwrap().1,
            RawQuery::And(
                Box::new(RawQuery::Pattern("foo")),
                Box::new(RawQuery::Pattern("bar"))
            ),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo or bar").unwrap().1,
            RawQuery::Or(
                Box::new(RawQuery::Pattern("foo")),
                Box::new(RawQuery::Pattern("bar"))
            ),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo , bar").unwrap().1,
            RawQuery::Or(
                Box::new(RawQuery::Pattern("foo")),
                Box::new(RawQuery::Pattern("bar"))
            ),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo,bar").unwrap().1,
            RawQuery::Or(
                Box::new(RawQuery::Pattern("foo")),
                Box::new(RawQuery::Pattern("bar"))
            ),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo, bar").unwrap().1,
            RawQuery::Or(
                Box::new(RawQuery::Pattern("foo")),
                Box::new(RawQuery::Pattern("bar"))
            ),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("w1.prod, w2.prod")
                .unwrap()
                .1,
            RawQuery::Or(
                Box::new(RawQuery::Pattern("w1.prod")),
                Box::new(RawQuery::Pattern("w2.prod"))
            ),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo ,bar").unwrap().1,
            RawQuery::Or(
                Box::new(RawQuery::Pattern("foo")),
                Box::new(RawQuery::Pattern("bar"))
            ),
        );

        // two lvl
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo and bar and yak")
                .unwrap()
                .1,
            RawQuery::And(
                Box::new(RawQuery::Pattern("foo")),
                Box::new(RawQuery::And(
                    Box::new(RawQuery::Pattern("bar")),
                    Box::new(RawQuery::Pattern("yak"))
                ))
            ),
        );
    }
    #[test]
    fn test_not() {
        assert!(one_shot_not::<VerboseError<&str>>("not ").is_ok());
        assert!(one_shot_not::<VerboseError<&str>>("!").is_ok());

        // basic not
        assert_eq!(
            simple_not_clause::<VerboseError<&str>>("not foobar")
                .unwrap()
                .1,
            RawQuery::Pattern("foobar")
        );
        assert_eq!(
            simple_not_clause::<VerboseError<&str>>("!foobar")
                .unwrap()
                .1,
            RawQuery::Pattern("foobar")
        );
        assert_eq!(
            simple_not_clause::<VerboseError<&str>>("not foobar:baz")
                .unwrap()
                .1,
            RawQuery::FieldPattern("foobar", Box::new(RawQuery::Pattern("baz")))
        );
        assert_eq!(
            simple_not_clause::<VerboseError<&str>>("!foobar:baz")
                .unwrap()
                .1,
            RawQuery::FieldPattern("foobar", Box::new(RawQuery::Pattern("baz")))
        );

        assert_eq!(
            parse_raw::<VerboseError<&str>>("not foobar").unwrap().1,
            RawQuery::Not(Box::new(RawQuery::Pattern("foobar")))
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("!foobar").unwrap().1,
            RawQuery::Not(Box::new(RawQuery::Pattern("foobar")))
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("not foobar:baz").unwrap().1,
            RawQuery::Not(Box::new(RawQuery::FieldPattern(
                "foobar",
                Box::new(RawQuery::Pattern("baz"))
            )))
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("!foobar:baz").unwrap().1,
            RawQuery::Not(Box::new(RawQuery::FieldPattern(
                "foobar",
                Box::new(RawQuery::Pattern("baz"))
            )))
        );
    }
}
