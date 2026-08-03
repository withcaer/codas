//! Coda Markdown parser.
//!
//! # Unstable
//!
//! The APIs exposed by this module are _primarily_
//! for use by automated tooling (macros, CLIs, etc.);
//! the exact APIs are subject to change, and may
//! not be well-optimized.

use alloc::boxed::Box;
use core::{iter::Peekable, ops::Range};

use logos::{Lexer, Logos};
use snafu::Snafu;
use token::Token;

use crate::{
    codec::Format,
    types::{Coda, DataField, DataType, Text, Type},
};

mod token;

/// Maximum user-defined ordinal value.
///
/// Built-in types occupy a range of ordinals descending from 255.
const MAX_USER_ORDINAL: u8 = 240;

/// Parses `markdown` into a [`Coda`].
pub fn parse(markdown: &str) -> Result<Coda, ParseError> {
    // Parse the raw coda from the markdown.
    let markdown = markdown.trim();
    let mut parser = Parser::new(markdown);
    let parsed_coda = parser.parse()?;

    // Prepare an in-memory coda.
    let docs = if parsed_coda.docs.is_empty() {
        None
    } else {
        Some(markdown[parsed_coda.docs].trim().into())
    };
    let mut coda = Coda::new(parsed_coda.global_name, parsed_coda.local_name, docs, &[]);

    // Create data types.
    for (ordinal, parsed_data) in parsed_coda.data.into_iter().enumerate() {
        // User-defined ordinals start at 1 (0 reserved for Unspecified).
        let ordinal = ordinal + 1;
        if ordinal > MAX_USER_ORDINAL as usize {
            return Err(ParseError::TooManyDataTypes {
                max: MAX_USER_ORDINAL,
            });
        }
        let ordinal = ordinal as u8;

        // Extract docs.
        let docs = if parsed_data.docs.is_empty() {
            None
        } else {
            Some(markdown[parsed_data.docs].trim().into())
        };

        // Extract fields.
        let mut data = DataType::new(parsed_data.name, docs, ordinal, &[], &[]);
        for parsed_field in parsed_data.fields {
            // Extract docs.
            let docs = if parsed_field.docs.is_empty() {
                None
            } else {
                Some(markdown[parsed_field.docs].trim().into())
            };

            // Extract typing.
            let typing = resolve_field_type(&coda, parsed_field.typing)?;

            // Non-optional fixed-size fields grow the type's blob section,
            // which is capped at u16::MAX bytes.
            if !parsed_field.optional {
                if let Format::Blob(size) = typing.format() {
                    let blob_size = data.format().as_data_format().blob_size as u32 + size as u32;
                    if blob_size > u16::MAX as u32 {
                        return Err(ParseError::BlobSectionOverflow { data: data.name });
                    }
                }
            }

            data = data.with(DataField {
                name: parsed_field.name,
                docs,
                typing,
                optional: parsed_field.optional,
                flattened: parsed_field.flattened,
            });
        }

        coda.data.push(data);
    }

    Ok(coda)
}

/// Recursively resolves a [`ParsedFieldType`] into a [`Type`],
/// using `coda` to resolve type names.
fn resolve_field_type(coda: &Coda, typing: ParsedFieldType) -> Result<Type, ParseError> {
    match typing {
        ParsedFieldType::Scalar(name) => Ok(match coda.type_from_name(&name) {
            Some(typing) => typing,
            None => Type::Data(DataType::new_fluid(name, None)),
        }),

        ParsedFieldType::List(dimensions, inner) => {
            let mut typing = resolve_field_type(coda, *inner)?;
            for _ in 0..dimensions {
                typing = Type::List(typing.into());
            }
            Ok(typing)
        }

        ParsedFieldType::Map(key_name, value_name) => {
            let key_typing = resolve_field_type(coda, ParsedFieldType::Scalar(key_name))?;
            let value_typing = resolve_field_type(coda, ParsedFieldType::Scalar(value_name))?;
            Ok(Type::Map((key_typing, value_typing).into()))
        }

        ParsedFieldType::Array(count, elem) => {
            if count == 0 || count > u16::MAX as u64 {
                return Err(ParseError::InvalidArraySize { count });
            }

            // Array elements must have a fixed size, and the
            // array's total size must fit in a blob section.
            let elem = resolve_field_type(coda, *elem)?;
            match elem.format() {
                Format::Blob(size) => {
                    let bytes = count as u32 * size as u32;
                    if bytes > u16::MAX as u32 {
                        return Err(ParseError::ArrayTooLarge { bytes });
                    }
                }
                _ => return Err(ParseError::ArrayElementNotFixedSize),
            }

            Ok(Type::Array(count as u16, elem.into()))
        }
    }
}

