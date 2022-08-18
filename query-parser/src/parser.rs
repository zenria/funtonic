use super::Query as RawQuery;
use nom::branch::alt;
use nom::bytes::complete::{tag, tag_no_case, take_while, take_while1};
use nom::character::complete::{char, multispace1};
use nom::character::is_alphanumeric;
use nom::combinator::{complete, map, value};
use nom::error::ParseError;
use nom::sequence::{delimited, pair, preceded, separated_pair, terminated, tuple};
use nom::{Err, IResult};

impl<'a> RawQuery<'a> {
    /// RawQuery::Text variant builder
    fn pattern(text: &'a str) -> RawQuery<'a> {
        RawQuery::Pattern(text)
    }

    /// RawQuery::FieldText variant builder
    fn field_pattern(field: &'a str, pattern: RawQuery<'a>) -> RawQuery<'a> {
        RawQuery::FieldPattern(field, Box::new(pattern))
    }

    /// RawQuery::Not variant builder
    fn not(not: RawQuery<'a>) -> RawQuery<'a> {
        RawQuery::Not(Box::new(not))
    }
}

pub fn parse_raw<'a, E: ParseError<&'a str>>(
    i: &'a str,
) -> Result<(&'a str, RawQuery<'a>), Err<E>> {
    //parse_query(i)
    complete(parser_ng::expression)(i)
}

const SPACES: &'static str = " \t\r\n";
const SPECIAL_AUTHORIZED_CHARS: &'static str = "-_@#.";

mod parser_ng {
    use super::{RawQuery, SPECIAL_AUTHORIZED_CHARS};
    use nom::{
        branch::alt,
        bytes::complete::{is_not, tag, tag_no_case, take_while1},
        character::{
            complete::{alphanumeric1, char, digit1, multispace0, multispace1},
            is_alphanumeric,
        },
        combinator::map,
        error::ParseError,
        multi::{separated_list0, separated_list1},
        sequence::{delimited, preceded, separated_pair, terminated, tuple},
        IResult, Parser,
    };

    /// main entry point
    ///
    /// Or | Term
    pub(crate) fn expression<'a, E: ParseError<&'a str>>(
        input: &'a str,
    ) -> IResult<&'a str, RawQuery<'a>, E> {
        or(input)
    }

    fn or_tag<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, &'a str, E> {
        alt((tag_no_case("or"), tag("||"), tag(",")))(input)
    }

    /// Term "OR" Term
    fn or<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
        map(
            separated_list1(
                alt((
                    terminated(tag_no_case("or"), multispace1),
                    terminated(alt((tag("||"), tag(","))), multispace0),
                )),
                term,
            ),
            |clauses| {
                if clauses.len() == 1 {
                    clauses.into_iter().nth(0).unwrap()
                } else {
                    RawQuery::Or(clauses)
                }
            },
        )(input)
    }

    /// And | NotFactor
    fn term<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
        alt((and, not_factor))(input)
    }

    fn and_tags<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, &'a str, E> {
        alt((tag_no_case("and"), tag("&&")))(input)
    }

    /// NotFactor "AND" NotFactor
    fn and<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
        map(
            separated_list1(
                alt((
                    terminated(tag_no_case("and"), multispace1),
                    terminated(tag("&&"), multispace0),
                )),
                not_factor,
            ),
            |clauses| {
                if clauses.len() == 1 {
                    clauses.into_iter().nth(0).unwrap()
                } else {
                    RawQuery::And(clauses)
                }
            },
        )(input)
    }

    fn not_tags<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, &'a str, E> {
        alt((tag_no_case("not"), tag("!")))(input)
    }

    // "NOT" Factor | Factor
    fn not_factor<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
        alt((
            preceded(terminated(tag_no_case("not"), multispace1), factor).map(RawQuery::not),
            preceded(terminated(tag("!"), multispace0), factor).map(RawQuery::not),
            preceded(not_tags, parens).map(RawQuery::not),
            factor,
        ))(input)
    }

    /// Parens | Query
    fn factor<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, RawQuery, E> {
        alt((parens, query))(input)
    }

    /// "(" RawQueryession ")"
    fn parens<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
        delimited(
            terminated(char('('), multispace0),
            expression,
            terminated(char(')'), multispace0),
        )(input)
    }

    /// FieldText | Quoted | Word | Wildcard
    fn query<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
        alt((
            wildcard,
            field_text,
            quoted.map(RawQuery::Pattern),
            word.map(RawQuery::Pattern),
        ))(input)
    }

    fn field_name<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, &'a str, E> {
        take_while1(|c| is_alphanumeric(c as u8) || SPECIAL_AUTHORIZED_CHARS.contains(c))(input)
    }

    /// alpha1 ":" (Expression)
    fn field_text<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
        map(
            separated_pair(field_name, char(':'), factor),
            |(field_name, expr)| RawQuery::field_pattern(field_name, expr),
        )(input)
    }

    /// Single word
    fn word<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, &'a str, E> {
        terminated(is_not(" ():,&|"), multispace0)(input)
    }

    fn quoted<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, &'a str, E> {
        // TODO proper escaping
        terminated(delimited(char('"'), is_not("\""), char('"')), multispace0)(input)
    }

    fn wildcard<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, RawQuery<'a>, E> {
        map(terminated(char('*'), multispace0), |_| RawQuery::Wildcard)(input)
    }
}

