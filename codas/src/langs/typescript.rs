//! Typescript code generators.
//!
//! ## What's Here
//!
//! For a given coda, the following code will be
//! generated:
//!
//! - A `class` for each data type.
//!
//! - A `type` for the coda, which is a union of
//!   the classes generated for each data type.
//!
//! - A `const object` for the coda, which exposes
//!   functions for creating instances of each data
//!   type, and a utility for matching on each data type.
//!
//! - An `interface` for the parameter to the matching
//!   utility function.
//!
//! **Codecs are not generated.** They will be
//! generated once there is a native Typescript library
//! for encoding and decoding coda-encoded data.
use core::fmt::Write;

use alloc::format;

use indoc::writedoc;

use crate::{
    stream::{FmtWriter, StreamError, Writes},
    types::{Coda, Text, Type, Unspecified},
};

/// Generates the Typescript types for `coda`.
pub fn generate_types(coda: &Coda, stream: &mut impl Writes) -> Result<(), StreamError> {
    // Extract coda metadata.
    let coda_type_name = format!("{}Data", coda.local_name.trim());
    let coda_type_docs = match &coda.docs {
        Some(docs) => docs.trim(),
        None => "Undocumented Coda. How could you? ;~;",
    };

    let mut writer = FmtWriter::from(stream);

    // Generate coda const.
    write_typescript_doc(&mut writer, 0, coda_type_docs)?;
    let _ = writedoc!(
        writer,
        r#"
        const {coda_type_name} = {{
        "#
    );
    for data_type in [Unspecified::DATA_TYPE].iter().chain(coda.iter()) {
        let data_type_name = &data_type.name;
        let data_type_docs = match &data_type.docs {
            Some(docs) => docs.trim(),
            None => "Undocumented Data. How could you? ;~;",
        };
        write_typescript_doc(&mut writer, 4, data_type_docs)?;

        let _ = writeln!(writer, "    {data_type_name}(): {data_type_name} {{");
        let _ = writeln!(writer, "        return new {data_type_name}();");
        let _ = write!(writer, "    }},\n\n");
    }

    // Generate matcher utility.
    let _ = writeln!(writer, "    /**");
    let _ = writeln!(
        writer,
        "     * Executes the matching handler in `matcher` for `data`."
    );
    let _ = writeln!(writer, "     *");
    let _ = writeln!(writer, "     * @remarks");
    let _ = writeln!(writer, "     *");
    let _ = writeln!(
        writer,
        "     * If `data` does not match any of the handlers in `matcher`,"
    );
    let _ = writeln!(
        writer,
        "     * the {{@link {coda_type_name}Matcher.Unspecified}} handler will be called."
    );
    let _ = writeln!(writer, "     */");
    let _ = writeln!(
        writer,
        "    match<R>(data: {coda_type_name}, matcher: {coda_type_name}Matcher<R>): R {{"
    );
    let _ = writeln!(writer, "        switch (data.__ordinal) {{");
    for (ordinal, data_type) in coda.iter().enumerate() {
        let ordinal = ordinal + 1;
        let data_type_name = &data_type.name;
        let _ = writeln!(writer, "            case {ordinal}: return matcher.{data_type_name} ? matcher.{data_type_name}(data as {data_type_name}) : matcher.Unspecified();");
    }
    let _ = writeln!(writer, "            default: return matcher.Unspecified();");
    let _ = writeln!(writer, "        }}");
    let _ = writeln!(writer, "    }},");

    let _ = write!(writer, "}};\n\n");

    // Generate coda type.
    let _ = write!(writer, "type {coda_type_name} = Unspecified");
    for data_type in coda.iter() {
        let _ = write!(writer, " | {}", data_type.name);
    }
    let _ = write!(writer, ";\n\n");

    // Generate matcher interface.
    let _ = writedoc!(
        writer,
        r#"
        /**
         * Parameter for {{@link {coda_type_name}.match}}.
         * 
         * @remarks
         * 
         * Each function on this interface corresponds to one
         * of the data types within a {{@link {coda_type_name}}}. When a
         * given data type is matched, its corresponding function
         * is called.
         * 
         * Every function is optional _except_ `Unspecified`, which
         * is called by default when no other function matches a
         * given {{@link {coda_type_name}}}.
         * 
         * The functions may return any kind of data, but all
         * functions must return the _same_ type of data.
         */
        interface {coda_type_name}Matcher<R> {{

            /**
             * Handler for {{@link Unspecified}} or otherwise unknown data.
             */
            Unspecified: () => R;
        "#
    );
    for data_type in coda.iter() {
        let data_type_name = &data_type.name;
        let _ = writeln!(writer);
        write_typescript_doc(
            &mut writer,
            4,
            &format!("Handler for {{@link {data_type_name}}} data."),
        )?;
        let _ = writeln!(
            writer,
            "    {data_type_name}?: (data: {data_type_name}) => R;"
        );
    }
    let _ = write!(writer, "}}\n\n");

    // Generate data interfaces.
    for data_type in [Unspecified::DATA_TYPE].iter().chain(coda.iter()) {
        let ordinal = data_type.format().as_data_format().ordinal;

        // Declaration and ordinal.
        let data_type_name = &data_type.name;
        let _ = writedoc!(
            writer,
            r#"
            /**
             * Class corresponding to {{@link {coda_type_name}.{data_type_name}}}.
             */
            class {data_type_name} {{
                readonly __ordinal: {ordinal} = {ordinal};
        "#
        );

        // Fields.
        for field in data_type.iter() {
            let field_name = &field.name;
            let field_docs = match &field.docs {
                Some(docs) => docs.trim(),
                None => "Undocumented Field. How could you? ;~;",
            };
            let field_type = typescript_type(&field.typing);
            let _ = writeln!(writer);
            write_typescript_doc(&mut writer, 4, field_docs)?;

            if field.optional {
                let _ = writeln!(writer, "    {field_name}?: {field_type} = undefined;");
            } else {
                let field_default = typescript_default_val(&field.typing);
                let _ = writeln!(writer, "    {field_name}: {field_type} = {field_default};");
            }
        }
        let _ = write!(writer, "}}\n\n");
    }

    Ok(())
}

/// Writes a Typescript multi-line doc with leading
/// `indentation` containing `docs` to `writer`.
fn write_typescript_doc<W: Writes>(
    writer: &mut FmtWriter<'_, W>,
    indentation: usize,
    docs: &str,
) -> Result<(), StreamError> {
    // Docstring header.
    write_indentation(writer, indentation)?;
    let _ = writeln!(writer, "/**");

    // Docstring body.
    for line in docs.lines() {
        write_indentation(writer, indentation)?;
        let _ = writeln!(writer, " * {line}");
    }

    // Docstring footer.
    write_indentation(writer, indentation)?;
    let _ = writeln!(writer, " */");

    Ok(())
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

/// Returns the Typescript literal of `type`'s default value.
fn typescript_default_val(typing: &Type) -> Text {
    match typing {
        Type::Unspecified => Text::Static("undefined"),
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
        Type::Bool => Text::Static("false"),
        Type::Text => Text::Static("\"\""),
        Type::Array(count, elem) => match typescript_typed_array(elem) {
            Some(typed_array) => format!("new {typed_array}({count})").into(),
            None => {
                let elem_default = typescript_default_val(elem);
                format!("Array.from({{ length: {count} }}, () => {elem_default})").into()
            }
        },
        Type::Data(typing) => {
            let name = &typing.name;
            format!("new {name}()").into()
        }
        Type::List(_) => Text::Static("[]"),
        Type::Map(_) => Text::Static("new Map()"),
    }
}

/// Returns the native Typescript typed array
/// corresponding to arrays of `elem`, if one exists.
fn typescript_typed_array(elem: &Type) -> Option<&'static str> {
    match elem {
        Type::U8 => Some("Uint8Array"),
        Type::I8 => Some("Int8Array"),
        Type::U16 => Some("Uint16Array"),
        Type::I16 => Some("Int16Array"),
        Type::U32 => Some("Uint32Array"),
        Type::I32 => Some("Int32Array"),
        Type::U64 => Some("BigUint64Array"),
        Type::I64 => Some("BigInt64Array"),
        Type::F32 => Some("Float32Array"),
        Type::F64 => Some("Float64Array"),
        _ => None,
    }
}

/// Returns the native Typescript identifier of `type`.
///
/// If `type` is a [`codas::spec::Type::Data`], the
/// data's name will be interpereted as a
/// native Typescript identifier.
fn typescript_type(typing: &Type) -> Text {
    match typing {
        Type::Unspecified => Text::Static("unknown"),
        Type::U8 => Text::Static("number"),
        Type::U16 => Text::Static("number"),
        Type::U32 => Text::Static("number"),
        Type::U64 => Text::Static("number"),
        Type::I8 => Text::Static("number"),
        Type::I16 => Text::Static("number"),
        Type::I32 => Text::Static("number"),
        Type::I64 => Text::Static("number"),
        Type::F32 => Text::Static("number"),
        Type::F64 => Text::Static("number"),
        Type::Bool => Text::Static("boolean"),
        Type::Text => Text::Static("string"),
        Type::Array(_, elem) => match typescript_typed_array(elem) {
            Some(typed_array) => Text::Static(typed_array),
            None => {
                let elem = typescript_type(elem.as_ref());
                format!("Array<{elem}>").into()
            }
        },
        Type::Data(typing) => typing.name.clone(),
        Type::List(typing) => {
            let typing = typescript_type(typing.as_ref());
            format!("Array<{typing}>").into()
        }
        Type::Map(typing) => {
            let key_typing = typescript_type(&typing.as_ref().0);
            let value_typing = typescript_type(&typing.as_ref().1);
            format!("Map<{key_typing}, {value_typing}>").into()
        }
    }
}