/// A Markdown parser for codas.
struct Parser<'lexer> {
    /// The token lexer being parsed.
    lexer: Peekable<Lexer<'lexer, Token<'lexer>>>,
}

impl<'lexer> Parser<'lexer> {
    /// Creates a new parser for `text`.
    fn new(text: &'lexer str) -> Self {
        Self {
            lexer: Token::lexer(text).peekable(),
        }
    }

    /// Parses the next [`Coda`] from the text.
    fn parse(&mut self) -> Result<ParsedCoda, ParseError> {
        Ok(self.take_coda()?.unwrap())
    }

    /// Takes the next [`Token::Coda`].
    fn take_coda(&mut self) -> Result<Option<ParsedCoda>, ParseError> {
        let name = match self.lexer.next() {
            Some(Ok(Token::Coda(name))) => name,
            _ => return Err(ParseError::ExpectedCoda),
        };

        let mut coda = ParsedCoda {
            global_name: name.0.into(),
            local_name: name.1.into(),
            docs: 0..0,
            data: alloc::vec![],
        };

        // Parse docs.
        let (docs, whitespace) = self.take_docs_lines()?;
        assert!(docs.is_empty() || whitespace == 0);
        if !docs.is_empty() && whitespace != 0 {
            return Err(ParseError::UnexpectedDocsIndentation { actual: whitespace });
        }
        coda.docs = docs;

        // Parse data types.
        while let Some(data_type) = self.take_data()? {
            coda.data.push(data_type);
        }

        Ok(Some(coda))
    }

    /// Takes the next [`Token::Data`].
    fn take_data(&mut self) -> Result<Option<ParsedDataType>, ParseError> {
        let name = match self.lexer.peek() {
            Some(Ok(Token::Data(name))) => {
                let name = (*name).into();
                self.lexer.next();
                name
            }
            None | Some(Ok(..)) => return Ok(None),
            _ => return Err(ParseError::ExpectedDataType),
        };

        let mut data_type = ParsedDataType {
            name,
            docs: 0..0,
            fields: alloc::vec![],
        };

        // Parse the data's docs.
        let (docs, whitespace) = self.take_docs_lines()?;
        if !docs.is_empty() && whitespace != 0 {
            return Err(ParseError::UnexpectedDocsIndentation { actual: whitespace });
        }
        data_type.docs = docs;

        // Parse the data's fields.
        while let Some(data_field) = self.take_data_field()? {
            data_type.fields.push(data_field);
        }

        Ok(Some(data_type))
    }

    /// Takes the next [`Token::DataField`].
    fn take_data_field(&mut self) -> Result<Option<ParsedField>, ParseError> {
        let mut field = match self.lexer.peek() {
            Some(Ok(Token::DataField(field))) => {
                let field = field.clone();
                self.lexer.next();
                field
            }
            None | Some(Ok(..)) => return Ok(None),
            _ => return Err(ParseError::ExpectedDataField),
        };

        // Parse the fields' docs.
        let (docs, whitespace) = self.take_docs_lines()?;
        if !docs.is_empty() && whitespace == 0 {
            return Err(ParseError::ExpectedDocsIndentation {
                minimum_expected: 1,
            });
        }
        field.docs = docs;

        Ok(Some(field))
    }

