//! ## Unstable
//!
//! SQLite schema generators for codas, with a focus on
//! supporting [Cloudflare D1](https://developers.cloudflare.com/d1/)
//! and other SQLite-compatible databases.
//!
//! ## What's Here
//!
//! For a given coda, a `CREATE TABLE IF NOT EXISTS ... STRICT`
//! statement will be generated for each data type, mapping:
//!
//! - Integer and boolean fields to `INTEGER` columns.
//! - Text fields to `TEXT` columns.
//! - Floating-point fields to `REAL` columns.
//! - Fixed-size byte arrays (`array of N u8`) to `BLOB` columns
//!   with an exact-length `CHECK` constraint.
//! - Other fixed-size arrays to `BLOB` columns containing the
//!   array's raw little-endian bytes, with an exact-length `CHECK`.
//! - Byte lists (`list of u8`) to `BLOB` columns containing the raw bytes.
//! - All other lists, maps, nested data, and unspecified fields
//!   to `BLOB` columns containing the value's Coda-encoded bytes.
//!
//! Optional fields become nullable columns. All other columns
//! are `NOT NULL`. Column comments preserve the original coda
//! type wherever the SQLite type is lossy.
//!
//! SQLite has no unsigned 64-bit integer, so `u64` fields are
//! stored as (signed) `INTEGER` columns, and values above
//! `i64::MAX` are unrepresentable.
//!
//! Primary keys and indexes are application decisions that
//! codas don't (currently?) describe. Every generated table
//! has SQLite's implicit `rowid` as its default key. To manually
//! key a table, declare a `CREATE UNIQUE INDEX` in a follow-up migration.
use core::fmt::Write;

use alloc::{format, string::String, vec::Vec};

use crate::{
    stream::{FmtWriter, StreamError, Writes},
    types::{Coda, Text, Type, Unspecified},
};

/// Generates the SQLite tables for `coda`, writing them to `stream`.
pub fn generate_types(coda: &Coda, stream: &mut impl Writes) -> Result<(), StreamError> {
    let mut writer = FmtWriter::from(stream);

    for (ordinal, typing) in [Unspecified::DATA_TYPE]
        .iter()
        .chain(coda.iter())
        .enumerate()
    {
        let data_type_name = typing.name.trim();

        // Build each column's definition and comment.
        let mut columns: Vec<(String, Option<Text>)> = Vec::new();
        for field in typing.iter() {
            let field_name = &field.name;

            let mut column = format!("  \"{field_name}\" {}", sqlite_type(&field.typing));

            if !field.optional {
                column.push_str(" NOT NULL");
            }

            if let Some(check) = sqlite_check(field_name, &field.typing) {
                column.push_str(&format!(" CHECK ({check})"));
            }

            columns.push((column, sqlite_comment(&field.typing)));
        }

        let _ = writeln!(writer);
        let _ = writeln!(writer, "-- Data {ordinal}.");

        if columns.is_empty() {
            let _ = writeln!(
                writer,
                "-- Data with no specified fields has no table; store it as Coda-encoded BLOBs where needed."
            );
            continue;
        }

        let _ = writeln!(writer, "CREATE TABLE IF NOT EXISTS \"{data_type_name}\" (");

        let last = columns.len() - 1;
        for (i, (column, comment)) in columns.iter().enumerate() {
            let separator = if i == last { "" } else { "," };
            match comment {
                Some(comment) => {
                    let _ = writeln!(writer, "{column}{separator} -- {comment}");
                }
                None => {
                    let _ = writeln!(writer, "{column}{separator}");
                }
            }
        }

        let _ = writeln!(writer, ") STRICT;");
    }

    Ok(())
}

/// Returns the SQLite storage class for a field of `typing`.
fn sqlite_type(typing: &Type) -> Text {
    match typing {
        Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::Bool => Text::Static("INTEGER"),

        Type::F32 | Type::F64 => Text::Static("REAL"),

        Type::Text => Text::Static("TEXT"),

        // Everything else is stored as binary data: raw bytes
        // for byte sequences and fixed-size arrays, Coda-encoded
        // bytes for structured values.
        Type::Unspecified | Type::Array(..) | Type::Data(_) | Type::List(_) | Type::Map(_) => {
            Text::Static("BLOB")
        }
    }
}

/// Returns a `CHECK` expression enforcing `typing`'s
/// invariants on the column `name`, if it has any.
///
/// `CHECK` constraints pass automatically for `NULL`
/// values, so they compose with optional (nullable) columns.
fn sqlite_check(name: &Text, typing: &Type) -> Option<Text> {
    match typing {
        // Fixed-size arrays store an exact number of bytes.
        Type::Array(..) => {
            let bytes = match typing.format() {
                crate::codec::Format::Blob(size) => size,
                _ => unreachable!("arrays always have blob formats"),
            };
            Some(format!("length(\"{name}\") = {bytes}").into())
        }

        Type::Bool => Some(format!("\"{name}\" IN (0, 1)").into()),

        _ => None,
    }
}

