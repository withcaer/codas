//! Markdown lexical analysis and
//! tokenization utilities.
use alloc::vec;

use core::ops::Range;

use logos::{Logos, Span};

use super::{ParsedField, ParsedFieldType};

/// Enumeration of tokens that can be
/// parsed from Markdown.
#[derive(Logos, Debug, PartialEq)]
#[logos(subpattern space = r"[^\S\r\n]")]
#[logos(subpattern linebreak = r"[\r\n|\r|\n]+")]
#[logos(subpattern to_end_of_line = r"[^\r\n]*")]
#[logos(subpattern coda_id = r"[/:.a-zA-Z0-9_-]+")]
#[logos(subpattern data_id = r"[a-zA-Z0-9_-]+")]
#[logos(subpattern field_id = r"[a-zA-Z0-9_-]+")]
pub enum Token<'a> {
    /// ``# `The.Coda/Name` Coda``
    ///
    /// This token marks the beginning of
    /// a coda document, where `The.Coda/Name` is
    /// the name of the coda.
    ///
    /// Each tuple contains two strings:
    ///
    /// 1. The first string is the full name of the coda
    ///    as it is presented in the coda document.
    ///
    /// 2. The second string is the final section of the
    ///    name in its hierarchy (e.g., `Name` in the
    ///    example Coda above).
    ///
    /// If the coda name contains any hierarchy-defining
    /// characters (`.`, `/`, or `:`), only the _last_
    /// component in the path is guaranteed to be preserved
    /// as the type identifier for the coda during
    /// language-specific code generation.
    #[regex(r"[\r\n|\r|\n]*#(?&space)`(?&coda_id)`(?i)((?&space)coda)?", |lex| {
        let slice = lex.slice();

        let slice = slice.trim(); // trim whitespace
        let slice = &slice[1..]; // trim leading #
        let slice = slice.trim(); // trim whitespace

        // Slice should contain:
        // `The.Coda/Name` Coda
        let split = slice.split_whitespace();

        // Scan for the first slice in the split
        // containing ` characters; this is the
        // slice containing the coda's name.
        let mut name = slice;
        for next in split {
            if next.contains('`') {
                name = next;
                break;
            }
        }

        // Trim leading and trailing grave characters
        // to obtain the full name.
        let full_name = &name[1..name.len() - 1];

        // Split on all hierarchy characters to
        // obtain the local name.
        let mut local_name = full_name;
        for next in local_name.split(&['.', ':', '/']) {
            if !next.is_empty() {
                local_name = next;
            }
        }

        (full_name, local_name)

    })]
    Coda((&'a str, &'a str)),

    /// ``## `TheDataName` Data``
    ///
    /// This token marks the beginning of
    /// a data type, where `TheDataName` is
    /// the name of the specified type.
    #[regex(r"(?&linebreak)##(?&space)`(?&data_id)`(?&space)(?i)(data)", |lex| {
        let slice = lex.slice();

        let slice = slice.trim(); // trim whitespace
        let slice = &slice[2..]; // trim leading ##
        let slice = slice.trim(); // trim whitespace

        // Slice should contain:
        // `DataName` Data
        let mut split = slice.split_whitespace();
        let data_name = split.next().unwrap();

        // Trim leading and trailing grave characters.
        &data_name[1..data_name.len() - 1]
    })]
    Data(&'a str),

    /// ``+ `the_field_name` optional [N]d list of TheDataType``
    ///
    /// This token marks the beginning of a field in
    /// a data type, where `the_field_name` is the name
    /// of the field, and `TheDataType` is the type of
    /// the field.
    ///
    /// This token may _optionally_ contain the following
    /// keywords or sub-tokens:
    ///
    /// - `optional`, indicating the field is semantically optional.
    /// - `list of`, indicating the field is semantically a list.
    /// - `[N]d`, indicating the field is semantically a list with `N` dimensions
    ///
    /// `TheDataType` may optionally be written as a Markdown
    /// link, like: `[TheDataType](#link-to-the-datatype)`.
    #[regex(r"(?&linebreak)\+(?&space)`(?&field_id)`(?&space)(?&to_end_of_line)", |lex| {
        parse_data_field(lex.slice())
    })]
    DataField(ParsedField),

    /// Matches any non-token text on a newline, with
    /// zero or more leading spaces.
    #[regex(r"(?&linebreak)(?&to_end_of_line)", |lex| {
        let span = lex.span();
        let mut whitespace = 0;
        for c in lex.slice().chars() {
            match c {
                '\n' | '\r' => continue,
                ' ' | '\t' => whitespace += 1,
                _ => break,

            }
        }

        (lex.slice(), span, whitespace)
    })]
    DocsLine((&'a str, Span, usize)),
}

/// Enumeration of sub-tokens that can be parsed
/// from a [`Token::DataField`] token.
///
/// ``+ `the_field_name` optional map of TheDataType to TheDataType``
/// ``+ `the_field_name` optional [N]d list of TheDataType``
/// ``+ `the_field_name` optional TheDataType``
/// ``+ `the_field_name` map of TheDataType to TheDataType``
/// ``+ `the_field_name` [N]d list of TheDataType``
/// ``+ `the_field_name` TheDataType``
#[derive(Logos, Debug, PartialEq)]
#[logos(subpattern space = r"[^\S\r\n]")]
#[logos(subpattern linebreak = r"[\r\n|\r|\n]+")]
#[logos(subpattern data_id = r"[a-zA-Z0-9_-]+")]
#[logos(subpattern field_id = r"[a-zA-Z0-9_-]+")]
pub enum DataFieldToken<'a> {
    /// This token marks the beginning of a data
    /// field, containing its name.
    #[regex(r"(?&linebreak)\+(?&space)`(?&field_id)`(?&space)", |lex| {
        let slice = lex.slice();

        let slice = slice.trim(); // trim whitespace
        let slice = &slice[1..]; // trim leading +
        let slice = slice.trim(); // trim whitespace
        let slice = &slice[1..]; // trim leading `
        let slice = &slice[..slice.len() - 1]; // trim trailing `
        let slice = slice.trim(); // trim whitespace

        slice
    })]
    FieldName(&'a str),

    /// This token indicates a field is
    /// semantically optional
    #[regex(r"(?i)optional(?&space)")]
    Optional,

    /// This token indicates a field is
    /// semantically flattened.
    #[regex(r"(?i)flattened(?&space)")]
    Flattened,

    /// This token indicates a field is a fixed-size
    /// array containing the array's element count.
    ///
    /// If encountered, this token _should_ be followed
    /// by the array's element type.
    ///
    /// The count is validated (non-zero, at most [`u16::MAX`])
    /// during coda parsing, not lexing.
    #[regex(r"(?i)array(?&space)of(?&space)[0-9]+(?&space)", |lex| {
        let slice = lex.slice();

        // Slice contains: array of N
        let mut split = slice.split_whitespace();
        split.nth(2).and_then(|count| count.parse().ok()).unwrap_or(u64::MAX)
    })]
    Array(u64),

    /// This token indicates a field is
    /// semantically a list.
    ///
    /// The value inside the token is the
    /// number of _dimensions_ for the list,
    /// which must be at least `1`.
    #[regex(r"(?i)([0-9]+d(?&space))?list(?&space)of(?&space)", |lex| {
        let slice = lex.slice();
        let slice = slice.trim();

        // Check if the slice contains list dimensions.
        let mut split = slice.split_whitespace();
        if let Some(next) = split.next() {
            if next.ends_with('d') || next.ends_with('D') {
                let numeric = next.trim_end_matches(['d', 'D']);
                if let Ok(number) = numeric.parse() {
                    return number;
                }
            }
        }

        1
    })]
    List(usize),

    /// This token indicates a field is semantically a map.
    ///
    /// If encountered, this token _should_
    /// be followed by two [`FieldType`]s.
    #[regex(r"(?i)map(?&space)of(?&space)")]
    Map,

    /// This token contains the fully-qualified
    /// type of a field.
    #[regex(r"(?i)(to(?&space))?\[`(?&data_id)`\]\([^)]*\)", |lex| {
        let slice = lex.slice();

        // Strip off any leading `to `.
        let split = slice.split_whitespace();
        let slice = split.last().unwrap();

        // Trim leading [`.
        let slice = &slice[2..];

        // Find trailing ` and truncate the remainder.
        let end = slice.find('`').unwrap();
        &slice[..end]
    })]
    #[regex(r"(?i)(to(?&space))?(?&data_id)", |lex| {
        let slice = lex.slice();

        // Strip off any leading `to `.
        let split = slice.split_whitespace();
        split.last().unwrap()
    })]
    FieldType(&'a str),
}