#[cfg(test)]
mod test {
    use crate::parser::{parse_raw, RawQuery};
    use nom::error::VerboseError;

    #[test]
    fn test() {
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
            RawQuery::And(vec![RawQuery::Pattern("foo"), RawQuery::Pattern("bar")]),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo or bar").unwrap().1,
            RawQuery::Or(vec![RawQuery::Pattern("foo"), RawQuery::Pattern("bar")]),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo , bar").unwrap().1,
            RawQuery::Or(vec![RawQuery::Pattern("foo"), RawQuery::Pattern("bar")]),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo,bar").unwrap().1,
            RawQuery::Or(vec![RawQuery::Pattern("foo"), RawQuery::Pattern("bar")]),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo, bar").unwrap().1,
            RawQuery::Or(vec![RawQuery::Pattern("foo"), RawQuery::Pattern("bar")]),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("w1.prod, w2.prod")
                .unwrap()
                .1,
            RawQuery::Or(vec![
                RawQuery::Pattern("w1.prod"),
                RawQuery::Pattern("w2.prod")
            ]),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo ,bar").unwrap().1,
            RawQuery::Or(vec![RawQuery::Pattern("foo"), RawQuery::Pattern("bar")]),
        );

        // two lvl
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo and bar and yak")
                .unwrap()
                .1,
            RawQuery::And(vec![
                RawQuery::Pattern("foo"),
                RawQuery::Pattern("bar"),
                RawQuery::Pattern("yak")
            ]),
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo or bar or yak")
                .unwrap()
                .1,
            RawQuery::Or(vec![
                RawQuery::Pattern("foo"),
                RawQuery::Pattern("bar"),
                RawQuery::Pattern("yak")
            ]),
        );
    }
    #[test]
    fn test_not() {
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

        // comlpex not
        assert_eq!(
            parse_raw::<VerboseError<&str>>("not(foobar:baz)")
                .unwrap()
                .1,
            RawQuery::Not(Box::new(RawQuery::FieldPattern(
                "foobar",
                Box::new(RawQuery::Pattern("baz"))
            )))
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("!(foobar:baz)").unwrap().1,
            RawQuery::Not(Box::new(RawQuery::FieldPattern(
                "foobar",
                Box::new(RawQuery::Pattern("baz"))
            )))
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("not (foobar:baz)")
                .unwrap()
                .1,
            RawQuery::Not(Box::new(RawQuery::FieldPattern(
                "foobar",
                Box::new(RawQuery::Pattern("baz"))
            )))
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("! (foobar:baz)").unwrap().1,
            RawQuery::Not(Box::new(RawQuery::FieldPattern(
                "foobar",
                Box::new(RawQuery::Pattern("baz"))
            )))
        );

        assert_eq!(
            parse_raw::<VerboseError<&str>>("not( foobar:baz)")
                .unwrap()
                .1,
            RawQuery::Not(Box::new(RawQuery::FieldPattern(
                "foobar",
                Box::new(RawQuery::Pattern("baz"))
            )))
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("!( foobar:baz)").unwrap().1,
            RawQuery::Not(Box::new(RawQuery::FieldPattern(
                "foobar",
                Box::new(RawQuery::Pattern("baz"))
            )))
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("not(foobar:baz )")
                .unwrap()
                .1,
            RawQuery::Not(Box::new(RawQuery::FieldPattern(
                "foobar",
                Box::new(RawQuery::Pattern("baz"))
            )))
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("!(foobar:baz )").unwrap().1,
            RawQuery::Not(Box::new(RawQuery::FieldPattern(
                "foobar",
                Box::new(RawQuery::Pattern("baz"))
            )))
        );
    }

    #[test]
    fn test_precedence() {
        assert_eq!(
            parse_raw::<VerboseError<&str>>("env:qa or location:paris")
                .unwrap()
                .1,
            RawQuery::Or(vec![
                RawQuery::FieldPattern("env", Box::new(RawQuery::Pattern("qa"))),
                RawQuery::FieldPattern("location", Box::new(RawQuery::Pattern("paris")))
            ])
        );

        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo or bar and baz")
                .unwrap()
                .1,
            RawQuery::Or(vec![
                RawQuery::Pattern("foo"),
                RawQuery::And(vec![RawQuery::Pattern("bar"), RawQuery::Pattern("baz")])
            ])
        );
        assert_eq!(
            parse_raw::<VerboseError<&str>>("foo and bar or baz")
                .unwrap()
                .1,
            RawQuery::Or(vec![
                RawQuery::And(vec![RawQuery::Pattern("foo"), RawQuery::Pattern("bar"),]),
                RawQuery::Pattern("baz"),
            ])
        );
    }
}
