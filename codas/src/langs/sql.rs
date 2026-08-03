//! ## Unstable
//!
//! SQL code generators for codas, with a focus
//! on supporting the DuckDB SQL dialect.

use core::fmt::Write;

use alloc::{format, string::String};

use indoc::writedoc;

use crate::{
    stream::{FmtWriter, StreamError, Writes},
    types::{Coda, Text, Type, Unspecified},
};

/// Generates the SQL types for `coda`, writing them to `stream`.
pub fn generate_types(coda: &Coda, stream: &mut impl Writes) -> Result<(), StreamError> {
    let mut writer = FmtWriter::from(stream);

    // Generate coda data type statements.
    for (ordinal, typing) in [Unspecified::DATA_TYPE]
        .iter()
        .chain(coda.iter())
        .enumerate()
    {
        let data_type_name = typing.name.trim();

        // Build a string containing all fields.
        let mut field_string = String::new();
        for field in typing.iter() {
            let field_name = &field.name;
            let duckdb_type = duckdb_type(&field.typing);

            field_string.push_str(&format!("  \"{field_name}\" {duckdb_type},\n"));
        }

        if field_string.is_empty() {
            let _ = writedoc!(
                writer,
                r#"
    
            -- Data {ordinal}.
            -- Data with no specified fields is stored as a blob of Coda-encoded data.
            CREATE TYPE {data_type_name} AS BLOB;"#
            );
            let _ = writedoc!(writer, "\n");
        } else {
            field_string.pop(); // pop trailing newline
            field_string.pop(); // pop trailing comma

            let _ = writedoc!(
                writer,
                r#"
    
            -- Data {ordinal}.
            CREATE TYPE {data_type_name} AS STRUCT (
            "#
            );

            let _ = writedoc!(writer, "{field_string}\n);\n");
        }
    }

    Ok(())
}

/// Returns the native SQL identifier of a type.
fn duckdb_type(typing: &Type) -> Text {
    match typing {
        Type::Unspecified => Text::Static("BLOB"),
        Type::U8 => Text::Static("UTINYINT"),
        Type::U16 => Text::Static("USMALLINT"),
        Type::U32 => Text::Static("UINTEGER"),
        Type::U64 => Text::Static("UBIGINT"),
        Type::I8 => Text::Static("TINYINT"),
        Type::I16 => Text::Static("SMALLINT"),
        Type::I32 => Text::Static("INTEGER"),
        Type::I64 => Text::Static("BIGINT"),
        Type::F32 => Text::Static("FLOAT"),
        Type::F64 => Text::Static("DOUBLE"),
        Type::Bool => Text::Static("BOOLEAN"),
        Type::Text => Text::Static("VARCHAR"),
        // Byte arrays are semantically opaque tokens (hashes, keys, signatures),
        // and BLOB gives them scalar comparison and grouping semantics, which nested arrays lack.
        Type::Array(_, elem) if **elem == Type::U8 => Text::Static("BLOB"),
        Type::Array(count, elem) => {
            let elem = duckdb_type(elem);
            format!("{elem}[{count}]").into()
        }
        Type::Data(typing) => typing.name.clone(),
        Type::List(typing) => {
            let inner = duckdb_type(typing);
            format!("{}[]", inner).into()
        }
        Type::Map(typing) => {
            let key_type = duckdb_type(&typing.0);
            let value_type = duckdb_type(&typing.1);
            format!("MAP({}, {})", key_type, value_type).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::parse::{parse, tests::TEST_CODA_MARKDOWN};

    #[test]
    fn smoke() {
        let coda = parse(TEST_CODA_MARKDOWN).unwrap();
        let mut sql = Vec::new();
        generate_types(&coda, &mut sql).unwrap();
        let sql = String::from_utf8_lossy(&sql);

        assert_eq!(
            r#"
-- Data 0.
-- Data with no specified fields is stored as a blob of Coda-encoded data.
CREATE TYPE Unspecified AS BLOB;

-- Data 1.
CREATE TYPE MyNestedDataType AS STRUCT (
  "floaty_field" FLOAT,
  "listy_field" VARCHAR[]
);

-- Data 2.
CREATE TYPE MyDataType AS STRUCT (
  "integral_field" INTEGER,
  "array_field" BLOB,
  "matrix_field" FLOAT[3][3],
  "textual_field" VARCHAR,
  "nested_field" MyNestedDataType,
  "optional_field" UBIGINT,
  "3d_field" INTEGER[][][],
  "map_field" MAP(VARCHAR, INTEGER),
  "unspecified_field" BLOB,
  "frames_field" FLOAT[3][]
);"#
            .trim(),
            sql.trim()
        );
    }
}