    /// Takes the next contiguous set of [`Token::DocsLine`]s
    /// with the same level of leading whitespace.
    fn take_docs_lines(&mut self) -> Result<(Range<usize>, usize), ParseError> {
        let mut leading_whitespace = 0;
        let mut range = 0..0;

        while let Some(token) = self.lexer.peek() {
            match token {
                Ok(Token::DocsLine((line, line_range, line_whitespace))) => {
                    // Init.
                    if range.is_empty() {
                        range = line_range.clone();
                        leading_whitespace = *line_whitespace;
                        self.lexer.next();
                        continue;
                    }

                    // Iter.
                    if line == &"\n" || line == &"\r" || *line_whitespace >= leading_whitespace {
                        range.end = line_range.end;
                        self.lexer.next();
                        continue;
                    }

                    // Done.
                    break;
                }

                Ok(..) => break,

                _ => return Err(ParseError::UnexpectedError),
            }
        }

        Ok((range, leading_whitespace))
    }
}

/// [`Coda`] parsed from text.
#[derive(Clone, Debug, PartialEq)]
struct ParsedCoda {
    global_name: Text,
    local_name: Text,
    docs: Range<usize>,
    data: alloc::vec::Vec<ParsedDataType>,
}

/// [`DataType`] parsed from text.
#[derive(Clone, Debug, PartialEq)]
struct ParsedDataType {
    name: Text,
    docs: Range<usize>,
    fields: alloc::vec::Vec<ParsedField>,
}

/// [`DataField`] parsed from text.
#[derive(Clone, Debug, PartialEq)]
struct ParsedField {
    name: Text,

    /// The span of the lexer's contents
    /// containing the field's docs.
    docs: Range<usize>,

    /// The parsed (but unresolved) typing.
    typing: ParsedFieldType,

    /// True if the field is optional.
    optional: bool,

    /// True if the field is flattened.
    flattened: bool,
}

/// Unresolved typing of a [`ParsedField`].
#[derive(Clone, Debug, PartialEq)]
enum ParsedFieldType {
    /// A single value of one type.
    Scalar(Text),

    /// A fixed-size array of values of one type.
    Array(u64, Box<ParsedFieldType>),

    /// An N-dimensional list of values of one type.
    List(usize, Box<ParsedFieldType>),

    /// A mapping of one type to another.
    Map(Text, Text),
}

/// Enumeration of errors that may occur when parsing codas.
#[derive(Debug, Snafu)]
pub enum ParseError {
    #[snafu(display("Expected to parse a Coda header."))]
    ExpectedCoda,

    #[snafu(display("Expected to parse a Data type header."))]
    ExpectedDataType,

    #[snafu(display("Expected to parse a Data Field."))]
    ExpectedDataField,

    #[snafu(display(
        "Expected to parse docs with no spaces of indentation, instead of {actual}."
    ))]
    UnexpectedDocsIndentation { actual: usize },

    #[snafu(display(
        "Expected to parse docs with at least {minimum_expected} space(s) of indentation, not 0."
    ))]
    ExpectedDocsIndentation { minimum_expected: usize },

    #[snafu(display("Too many data types: maximum is {max}."))]
    TooManyDataTypes { max: u8 },

    #[snafu(display("Arrays must have between 1 and 65535 elements, not {count}."))]
    InvalidArraySize { count: u64 },

    #[snafu(display("An array's total size must be at most 65535 bytes, not {bytes}."))]
    ArrayTooLarge { bytes: u32 },

    #[snafu(display(
        "Array elements must be fixed-size types (numbers, bools, or other arrays)."
    ))]
    ArrayElementNotFixedSize,

    #[snafu(display("The combined size of `{data}`'s fixed-size fields exceeds 65535 bytes."))]
    BlobSectionOverflow { data: Text },

    #[snafu(display("An unexpected error occurred while parsing the source text."))]
    UnexpectedError,
}

#[cfg(test)]
pub(crate) mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    pub const TEST_CODA_MARKDOWN: &str = r#"
# `MyCoda` Coda

An example Markdown Coda.

## `MyNestedDataType` Data

A data type for nesting inside [`MyDataType`].

+ `floaty_field` f32

    A 32-bit floating-point field.

+ `listy_field` list of text

    A list of data with the same type.

    > This field has some fancy nested docs, too.

    Like, _really_ fancy. <3

## `MyDataType` Data

An example Markdown Data Type.

+ `textual_field` text

    A UTF-8 encoded text field.

+ `integral_field` i32

    A 32-bit signed integer field.

