//! OpenAPI schema generators.
//!
//! ## What's Here
//!
//! For a given coda, the following OpenAPI schema YAML
//! will be generated:
//!
//! - A `components.schemas` list, containing a schema
//!   for each data type.
//!
//! - A schema for the coda, defined as oneof any of the
//!   data types' schemas.
//!
//! Generated schemas can be validated manually via
//! [Swagger's OpenAPI editor](https://editor.swagger.io).
use core::fmt::Write;

use alloc::boxed::Box;
use alloc::format;

use indoc::writedoc;

use crate::{
    stream::{FmtWriter, StreamError, Writes},
    types::{Coda, Text, Type, Unspecified},
};

/// Number of spaces used for indenting
/// each level of a YAML document.
const YAML_INDENTATION_STEP: usize = 2;

/// Generates the OpenAPI spec for `coda`.
pub fn generate_spec(coda: &Coda, stream: &mut impl Writes) -> Result<(), StreamError> {
    // Extract coda metadata.
    let coda_type_name = format!("{}Data", coda.local_name.trim());
    let coda_type_docs = match &coda.docs {
        Some(docs) => docs.trim(),
        None => "Undocumented Coda. How could you? ;~;",
    };

    let mut writer = FmtWriter::from(stream);

    // Generate info header.
    let _ = writedoc!(
        writer,
        r#"
    openapi: 3.0.3
    info:
      title: {coda_type_name}
      version: 0.0.1
      description: |-
    "#
    );

    // Generate top-level docs.
    for line in coda_type_docs.lines() {
        write_indentation(&mut writer, 4)?;
        let _ = writeln!(writer, "{line}");
    }

    // Generate components.schemas header.
    let _ = writedoc!(
        writer,
        r#"
    paths: {{}}
    components:
      schemas:
    "#
    );

    // Generate data type schemas.
    for data_type in [Unspecified::DATA_TYPE].iter().chain(coda.iter()) {
        let data_type_name = &data_type.name;
        let data_type_docs = match &data_type.docs {
            Some(docs) => docs.trim(),
            None => "Undocumented Data. How could you? ;~;",
        };

        // Generate type header.
        let _ = writeln!(writer, "    {data_type_name}:");
        let _ = writeln!(writer, "      type: object");

        // Generate required field list.
        if data_type.iter().any(|f| !f.optional) {
            let _ = writeln!(writer, "      required:");
            for field in data_type.iter().filter(|f| !f.optional) {
                let _ = writeln!(writer, "        - {}", field.name);
            }
        }

        // Generate type docs.
        let _ = writeln!(writer, "      description: |-");
        for line in data_type_docs.lines() {
            write_indentation(&mut writer, 8)?;
            let _ = writeln!(writer, "{line}");
        }

        // Generate type fields.
        if data_type.iter().count() > 0 {
            let _ = writeln!(writer, "      properties:");
        }
        for field in data_type.iter() {
            let field_name = &field.name;
            let field_docs = match &field.docs {
                Some(docs) => docs.trim(),
                None => "Undocumented Field. How could you? ;~;",
            };

            // Generate field header.
            let _ = writeln!(writer, "        {field_name}:");

            let field_type = open_api_type(&field.typing);

            // Generate field docs for fields
            // that _aren't_ references.
            if !matches!(field_type, OpenApiTypeIdentifier::ObjectReference(..)) {
                let _ = writeln!(writer, "          description: |-");
                for line in field_docs.lines() {
                    write_indentation(&mut writer, 12)?;
                    let _ = writeln!(writer, "{line}");
                }
            }

            // Generate field typing.
            field_type.write_yaml(&mut writer, 10)?;
        }
    }

    // Generate coda schema.
    let _ = writeln!(writer, "    {coda_type_name}:");
    let _ = writeln!(writer, "      oneOf:");
    for data_type in [Unspecified::DATA_TYPE].iter().chain(coda.iter()) {
        let data_type_name = &data_type.name;
        let _ = writeln!(
            writer,
            "        - $ref: '#/components/schemas/{data_type_name}'"
        );
    }

    Ok(())
}