/// Returns a comment describing `typing` for columns whose
/// SQLite storage class is lossy, or `None` when the column
/// type says everything (`TEXT`).
fn sqlite_comment(typing: &Type) -> Option<Text> {
    match typing {
        Type::Text => None,

        // Raw-byte storage: the comment is just the coda type.
        Type::Array(_, elem) if **elem == Type::U8 => Some(coda_type_name(typing)),
        Type::Array(..) => {
            Some(format!("{} (raw little-endian bytes)", coda_type_name(typing)).into())
        }
        Type::List(elem) if **elem == Type::U8 => Some(coda_type_name(typing)),

        // Coda-encoded storage.
        Type::Unspecified | Type::Data(_) | Type::List(_) | Type::Map(_) => {
            Some(format!("Coda-encoded {}", coda_type_name(typing)).into())
        }

        // Numeric columns note their exact width and sign.
        _ => Some(coda_type_name(typing)),
    }
}

/// Returns the coda-syntax name of `typing`.
fn coda_type_name(typing: &Type) -> Text {
    match typing {
        Type::Unspecified => Text::Static("unspecified"),
        Type::U8 => Text::Static("u8"),
        Type::U16 => Text::Static("u16"),
        Type::U32 => Text::Static("u32"),
        Type::U64 => Text::Static("u64"),
        Type::I8 => Text::Static("i8"),
        Type::I16 => Text::Static("i16"),
        Type::I32 => Text::Static("i32"),
        Type::I64 => Text::Static("i64"),
        Type::F32 => Text::Static("f32"),
        Type::F64 => Text::Static("f64"),
        Type::Bool => Text::Static("bool"),
        Type::Text => Text::Static("text"),
        Type::Array(count, elem) => format!("array of {count} {}", coda_type_name(elem)).into(),
        Type::Data(typing) => typing.name.clone(),
        Type::List(elem) => format!("list of {}", coda_type_name(elem)).into(),
        Type::Map(typing) => format!(
            "map of {} to {}",
            coda_type_name(&typing.0),
            coda_type_name(&typing.1)
        )
        .into(),
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
-- Data with no specified fields has no table; store it as Coda-encoded BLOBs where needed.

-- Data 1.
CREATE TABLE IF NOT EXISTS "MyNestedDataType" (
  "floaty_field" REAL NOT NULL, -- f32
  "listy_field" BLOB NOT NULL -- Coda-encoded list of text
) STRICT;

-- Data 2.
CREATE TABLE IF NOT EXISTS "MyDataType" (
  "integral_field" INTEGER NOT NULL, -- i32
  "array_field" BLOB NOT NULL CHECK (length("array_field") = 16), -- array of 16 u8
  "matrix_field" BLOB NOT NULL CHECK (length("matrix_field") = 36), -- array of 3 array of 3 f32 (raw little-endian bytes)
  "textual_field" TEXT NOT NULL,
  "nested_field" BLOB NOT NULL, -- Coda-encoded MyNestedDataType
  "optional_field" INTEGER, -- u64
  "3d_field" BLOB NOT NULL, -- Coda-encoded list of list of list of i32
  "map_field" BLOB NOT NULL, -- Coda-encoded map of text to i32
  "unspecified_field" BLOB NOT NULL, -- Coda-encoded unspecified
  "frames_field" BLOB NOT NULL -- Coda-encoded list of array of 3 f32
) STRICT;"#
                .trim(),
            sql.trim()
        );
    }

    #[test]
    fn optional_arrays_are_nullable_with_checks() {
        let coda = parse(
            "# `Test` Coda\n\n## `Data` Data\n\n+ `sig` optional array of 64 u8\n\n+ `flag` bool\n\n+ `payload` list of u8\n",
        )
        .unwrap();
        let mut sql = Vec::new();
        generate_types(&coda, &mut sql).unwrap();
        let sql = String::from_utf8_lossy(&sql);

        // Optional arrays: nullable, but still length-checked
        // (CHECK passes automatically for NULL).
        assert!(
            sql.contains("\"sig\" BLOB CHECK (length(\"sig\") = 64), -- array of 64 u8"),
            "unexpected sql: {sql}"
        );

        // Bools are range-checked integers.
        assert!(
            sql.contains("\"flag\" INTEGER NOT NULL CHECK (\"flag\" IN (0, 1)), -- bool"),
            "unexpected sql: {sql}"
        );

        // Byte lists are raw, unchecked BLOBs.
        assert!(
            sql.contains("\"payload\" BLOB NOT NULL -- list of u8"),
            "unexpected sql: {sql}"
        );
    }
}