/// Parser for a [`Token::DataField`] via a [`DataFieldToken`].
fn parse_data_field(slice: &str) -> ParsedField {
    let lexer = DataFieldToken::lexer(slice);

    let mut name = slice;
    let mut optional = false;
    let mut flattened = false;
    let mut tokens = vec![];

    for token in lexer.filter_map(|t| t.ok()) {
        match token {
            DataFieldToken::FieldName(field_name) => name = field_name,
            DataFieldToken::Optional => optional = true,
            DataFieldToken::Flattened => flattened = true,
            token => tokens.push(token),
        }
    }

    let mut tokens = tokens.into_iter();
    let typing = parse_field_type(&mut tokens);

    // Any tokens remaining after a complete type are a mistake.
    if let Some(token) = tokens.next() {
        todo!("malformed field: unexpected {token:?}");
    }

    ParsedField {
        name: name.into(),
        docs: Range::default(),
        typing,
        optional,
        flattened,
    }
}

/// Recursively parses the next complete
/// [`ParsedFieldType`] from `tokens`.
fn parse_field_type<'a>(tokens: &mut impl Iterator<Item = DataFieldToken<'a>>) -> ParsedFieldType {
    match tokens.next() {
        // A scalar.
        Some(DataFieldToken::FieldType(type_name)) => ParsedFieldType::Scalar(type_name.into()),

        // An array of some other type.
        Some(DataFieldToken::Array(count)) => {
            ParsedFieldType::Array(count, parse_field_type(tokens).into())
        }

        // A list of some other type.
        Some(DataFieldToken::List(dimensions)) => {
            ParsedFieldType::List(dimensions, parse_field_type(tokens).into())
        }

        // A map of one scalar type to another.
        Some(DataFieldToken::Map) => {
            let key_typing = match tokens.next() {
                Some(DataFieldToken::FieldType(type_name)) => type_name,
                token => todo!("malformed field: expected a map key type, found {token:?}"),
            };
            let value_typing = match tokens.next() {
                Some(DataFieldToken::FieldType(type_name)) => type_name,
                token => todo!("malformed field: expected a map value type, found {token:?}"),
            };
            ParsedFieldType::Map(key_typing.into(), value_typing.into())
        }

        // A mistake.
        token => todo!("malformed field: expected a type, found {token:?}"),
    }
}