/// Returns the native OpenAPI identifier of `type`.
///
/// If `type` is a [`codas::spec::Type::Data`], the
/// data's name will be interpereted as a
/// native Typescript identifier.
fn open_api_type(typing: &Type) -> OpenApiTypeIdentifier {
    match typing {
        Type::Unspecified => OpenApiTypeIdentifier::Unformatted(Text::Static("object")),
        Type::U8 => OpenApiTypeIdentifier::Unformatted(Text::Static("integer")),
        Type::U16 => OpenApiTypeIdentifier::Unformatted(Text::Static("integer")),
        Type::U32 => OpenApiTypeIdentifier::Unformatted(Text::Static("integer")),
        Type::U64 => OpenApiTypeIdentifier::Unformatted(Text::Static("integer")),
        Type::I8 => OpenApiTypeIdentifier::Unformatted(Text::Static("integer")),
        Type::I16 => OpenApiTypeIdentifier::Unformatted(Text::Static("integer")),
        Type::I32 => {
            OpenApiTypeIdentifier::Formatted(Text::Static("integer"), Text::Static("int32"))
        }
        Type::I64 => {
            OpenApiTypeIdentifier::Formatted(Text::Static("integer"), Text::Static("int64"))
        }
        Type::F32 => {
            OpenApiTypeIdentifier::Formatted(Text::Static("number"), Text::Static("float"))
        }
        Type::F64 => {
            OpenApiTypeIdentifier::Formatted(Text::Static("number"), Text::Static("double"))
        }
        Type::Bool => OpenApiTypeIdentifier::Unformatted(Text::Static("boolean")),
        Type::Text => OpenApiTypeIdentifier::Unformatted(Text::Static("string")),
        Type::Array(_, elem) => {
            let elem = open_api_type(elem.as_ref());
            OpenApiTypeIdentifier::Array(elem.into())
        }
        Type::Data(typing) => OpenApiTypeIdentifier::ObjectReference(typing.name.clone()),
        Type::List(typing) => {
            let typing = open_api_type(typing.as_ref());
            OpenApiTypeIdentifier::Array(typing.into())
        }
        Type::Map(typing) => {
            let key_typing = open_api_type(&typing.as_ref().0);
            let value_typing = open_api_type(&typing.as_ref().1);
            OpenApiTypeIdentifier::Map((key_typing, value_typing).into())
        }
    }
}

/// Return value of [`open_api_type`].
enum OpenApiTypeIdentifier {
    Unformatted(Text),
    Formatted(Text, Text),
    ObjectReference(Text),
    Array(Box<OpenApiTypeIdentifier>),
    Map(Box<(OpenApiTypeIdentifier, OpenApiTypeIdentifier)>),
}

impl OpenApiTypeIdentifier {
    /// Writes the YAML representation of this identifier to `writer`
    /// with initial leading `indentation`.
    pub fn write_yaml<W: Writes>(
        &self,
        writer: &mut FmtWriter<'_, W>,
        indentation: usize,
    ) -> Result<(), StreamError> {
        match self {
            OpenApiTypeIdentifier::Unformatted(typing) => {
                write_indentation(writer, indentation)?;
                let _ = writeln!(writer, "type: {typing}");
            }
            OpenApiTypeIdentifier::Formatted(typing, formatting) => {
                write_indentation(writer, indentation)?;
                let _ = writeln!(writer, "type: {typing}");
                write_indentation(writer, indentation)?;
                let _ = writeln!(writer, "format: {formatting}");
            }
            OpenApiTypeIdentifier::ObjectReference(reference) => {
                write_indentation(writer, indentation)?;
                let _ = writeln!(writer, "$ref: '#/components/schemas/{reference}'");
            }
            OpenApiTypeIdentifier::Array(open_api_type_identifier) => {
                write_indentation(writer, indentation)?;
                let _ = writeln!(writer, "type: array");
                write_indentation(writer, indentation)?;
                let _ = writeln!(writer, "items:");
                open_api_type_identifier.write_yaml(writer, indentation + YAML_INDENTATION_STEP)?;
            }
            OpenApiTypeIdentifier::Map(type_identifiers) => {
                write_indentation(writer, indentation)?;
                let _ = writeln!(writer, "type: object");

                write_indentation(writer, indentation)?;
                let _ = writeln!(writer, "properties:");

                // Keys list.
                write_indentation(writer, indentation + YAML_INDENTATION_STEP)?;
                let _ = writeln!(writer, "keys:");
                write_indentation(writer, indentation + (YAML_INDENTATION_STEP * 2))?;
                let _ = writeln!(writer, "type: array");
                write_indentation(writer, indentation + (YAML_INDENTATION_STEP * 2))?;
                let _ = writeln!(writer, "items:");
                type_identifiers
                    .0
                    .write_yaml(writer, indentation + (YAML_INDENTATION_STEP * 3))?;

                // Values list.
                write_indentation(writer, indentation + YAML_INDENTATION_STEP)?;
                let _ = writeln!(writer, "values:");
                write_indentation(writer, indentation + (YAML_INDENTATION_STEP * 2))?;
                let _ = writeln!(writer, "type: array");
                write_indentation(writer, indentation + (YAML_INDENTATION_STEP * 2))?;
                let _ = writeln!(writer, "items:");
                type_identifiers
                    .1
                    .write_yaml(writer, indentation + (YAML_INDENTATION_STEP * 3))?;
            }
        };

        Ok(())
    }
}

/// Writes a number of spaces to `writer` equal to `indentation`.
fn write_indentation<W: Writes>(
    writer: &mut FmtWriter<'_, W>,
    indentation: usize,
) -> Result<(), StreamError> {
    for _ in 0..indentation {
        let _ = write!(writer, " ");
    }

    Ok(())
}