+ `nested_field` [`MyNestedDataType`](#mynesteddatatype-data)

    A nested field.

+ `optional_field` optional u64

    A semantically optional `u64` field.

+ `3d_field` 3d list of i32

    A field containing a 3-dimensional list of numbers.

+ `map_field` map of text to i32

    A field containing a map of text to numbers.

+ `unspecified_field` unspecified

    A field with unspecified typing.

+ `array_field` array of 16 u8

    A field containing 16 raw bytes.

+ `matrix_field` array of 3 array of 3 f32

    A field containing a 3x3 matrix.

+ `frames_field` list of array of 3 f32

    A field containing a list of 3-float frames.
"#;

    #[test]
    fn parses_markdown_into_coda() -> Result<(), ParseError> {
        let coda = parse(TEST_CODA_MARKDOWN)?;

        let mut expected = Coda::new(
            "MyCoda".into(),
            "MyCoda".into(),
            Some("An example Markdown Coda.".into()),
            &[],
        );

        // The "MyNestedDataType" spec.
        let nested_data_type = DataType::new(
            "MyNestedDataType".into(),
            Some("A data type for nesting inside [`MyDataType`].".into()),
            1,
            &[],
            &[],
        )
        .with(DataField {
            name: "floaty_field".into(),
            docs: Some("A 32-bit floating-point field.".into()),
            typing: Type::F32,
            optional: false,
            flattened: false,
        })
        .with(DataField {
            name: "listy_field".into(),
            docs: Some("A list of data with the same type.\n\n    > This field has some fancy nested docs, too.\n\n    Like, _really_ fancy. <3".into()),
            typing: Type::List(Type::Text.into()),
            optional: false,
            flattened: false,
        });
        expected.data.push(nested_data_type.clone());

        // The "MyDataType" spec.
        expected.data.push(
            DataType::new(
                "MyDataType".into(),
                Some("An example Markdown Data Type.".into()),
                2,
                &[],
                &[],
            )
            .with(DataField {
                name: "textual_field".into(),
                docs: Some("A UTF-8 encoded text field.".into()),
                typing: Type::Text,
                optional: false,
                flattened: false,
            })
            .with(DataField {
                name: "integral_field".into(),
                docs: Some("A 32-bit signed integer field.".into()),
                typing: Type::I32,
                optional: false,
                flattened: false,
            })
            .with(DataField {
                name: "nested_field".into(),
                docs: Some("A nested field.".into()),
                typing: Type::Data(nested_data_type),
                optional: false,
                flattened: false,
            })
            .with(DataField {
                name: "optional_field".into(),
                docs: Some("A semantically optional `u64` field.".into()),
                typing: Type::U64,
                optional: true,
                flattened: false,
            })
            .with(DataField {
                name: "3d_field".into(),
                docs: Some("A field containing a 3-dimensional list of numbers.".into()),
                typing: Type::List(Type::List(Type::List(Type::I32.into()).into()).into()),
                optional: false,
                flattened: false,
            })
            .with(DataField {
                name: "map_field".into(),
                docs: Some("A field containing a map of text to numbers.".into()),
                typing: Type::Map((Type::Text, Type::I32).into()),
                optional: false,
                flattened: false,
            })
            .with(DataField {
                name: "unspecified_field".into(),
                docs: Some("A field with unspecified typing.".into()),
                typing: Type::Unspecified,
                optional: false,
                flattened: false,
            })
            .with(DataField {
                name: "array_field".into(),
                docs: Some("A field containing 16 raw bytes.".into()),
                typing: Type::Array(16, Type::U8.into()),
                optional: false,
                flattened: false,
            })
            .with(DataField {
                name: "matrix_field".into(),
                docs: Some("A field containing a 3x3 matrix.".into()),
                typing: Type::Array(3, Type::Array(3, Type::F32.into()).into()),
                optional: false,
                flattened: false,
            })
            .with(DataField {
                name: "frames_field".into(),
                docs: Some("A field containing a list of 3-float frames.".into()),
                typing: Type::List(Type::Array(3, Type::F32.into()).into()),
                optional: false,
                flattened: false,
            }),
        );

        assert_eq!(expected, coda);

        Ok(())
    }

    #[test]
    fn parses_markdown_into_intermediate_representation() -> Result<(), ParseError> {
        let mut parser = Parser::new(TEST_CODA_MARKDOWN);
        let coda = parser.parse()?;

        assert_eq!("MyCoda", coda.global_name);
        assert_eq!("MyCoda", coda.local_name);
        assert_eq!(
            "An example Markdown Coda.",
            TEST_CODA_MARKDOWN[coda.docs].trim()
        );

        // Check first data.
        let data = &coda.data[0];
        assert_eq!("MyNestedDataType", data.name);
        assert_eq!(
            "A data type for nesting inside [`MyDataType`].",
            TEST_CODA_MARKDOWN[data.docs.clone()].trim()
        );
        let field = &data.fields[0];
        assert_eq!("floaty_field", field.name);
        assert_eq!(
            "A 32-bit floating-point field.",
            TEST_CODA_MARKDOWN[field.docs.clone()].trim()
        );
        assert_eq!(ParsedFieldType::Scalar("f32".into()), field.typing);
        assert!(!field.optional);
        let field = &data.fields[1];
        assert_eq!("listy_field", field.name);
        assert_eq!(
            r#"A list of data with the same type.

    > This field has some fancy nested docs, too.

    Like, _really_ fancy. <3"#,
            TEST_CODA_MARKDOWN[field.docs.clone()].trim()
        );
        assert_eq!(
            ParsedFieldType::List(1, ParsedFieldType::Scalar("text".into()).into()),
            field.typing
        );
        assert!(!field.optional);

        // Check second data.
        let data = &coda.data[1];
        assert_eq!("MyDataType", data.name);
        assert_eq!(
            "An example Markdown Data Type.",
            TEST_CODA_MARKDOWN[data.docs.clone()].trim()
        );

        let field = &data.fields[0];
        assert_eq!("textual_field", field.name);
        assert_eq!(
            "A UTF-8 encoded text field.",
            TEST_CODA_MARKDOWN[field.docs.clone()].trim()
        );
        assert_eq!(ParsedFieldType::Scalar("text".into()), field.typing);
        assert!(!field.optional);

        let field = &data.fields[1];
        assert_eq!("integral_field", field.name);
        assert_eq!(
            "A 32-bit signed integer field.",
            TEST_CODA_MARKDOWN[field.docs.clone()].trim()
        );
        assert_eq!(ParsedFieldType::Scalar("i32".into()), field.typing);
        assert!(!field.optional);

        let field: &ParsedField = &data.fields[2];
        assert_eq!("nested_field", field.name);
        assert_eq!(
            "A nested field.",
            TEST_CODA_MARKDOWN[field.docs.clone()].trim()
        );
        assert_eq!(
            ParsedFieldType::Scalar("MyNestedDataType".into()),
            field.typing
        );
        assert!(!field.optional);

        let field: &ParsedField = &data.fields[3];
        assert_eq!("optional_field", field.name);
        assert_eq!(
            "A semantically optional `u64` field.",
            TEST_CODA_MARKDOWN[field.docs.clone()].trim()
        );
        assert_eq!(ParsedFieldType::Scalar("u64".into()), field.typing);
        assert!(field.optional);

        let field: &ParsedField = &data.fields[4];
        assert_eq!("3d_field", field.name);
        assert_eq!(
            "A field containing a 3-dimensional list of numbers.",
            TEST_CODA_MARKDOWN[field.docs.clone()].trim()
        );
        assert_eq!(
            ParsedFieldType::List(3, ParsedFieldType::Scalar("i32".into()).into()),
            field.typing
        );
        assert!(!field.optional);

        let field: &ParsedField = &data.fields[5];
        assert_eq!("map_field", field.name);
        assert_eq!(
            "A field containing a map of text to numbers.",
            TEST_CODA_MARKDOWN[field.docs.clone()].trim()
        );
        assert_eq!(
            ParsedFieldType::Map("text".into(), "i32".into()),
            field.typing
        );
        assert!(!field.optional);

        let field: &ParsedField = &data.fields[6];
        assert_eq!("unspecified_field", field.name);
        assert_eq!(
            "A field with unspecified typing.",
            TEST_CODA_MARKDOWN[field.docs.clone()].trim()
        );
        assert_eq!(ParsedFieldType::Scalar("unspecified".into()), field.typing);
        assert!(!field.optional);

        let field: &ParsedField = &data.fields[7];
        assert_eq!("array_field", field.name);
        assert_eq!(
            "A field containing 16 raw bytes.",
            TEST_CODA_MARKDOWN[field.docs.clone()].trim()
        );
        assert_eq!(
            ParsedFieldType::Array(16, ParsedFieldType::Scalar("u8".into()).into()),
            field.typing
        );
        assert!(!field.optional);

        let field: &ParsedField = &data.fields[8];
        assert_eq!("matrix_field", field.name);
        assert_eq!(
            ParsedFieldType::Array(
                3,
                ParsedFieldType::Array(3, ParsedFieldType::Scalar("f32".into()).into()).into()
            ),
            field.typing
        );

        let field: &ParsedField = &data.fields[9];
        assert_eq!("frames_field", field.name);
        assert_eq!(
            ParsedFieldType::List(
                1,
                ParsedFieldType::Array(3, ParsedFieldType::Scalar("f32".into()).into()).into()
            ),
            field.typing
        );

        Ok(())
    }

    #[test]
    fn rejects_invalid_array_fields() {
        // Zero-element arrays are invalid.
        let error = parse("# `Test` Coda\n\n## `Data` Data\n\n+ `f` array of 0 u8\n");
        assert!(
            matches!(error, Err(ParseError::InvalidArraySize { count: 0 })),
            "expected InvalidArraySize, got {error:?}"
        );

        // Arrays can't have more than u16::MAX elements.
        let error = parse("# `Test` Coda\n\n## `Data` Data\n\n+ `f` array of 65536 u8\n");
        assert!(
            matches!(error, Err(ParseError::InvalidArraySize { count: 65536 })),
            "expected InvalidArraySize, got {error:?}"
        );

        // Arrays can't total more than u16::MAX bytes.
        let error = parse("# `Test` Coda\n\n## `Data` Data\n\n+ `f` array of 40000 u32\n");
        assert!(
            matches!(error, Err(ParseError::ArrayTooLarge { bytes: 160000 })),
            "expected ArrayTooLarge, got {error:?}"
        );

        // Array elements must be fixed-size types.
        let error = parse("# `Test` Coda\n\n## `Data` Data\n\n+ `f` array of 3 text\n");
        assert!(
            matches!(error, Err(ParseError::ArrayElementNotFixedSize)),
            "expected ArrayElementNotFixedSize, got {error:?}"
        );

        // A type's blob section can't exceed u16::MAX bytes.
        let error = parse(
            "# `Test` Coda\n\n## `Data` Data\n\n+ `f1` array of 40000 u8\n\n+ `f2` array of 40000 u8\n",
        );
        assert!(
            matches!(error, Err(ParseError::BlobSectionOverflow { .. })),
            "expected BlobSectionOverflow, got {error:?}"
        );
    }

    #[test]
    fn parses_optional_arrays() -> Result<(), ParseError> {
        let coda = parse("# `Test` Coda\n\n## `Data` Data\n\n+ `f` optional array of 64 u8\n")?;
        let field = coda.data[0].iter().next().unwrap();
        assert_eq!(Type::Array(64, Type::U8.into()), field.typing);
        assert!(field.optional);

        // Optional arrays are wrapped into a data field
        // for presence signaling, instead of growing
        // the blob section.
        assert_eq!(
            Format::Data(crate::codec::DataFormat {
                blob_size: 0,
                data_fields: 1,
                ordinal: 1,
            }),
            *coda.data[0].format()
        );

        Ok(())
    }

    #[test]
    fn parses_coda_local_names() -> Result<(), ParseError> {
        // Test without Coda suffix.
        let mut parser = Parser::new("# `codas.dev:names/local/Test`");
        let coda = parser.parse()?;
        assert_eq!("codas.dev:names/local/Test", coda.global_name);
        assert_eq!("Test", coda.local_name);

        // Test with Coda suffix.
        let mut parser = Parser::new("# `codas.dev:names/local/Test` Coda");
        let coda = parser.parse()?;
        assert_eq!("codas.dev:names/local/Test", coda.global_name);
        assert_eq!("Test", coda.local_name);

        Ok(())
    }
}
