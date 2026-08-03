//! Python code generators.
//!
//! ## What's Here
//!
//! For a given coda, the following code will be
//! generated:
//!
//! - A base `class` for the coda, which all of
//!   the coda's types extend from.
//!
//! - A `class` for each data type.
//!
//! **Codecs are not generated.** They will be
//! generated once there is a native Python library
//! for encoding and decoding coda-encoded data.
use core::fmt::Write;

use alloc::format;

use indoc::writedoc;

use crate::{
    stream::{FmtWriter, StreamError, Writes},
    types::{Coda, Text, Type, Unspecified},
};

/// Generates the Python types for `coda`,
/// writing them to `stream`.
pub fn generate_types(coda: &Coda, stream: &mut impl Writes) -> Result<(), StreamError> {
    // Extract coda metadata.
    let coda_type_name = format!("{}Data", coda.local_name.trim());
    let coda_type_docs = match &coda.docs {
        Some(docs) => docs.trim(),
        None => "Undocumented Coda. How could you? ;~;",
    };

    let mut writer = FmtWriter::from(stream);

    // Generate coda base class.
    let _ = writedoc!(
        writer,
        r#"
    class {coda_type_name}:
        """
        {coda_type_docs}
        """
        pass

    "#
    );

    // Generate coda data type classes.
    for (ordinal, typing) in [Unspecified::DATA_TYPE]
        .iter()
        .chain(coda.iter())
        .enumerate()
    {
        // Extract type metadata.
        let data_type_name = typing.name.trim();
        let data_type_docs = match &typing.docs {
            Some(docs) => docs.trim(),
            None => "Undocumented Data. How could you? ;~;",
        };

        // Generate class header.
        let _ = writedoc!(
            writer,
            r#"
        # Data {ordinal}
        class {data_type_name}({coda_type_name}):
            """
            {data_type_docs}
            """

            def __init__(self):
        "#
        );

        // Insert pass statement if the data
        // type has _no_ fields.
        if typing.iter().count() == 0 {
            let _ = writeln!(writer, "        pass");
        }

        // Generate field default value assignments.
        for field in typing.iter() {
            let default_value = match field.optional {
                true => Text::Static("None"),
                false => python_default_val(&field.typing),
            };

            let _ = writeln!(
                writer,
                "        self._{} = {}",
                field.name.trim(),
                default_value,
            );
        }

        let _ = writeln!(writer);

        // Generate field property functions.
        for (ordinal, field) in typing.iter().enumerate() {
            // Field ordinals are 1-indexed.
            let ordinal = ordinal + 1;
            let field_name = field.name.trim();
            let field_type = python_type(&field.typing);
            let field_docs = match &field.docs {
                Some(docs) => docs.trim(),
                None => "Undocumented Field. How could you? ;~;",
            };

            // Generate type checks.
            let type_check = match python_type_check(&field.typing) {
                Some(type_check) => type_check,
                None => Text::from(""),
            };

            // Generate getter and setter.
            if field.optional {
                let _ = writedoc!(
                    writer,
                    r#"
                # Field {ordinal}
                    @property
                    def {field_name}(self) -> {field_type} | None:
                        """
                        {field_docs}
                        """
                        return self._{field_name}

                    @{field_name}.setter
                    def {field_name}(self, value: {field_type} | None):
                        if value is None:
                            self._{field_name} = None
                        else:
                            {type_check}
                            self._{field_name} = value
    
                "#
                );
            } else {
                let _ = writedoc!(
                    writer,
                    r#"
                # Field {ordinal}
                    @property
                    def {field_name}(self) -> {field_type}:
                        """
                        {field_docs}
                        """
                        return self._{field_name}
    
                    @{field_name}.setter
                    def {field_name}(self, value: {field_type}):
                        {type_check}
                        self._{field_name} = value
    
                "#
                );
            }
        }
    }

    Ok(())
}

/// Returns the Python literal of `type`'s default value.
fn python_default_val(typing: &Type) -> Text {
    match typing {
        Type::Unspecified => Text::Static("None"),
        Type::U8 => Text::Static("0"),
        Type::U16 => Text::Static("0"),
        Type::U32 => Text::Static("0"),
        Type::U64 => Text::Static("0"),
        Type::I8 => Text::Static("0"),
        Type::I16 => Text::Static("0"),
        Type::I32 => Text::Static("0"),
        Type::I64 => Text::Static("0"),
        Type::F32 => Text::Static("0.0"),
        Type::F64 => Text::Static("0.0"),
        Type::Bool => Text::Static("False"),
        Type::Text => Text::Static("\"\""),
        Type::Array(count, elem) => {
            let elem_default = python_default_val(elem);
            format!("[{elem_default}] * {count}").into()
        }
        Type::Data(typing) => format!("{}()", typing.name.trim()).into(),
        Type::List(_) => Text::Static("[]"),
        Type::Map(_) => Text::Static("{}"),
    }
}

/// Returns a Python fragment enforcing the
/// expected invariatns for a type.
///
/// Not all types have checks; these checks
/// primarily exist for numeric types, since
/// Python only has one type of integer (`int`)
/// but codas have many.
fn python_type_check(typing: &Type) -> Option<Text> {
    match typing {
        Type::Unspecified => None,
        Type::U8 => Some(Text::Static(
            "if not 0 <= value <= 255: raise ValueError(\"u8 must be >= 0 and <= 255\")",
        )),
        Type::U16 => Some(Text::Static(
            "if not 0 <= value <= 65535: raise ValueError(\"u16 must be >= 0 and <= 65535\")",
        )),
        Type::U32 => Some(Text::Static(
            "if not 0 <= value <= 4294967295: raise ValueError(\"u32 must be >= 0 and <= 4294967295\")",
        )),
        Type::U64 => Some(Text::Static(
            "if not 0 <= value <= 18446744073709551615: raise ValueError(\"u64 must be >= 0 and <= 18446744073709551615\")",
        )),
        Type::I8 => Some(Text::Static(
            "if not -128 <= value <= 127: raise ValueError(\"i8 must be >= -128 and <= 127\")",
        )),
        Type::I16 => Some(Text::Static(
            "if not -32768 <= value <= 32767: raise ValueError(\"i16 must be >= -32768 and <= 32767\")",
        )),
        Type::I32 => Some(Text::Static(
            "if not -2147483648 <= value <= 2147483647: raise ValueError(\"i32 must be >= -2147483648 and <= 2147483647\")",
        )),
        Type::I64 => Some(Text::Static(
            "if not -9223372036854775808 <= value <= 9223372036854775807: raise ValueError(\"i64 must be >= -9223372036854775808 and <= 9223372036854775807\")",
        )),
        Type::F32 => Some(Text::Static(
            "if not -3.4028235e38 <= value <= 3.4028235e38: raise ValueError(\"f32 must be >= -3.4028235e38 and <= 3.4028235e38\")",
        )),
        Type::F64 => Some(Text::Static(
            "if not -1.7976931348623157e308 <= value <= 1.7976931348623157e308: raise ValueError(\"f64 must be >= -1.7976931348623157e308 and <= 1.7976931348623157e308\")",
        )),
        Type::Bool => None,
        Type::Text => None,
        Type::Array(count, _) => Some(
            format!(
                "if len(value) != {count}: raise ValueError(\"array must have exactly {count} elements\")"
            )
            .into(),
        ),
        Type::Data(_) => None,
        Type::List(_) => None,
        Type::Map(_) => None,
    }
}

/// Returns the native Python identifier of `type`.
///
/// If `type` is a [`codas::spec::Type::Data`], the
/// data's name will be interpereted as a
/// native Python identifier.
fn python_type(typing: &Type) -> Text {
    match typing {
        Type::Unspecified => Text::Static("object"),
        Type::U8 => Text::Static("int"),
        Type::U16 => Text::Static("int"),
        Type::U32 => Text::Static("int"),
        Type::U64 => Text::Static("int"),
        Type::I8 => Text::Static("int"),
        Type::I16 => Text::Static("int"),
        Type::I32 => Text::Static("int"),
        Type::I64 => Text::Static("int"),
        Type::F32 => Text::Static("float"),
        Type::F64 => Text::Static("float"),
        Type::Bool => Text::Static("bool"),
        Type::Text => Text::Static("str"),
        Type::Array(_, elem) => {
            let elem = python_type(elem.as_ref());
            format!("list[{elem}]").into()
        }
        Type::Data(typing) => typing.name.clone(),
        Type::List(typing) => {
            let typing = python_type(typing.as_ref());
            format!("list[{typing}]").into()
        }
        Type::Map(typing) => {
            let key_typing = python_type(&typing.as_ref().0);
            let value_typing = python_type(&typing.as_ref().1);
            format!("dict[{key_typing}, {value_typing}]").into()
        }
    }
}
