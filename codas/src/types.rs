//! Built-in data types and their in-memory
//! representations.
//!
//! # Unstable
//!
//! The APIs exposed by this module are _primarily_
//! used for code generation and dynamic data manipulation;
//! the exact APIs are subject to change, and may
//! not be well-optimized.
use core::convert::Infallible;

use alloc::{boxed::Box, vec, vec::Vec};

use crate::codec::{
    CodecError, DataFormat, DataHeader, Decodable, Encodable, Format, ReadsDecodable,
    UnexpectedDataFormatSnafu, WritesEncodable,
};

pub mod binary;
pub mod cryptography;
pub mod dynamic;
pub mod list;
pub mod map;
pub mod number;
mod text;
pub use dynamic::Unspecified;
pub use text::*;

/// Enumeration of available built in types.
#[non_exhaustive]
#[derive(Default, Debug, Clone, PartialEq)]
pub enum Type {
    /// Unspecified data.
    #[default]
    Unspecified,

    /// Unsigned (positive) 8-bit number.
    U8,
    /// Unsigned (positive) 16-bit number.
    U16,
    /// Unsigned (positive) 32-bit number.
    U32,
    /// Unsigned (positive) 64-bit number.
    U64,

    /// Signed (positive or negative) 8-bit number.
    I8,
    /// Signed (positive or negative) 16-bit number.
    I16,
    /// Signed (positive or negative) 32-bit number.
    I32,
    /// Signed (positive or negative) 64-bit number.
    I64,

    /// 32-bit floating point (decimal) number.
    F32,
    /// 64-bit floating point (decimal) number.
    F64,

    /// Boolean (true or false).
    Bool,

    /// UTF-8 encoded text.
    Text,

    /// Data with [`DataType`].
    Data(DataType),

    /// Data with [`Type`] that's _semantically_ a list.
    List(Box<Type>),

    /// A mapping between data of two types.
    Map(Box<(Type, Type)>),
}

impl Type {
    // Wire ordinals for built-in types.
    // Built-in ordinals count down from 255; user-defined count up from 1.
    pub(crate) const UNSPECIFIED_ORDINAL: u8 = 0;
    pub(crate) const U8_ORDINAL: u8 = 255;
    pub(crate) const U16_ORDINAL: u8 = 254;
    pub(crate) const U32_ORDINAL: u8 = 253;
    pub(crate) const U64_ORDINAL: u8 = 252;
    pub(crate) const I8_ORDINAL: u8 = 251;
    pub(crate) const I16_ORDINAL: u8 = 250;
    pub(crate) const I32_ORDINAL: u8 = 249;
    pub(crate) const I64_ORDINAL: u8 = 248;
    pub(crate) const F32_ORDINAL: u8 = 247;
    pub(crate) const F64_ORDINAL: u8 = 246;
    pub(crate) const BOOL_ORDINAL: u8 = 245;
    pub(crate) const TEXT_ORDINAL: u8 = 244;
    pub(crate) const LIST_ORDINAL: u8 = 243;
    pub(crate) const MAP_ORDINAL: u8 = 242;

    /// Returns the wire ordinal for this type.
    pub const fn ordinal(&self) -> u8 {
        match self {
            Type::Unspecified => Self::UNSPECIFIED_ORDINAL,
            Type::U8 => Self::U8_ORDINAL,
            Type::U16 => Self::U16_ORDINAL,
            Type::U32 => Self::U32_ORDINAL,
            Type::U64 => Self::U64_ORDINAL,
            Type::I8 => Self::I8_ORDINAL,
            Type::I16 => Self::I16_ORDINAL,
            Type::I32 => Self::I32_ORDINAL,
            Type::I64 => Self::I64_ORDINAL,
            Type::F32 => Self::F32_ORDINAL,
            Type::F64 => Self::F64_ORDINAL,
            Type::Bool => Self::BOOL_ORDINAL,
            Type::Text => Self::TEXT_ORDINAL,
            Type::Data(data) => data.format.as_data_format().ordinal,
            Type::List(_) => Self::LIST_ORDINAL,
            Type::Map(_) => Self::MAP_ORDINAL,
        }
    }

    /// Returns the type corresponding to `ordinal`.
    ///
    /// Iff ordinal does not correspond to a built-in-type,
    /// `None` is returned.
    ///
    /// List and Map return placeholder inner types
    /// ([`Type::Unspecified`]) since the ordinal alone doesn't
    /// describe the element/key/value types.
    pub fn from_ordinal(ordinal: u8) -> Option<Self> {
        match ordinal {
            Self::UNSPECIFIED_ORDINAL => Some(Type::Unspecified),
            Self::U8_ORDINAL => Some(Type::U8),
            Self::U16_ORDINAL => Some(Type::U16),
            Self::U32_ORDINAL => Some(Type::U32),
            Self::U64_ORDINAL => Some(Type::U64),
            Self::I8_ORDINAL => Some(Type::I8),
            Self::I16_ORDINAL => Some(Type::I16),
            Self::I32_ORDINAL => Some(Type::I32),
            Self::I64_ORDINAL => Some(Type::I64),
            Self::F32_ORDINAL => Some(Type::F32),
            Self::F64_ORDINAL => Some(Type::F64),
            Self::BOOL_ORDINAL => Some(Type::Bool),
            Self::TEXT_ORDINAL => Some(Type::Text),
            Self::LIST_ORDINAL => Some(Type::List(Type::Unspecified.into())),
            Self::MAP_ORDINAL => Some(Type::Map((Type::Unspecified, Type::Unspecified).into())),
            _ => None,
        }
    }

    /// The type's encoding format.
    pub const fn format(&self) -> Format {
        match self {
            Type::Unspecified => Format::Fluid,
            Type::U8 => u8::FORMAT,
            Type::U16 => u16::FORMAT,
            Type::U32 => u32::FORMAT,
            Type::U64 => u64::FORMAT,
            Type::I8 => i8::FORMAT,
            Type::I16 => i16::FORMAT,
            Type::I32 => i32::FORMAT,
            Type::I64 => i64::FORMAT,
            Type::F32 => f32::FORMAT,
            Type::F64 => f64::FORMAT,
            Type::Bool => bool::FORMAT,
            Type::Text => Text::FORMAT,
            Type::Data(data) => data.format,
            Type::List(typing) => typing.format().as_data_format().as_format(),

            // Maps are formatted as a list of keys
            // followed by a list of values.
            Type::Map(..) => DataFormat {
                blob_size: 0,
                data_fields: 2,
                ordinal: 0,
            }
            .as_format(),
        }
    }

    /// Returns the type with `name`.
    ///
    /// This function assumes `name` is in ASCII lowercase.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "unspecified" => Some(Type::Unspecified),
            "u8" => Some(Type::U8),
            "u16" => Some(Type::U16),
            "u32" => Some(Type::U32),
            "u64" => Some(Type::U64),
            "i8" => Some(Type::I8),
            "i16" => Some(Type::I16),
            "i32" => Some(Type::I32),
            "i64" => Some(Type::I64),
            "f32" => Some(Type::F32),
            "f64" => Some(Type::F64),
            "bool" => Some(Type::Bool),
            "text" => Some(Type::Text),
            _ => None,
        }
    }
}

/// In-memory representation of a coda.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Coda {
    /// The coda's full name, including any
    /// hierarchical components and separators.
    pub global_name: Text,

    /// The final component of [`Self::global_name`]
    /// that does not describe a hierarchy.
    pub local_name: Text,

    pub docs: Option<Text>,

    /// Data in ascending order by ordinal.
    pub(crate) data: Vec<DataType>,
}

impl Coda {
    /// Returns a new coda containing `data`.
    pub fn new(global_name: Text, local_name: Text, docs: Option<Text>, data: &[DataType]) -> Self {
        Self {
            global_name,
            local_name,
            docs,
            data: Vec::from(data),
        }
    }

    /// Returns an iterator over all data types in the coda.
    ///
    /// The implicit [`crate::types::Unspecified`] data type
    /// is _not_ included in the returned iterator.
    pub fn iter(&self) -> impl Iterator<Item = &DataType> {
        self.data.iter()
    }

    /// Returns the data type with `name`,
    /// if it is known by the coda.
    #[cfg(feature = "parse")]
    pub(crate) fn type_from_name(&self, name: &str) -> Option<Type> {
        for data in self.data.iter() {
            if data.name.eq_ignore_ascii_case(name) {
                return Some(Type::Data(data.clone()));
            }
        }

        Type::from_name(name)
    }
}

/// Data containing a structured set of [`DataField`]s.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct DataType {
    /// The name of the data type.
    ///
    /// TODO: We've been structuring names similar
    /// to fully-qualified Rust type names (like `my::data::TypeName`).
    /// We should standardize on a language-neutral naming
    /// convention; perhaps HTTP-style URLs (like `/my/data/TypeName`)
    /// so downstream tools have an easy way to map hierarchical
    /// names back to native type names as appropriate.
    pub name: Text,

    /// Markdown-formatted documentation of the data type.
    pub docs: Option<Text>,

    /// Ordered set of [`Format::Blob`]
    /// fields in the data type.
    blob_fields: Vec<DataField>,

    /// Ordered set of [`Format::Data`]
    /// fields in the data type.
    ///
    /// These fields are always encoded, in
    /// order, _after_ all [`Self::blob_fields`].
    data_fields: Vec<DataField>,

    /// The encoding format of data with this type.
    format: Format,
}

impl DataType {
    /// Returns a new fixed data type with
    /// `name`, `ordinal`, `blob_fields`, and `data_fields`.
    pub fn new(
        name: Text,
        docs: Option<Text>,
        ordinal: u8,
        blob_fields: &[DataField],
        data_fields: &[DataField],
    ) -> Self {
        // Build a new encoding format for the data.
        let mut format = Format::data(ordinal);

        // Add blob fields to the format.
        let mut i = 0;
        while i < blob_fields.len() {
            let field = &blob_fields[i];
            format = format.with(field.typing.format());
            i += 1;
        }

        // Add data fields to the format.
        let mut i = 0;
        while i < data_fields.len() {
            let field = &data_fields[i];
            format = format.with(field.typing.format());
            i += 1;
        }

        Self {
            name,
            docs,
            blob_fields: Vec::from(blob_fields),
            data_fields: Vec::from(data_fields),
            format,
        }
    }

    /// Returns a new data type with a fluid format.
    pub const fn new_fluid(name: Text, docs: Option<Text>) -> Self {
        Self {
            name,
            docs,
            blob_fields: vec![],
            data_fields: vec![],
            format: Format::Fluid,
        }
    }

    /// Returns an iterator over all fields within the type.
    pub fn iter(&self) -> impl Iterator<Item = &DataField> {
        self.blob_fields.iter().chain(self.data_fields.iter())
    }

    /// Adds a new `field` to the type.
    pub fn with(mut self, field: DataField) -> Self {
        if matches!(self.format, Format::Fluid) {
            todo!("it should be an error to add fields to a type defined as fluid")
        }

        let field_format = field.typing.format();

        // Optional fields are wrapped into a data field
        // so they get a header for presence signaling.
        if field.optional {
            let boxed_format = Format::data(0).with(field_format);
            self.format = self.format.with(boxed_format);
            self.data_fields.push(field);
        } else {
            self.format = self.format.with(field_format);
            match field_format {
                Format::Blob(..) => {
                    self.blob_fields.push(field);
                }
                Format::Data(..) | Format::Fluid => {
                    self.data_fields.push(field);
                }
            };
        }

        self
    }

    /// Returns the type's encoding format.
    pub const fn format(&self) -> &Format {
        &self.format
    }
}

/// A field in a [`DataType`].
#[derive(Default, Clone, Debug, PartialEq)]
pub struct DataField {
    /// Name of the field.
    pub name: Text,

    /// Markdown-formatted documentation of the field.
    pub docs: Option<Text>,

    /// Type of the field.
    pub typing: Type,

    /// True if the field is semantically optional.
    pub optional: bool,

    /// True if the field is semantically flattened.
    ///
    /// This property has _no_ effect on the encoding,
    /// decoding, or in-language representation of
    /// a field; it's an informational marker that some
    /// marshallers (like JSON) may use to enable
    /// compatibility between coda-defined data and
    /// legacy systems.
    pub flattened: bool,
}

/// A thing that _might_ contain data with a
/// specific format `D`.
///
/// This trait is mainly intended for use with the
/// enums auto-generated for [`Coda`]s
pub trait TryAsFormat<D> {
    /// Type of error returned when `self`
    /// doesn't contain data of format `D`.
    ///
    /// This error should be the ordinal
    /// identifier of the _actual_ data in `D`,
    /// or [`Infallible`].
    type Error;

    /// Returns a `D`-formatted reference to the data.
    fn try_as_format(&self) -> Result<&D, Self::Error>;
}

/// Every data format can be interpreted as itself.
impl<T> TryAsFormat<T> for T {
    type Error = Infallible;

    fn try_as_format(&self) -> Result<&T, Self::Error> {
        Ok(self)
    }
}

// Codecs /////////////////////////////////////////////////

impl Encodable for Type {
    const FORMAT: Format = Format::Fluid;

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        match self {
            Type::Data(typing) => writer.write_data(typing),
            Type::List(typing) => writer.write_data(typing.as_ref()),
            Type::Map(typing) => {
                writer.write_data(&typing.as_ref().0)?;
                writer.write_data(&typing.as_ref().1)?;
                Ok(())
            }

            // Only data types contain additional encoded info.
            _ => Ok(()),
        }
    }

    fn encode_header(
        &self,
        writer: &mut (impl WritesEncodable + ?Sized),
    ) -> Result<(), CodecError> {
        let format = match self {
            Type::Map(_) => Format::data(self.ordinal())
                .with(Type::FORMAT)
                .with(Type::FORMAT)
                .as_data_format(),
            Type::Data(..) | Type::List(_) => Format::data(self.ordinal())
                .with(Type::FORMAT)
                .as_data_format(),
            _ => Format::data(self.ordinal()).as_data_format(),
        };

        DataHeader { count: 1, format }.encode(writer)
    }
}

impl Decodable for Type {
    fn decode(
        &mut self,
        reader: &mut (impl ReadsDecodable + ?Sized),
        header: Option<DataHeader>,
    ) -> Result<(), CodecError> {
        let header = header.ok_or_else(|| {
            UnexpectedDataFormatSnafu {
                expected: Self::FORMAT,
                actual: None::<DataHeader>,
            }
            .build()
        })?;

        // Type is always encoded with count=1.
        if header.count != 1 {
            return UnexpectedDataFormatSnafu {
                expected: Self::FORMAT,
                actual: Some(header),
            }
            .fail();
        }

        match header.format.ordinal {
            Self::LIST_ORDINAL => {
                // List: blob_size=0, data_fields=1 (inner Type).
                if header.format.blob_size != 0 || header.format.data_fields != 1 {
                    return UnexpectedDataFormatSnafu {
                        expected: Self::FORMAT,
                        actual: Some(header),
                    }
                    .fail();
                }
                let mut typing = Type::default();
                reader.read_data_into(&mut typing)?;
                *self = Type::List(typing.into());
            }
            Self::MAP_ORDINAL => {
                // Map: blob_size=0, data_fields=2 (key Type + value Type).
                if header.format.blob_size != 0 || header.format.data_fields != 2 {
                    return UnexpectedDataFormatSnafu {
                        expected: Self::FORMAT,
                        actual: Some(header),
                    }
                    .fail();
                }
                let mut key_typing = Type::default();
                reader.read_data_into(&mut key_typing)?;
                let mut value_typing = Type::default();
                reader.read_data_into(&mut value_typing)?;
                *self = Type::Map((key_typing, value_typing).into());
            }
            ordinal => match Type::from_ordinal(ordinal) {
                // Scalars: blob_size=0, data_fields=0 (no payload).
                Some(simple) => {
                    if header.format.blob_size != 0 || header.format.data_fields != 0 {
                        return UnexpectedDataFormatSnafu {
                            expected: Self::FORMAT,
                            actual: Some(header),
                        }
                        .fail();
                    }
                    *self = simple;
                }
                // Any unknown ordinal is a data type descriptor.
                None => {
                    // Data: blob_size=0, data_fields=1 (inner DataType).
                    if header.format.blob_size != 0 || header.format.data_fields != 1 {
                        return UnexpectedDataFormatSnafu {
                            expected: Self::FORMAT,
                            actual: Some(header),
                        }
                        .fail();
                    }
                    let mut typing = DataType::default();
                    reader.read_data_into(&mut typing)?;
                    *self = Type::Data(typing);
                }
            },
        }

        Ok(())
    }
}

impl Encodable for Coda {
    const FORMAT: crate::codec::Format = Format::data(0)
        .with(Text::FORMAT)
        .with(Text::FORMAT)
        .with(Text::FORMAT)
        .with(Vec::<DataType>::FORMAT);

    fn encode(
        &self,
        writer: &mut (impl crate::codec::WritesEncodable + ?Sized),
    ) -> Result<(), crate::codec::CodecError> {
        writer.write_data(&self.global_name)?;
        writer.write_data(&self.local_name)?;
        writer.write_data(&self.docs)?;
        writer.write_data(&self.data)?;
        Ok(())
    }
}

impl Decodable for Coda {
    fn decode(
        &mut self,
        reader: &mut (impl crate::codec::ReadsDecodable + ?Sized),
        header: Option<crate::codec::DataHeader>,
    ) -> Result<(), crate::codec::CodecError> {
        let _ = Self::ensure_header(header, &[0])?;

        reader.read_data_into(&mut self.global_name)?;
        reader.read_data_into(&mut self.local_name)?;
        reader.read_data_into(&mut self.docs)?;
        reader.read_data_into(&mut self.data)?;

        Ok(())
    }
}

impl Encodable for DataType {
    const FORMAT: Format = Format::data(0)
        .with(Text::FORMAT)
        .with(Option::<Text>::FORMAT)
        .with(Vec::<DataField>::FORMAT)
        .with(Vec::<DataField>::FORMAT)
        .with(Format::FORMAT);

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        writer.write_data(&self.name)?;
        writer.write_data(&self.docs)?;
        writer.write_data(&self.blob_fields)?;
        writer.write_data(&self.data_fields)?;
        writer.write_data(&self.format)?;
        Ok(())
    }
}

impl Decodable for DataType {
    fn decode(
        &mut self,
        reader: &mut (impl ReadsDecodable + ?Sized),
        header: Option<DataHeader>,
    ) -> Result<(), CodecError> {
        let _ = Self::ensure_header(header, &[0])?;

        reader.read_data_into(&mut self.name)?;
        reader.read_data_into(&mut self.docs)?;
        reader.read_data_into(&mut self.blob_fields)?;
        reader.read_data_into(&mut self.data_fields)?;
        reader.read_data_into(&mut self.format)?;

        Ok(())
    }
}

impl Encodable for DataField {
    const FORMAT: Format = Format::data(0)
        .with(bool::FORMAT)
        .with(bool::FORMAT)
        .with(Text::FORMAT)
        .with(Option::<Text>::FORMAT)
        .with(Type::FORMAT);

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        writer.write_data(&self.optional)?;
        writer.write_data(&self.flattened)?;
        writer.write_data(&self.name)?;
        writer.write_data(&self.docs)?;
        writer.write_data(&self.typing)?;
        Ok(())
    }
}

impl Decodable for DataField {
    fn decode(
        &mut self,
        reader: &mut (impl ReadsDecodable + ?Sized),
        header: Option<DataHeader>,
    ) -> Result<(), CodecError> {
        let _ = Self::ensure_header(header, &[0])?;
        reader.read_data_into(&mut self.optional)?;
        reader.read_data_into(&mut self.flattened)?;
        reader.read_data_into(&mut self.name)?;
        reader.read_data_into(&mut self.docs)?;
        reader.read_data_into(&mut self.typing)?;
        Ok(())
    }
}

impl<T> Encodable for Option<T>
where
    T: Default + Encodable + 'static,
{
    const FORMAT: Format = Format::data(0).with(T::FORMAT);

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        match self {
            None => Ok(()),
            Some(value) => writer.write_data(value),
        }
    }

    fn encode_header(
        &self,
        writer: &mut (impl WritesEncodable + ?Sized),
    ) -> Result<(), CodecError> {
        DataHeader {
            count: if self.is_some() { 1 } else { 0 },
            format: if self.is_some() {
                Self::FORMAT.as_data_format()
            } else {
                DataFormat::default()
            },
        }
        .encode(writer)
    }
}

impl<T> Decodable for Option<T>
where
    T: Decodable + Default + 'static,
{
    fn decode(
        &mut self,
        reader: &mut (impl ReadsDecodable + ?Sized),
        header: Option<DataHeader>,
    ) -> Result<(), CodecError> {
        let h = header.ok_or_else(|| {
            UnexpectedDataFormatSnafu {
                expected: Self::FORMAT,
                actual: None::<DataHeader>,
            }
            .build()
        })?;

        // Option is always count=0 (None) or count=1 (Some).
        if h.count > 1 {
            return UnexpectedDataFormatSnafu {
                expected: Self::FORMAT,
                actual: Some(h),
            }
            .fail();
        }

        if h.count == 0 {
            // For None, the format must be zeroed (no payload).
            if h.format != DataFormat::default() {
                return UnexpectedDataFormatSnafu {
                    expected: Self::FORMAT,
                    actual: Some(h),
                }
                .fail();
            }
            *self = None;
        } else {
            // For Some, the format must match the Option wrapper format.
            if h.format != Self::FORMAT.as_data_format() {
                return UnexpectedDataFormatSnafu {
                    expected: Self::FORMAT,
                    actual: Some(h),
                }
                .fail();
            }
            let mut value = T::default();
            reader.read_data_into(&mut value)?;
            *self = Some(value);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::codec::{Decodable, WritesEncodable};

    use super::*;

    /// Sample data structure for testing type manipulation APIs.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct TestData {
        pub number: i32,
        pub floaty: f64,
        pub text_list: Vec<Text>,
        pub text: Text,
        pub nested: NestedTestData,
        pub two_d: Vec<Vec<Text>>,
    }

    impl TestData {
        pub fn typing() -> DataType {
            let blob_fields = vec![
                DataField {
                    name: Text::from("number"),
                    docs: None,
                    typing: Type::I32,
                    optional: false,
                    flattened: false,
                },
                DataField {
                    name: Text::from("floaty"),
                    docs: None,
                    typing: Type::F64,
                    optional: false,
                    flattened: false,
                },
            ];

            let data_fields = vec![
                DataField {
                    name: Text::from("text_list"),
                    docs: None,
                    typing: Type::List(Type::Text.into()),
                    optional: false,
                    flattened: false,
                },
                DataField {
                    name: Text::from("text"),
                    docs: None,
                    typing: Type::Text,
                    optional: false,
                    flattened: false,
                },
                DataField {
                    name: Text::from("nested"),
                    docs: None,
                    typing: Type::Data(NestedTestData::typing()),
                    optional: false,
                    flattened: false,
                },
                DataField {
                    name: Text::from("two_d"),
                    docs: None,
                    typing: Type::List(Type::List(Type::Text.into()).into()),
                    optional: false,
                    flattened: false,
                },
            ];

            let typing = DataType::new(Text::from("Testdata"), None, 1, &blob_fields, &data_fields);

            assert_eq!(Self::FORMAT, *typing.format());

            typing
        }
    }

    impl Encodable for TestData {
        const FORMAT: Format = Format::data(1)
            .with(i32::FORMAT)
            .with(f64::FORMAT)
            .with(Vec::<Text>::FORMAT)
            .with(Text::FORMAT)
            .with(NestedTestData::FORMAT)
            .with(Vec::<Vec<Text>>::FORMAT);

        fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
            writer.write_data(&self.number)?;
            writer.write_data(&self.floaty)?;
            writer.write_data(&self.text_list)?;
            writer.write_data(&self.text)?;
            writer.write_data(&self.nested)?;
            writer.write_data(&self.two_d)?;
            Ok(())
        }
    }

    impl Decodable for TestData {
        fn decode(
            &mut self,
            reader: &mut (impl ReadsDecodable + ?Sized),
            header: Option<DataHeader>,
        ) -> Result<(), CodecError> {
            let _ = Self::ensure_header(header, &[1])?;

            reader.read_data_into(&mut self.number)?;
            reader.read_data_into(&mut self.floaty)?;
            reader.read_data_into(&mut self.text_list)?;
            reader.read_data_into(&mut self.text)?;
            reader.read_data_into(&mut self.nested)?;
            reader.read_data_into(&mut self.two_d)?;

            Ok(())
        }
    }

    /// Simple data structure intended for nesting
    /// inside of a [`TestData`].
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NestedTestData {
        pub boolean: bool,
    }

    impl NestedTestData {
        pub fn typing() -> DataType {
            let blob_fields = vec![DataField {
                name: Text::from("boolean"),
                docs: None,
                typing: Type::Bool,
                optional: false,
                flattened: false,
            }];

            let data_fields = vec![];

            let typing = DataType::new(
                Text::from("NestedTestdata"),
                None,
                2,
                &blob_fields,
                &data_fields,
            );

            assert_eq!(Self::FORMAT, *typing.format());

            typing
        }
    }

    impl Encodable for NestedTestData {
        const FORMAT: Format = Format::data(2).with(bool::FORMAT);

        fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
            writer.write_data(&self.boolean)?;
            Ok(())
        }
    }

    impl Decodable for NestedTestData {
        fn decode(
            &mut self,
            reader: &mut (impl ReadsDecodable + ?Sized),
            header: Option<DataHeader>,
        ) -> Result<(), CodecError> {
            let _ = Self::ensure_header(header, &[2])?;

            reader.read_data_into(&mut self.boolean)?;

            Ok(())
        }
    }

    #[test]
    pub fn data_type_codec() {
        let data_type = TestData::typing();

        let mut encoded_data_type = vec![];
        encoded_data_type.write_data(&data_type).unwrap();
        let decoded_data_type = encoded_data_type.as_slice().read_data().unwrap();

        assert_eq!(data_type, decoded_data_type);
    }

    #[test]
    fn codes_unstructured_optionals() {
        let option: Option<u32> = Some(1337u32);
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        println!("encoded");
        let decoded_option = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded_option);

        // Do None values decode as None?
        let option: Option<u32> = None;
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded_option = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded_option);

        // Default values round-trip as Some(default).
        let option: Option<u32> = Some(0);
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded_option: Option<u32> = data.as_slice().read_data().expect("decoded");
        assert_eq!(Some(0), decoded_option);
    }

    #[test]
    fn codes_structured_optionals() {
        let option: Option<Text> = Some("Hello, World!".into());
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        println!("encoded");
        let decoded_option = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded_option);

        // Do None values decode as None?
        let option: Option<Text> = None;
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded_option = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded_option);

        // Default values round-trip as Some(default).
        let option: Option<Text> = Some("".into());
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded_option: Option<Text> = data.as_slice().read_data().expect("decoded");
        assert_eq!(Some("".into()), decoded_option);
    }

    #[test]
    fn codes_nested_optionals() {
        // None
        let option: Option<Option<u32>> = None;
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded: Option<Option<u32>> = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded);

        // Some(None)
        let option: Option<Option<u32>> = Some(None);
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded: Option<Option<u32>> = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded);

        // Some(Some(0)) — the previously unrepresentable case
        let option: Option<Option<u32>> = Some(Some(0));
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded: Option<Option<u32>> = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded);

        // Some(Some(42))
        let option: Option<Option<u32>> = Some(Some(42));
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded: Option<Option<u32>> = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded);
    }

    #[test]
    fn codes_optional_vec() {
        // None
        let option: Option<Vec<u16>> = None;
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded: Option<Vec<u16>> = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded);

        // Some(vec![])
        let option: Option<Vec<u16>> = Some(vec![]);
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded: Option<Vec<u16>> = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded);

        // Some(vec![42])
        let option: Option<Vec<u16>> = Some(vec![42]);
        let mut data = vec![];
        data.write_data(&option).expect("encoded");
        let decoded: Option<Vec<u16>> = data.as_slice().read_data().expect("decoded");
        assert_eq!(option, decoded);
    }

    /// Verifies that `ordinal()` and `from_ordinal()` are consistent:
    /// for every ordinal 0–255, if `from_ordinal` returns `Some(t)`,
    /// then `t.ordinal()` equals the original ordinal.
    #[test]
    fn ordinal_round_trip() {
        for ordinal in 0..=255u8 {
            if let Some(typ) = Type::from_ordinal(ordinal) {
                assert_eq!(
                    ordinal,
                    typ.ordinal(),
                    "from_ordinal({ordinal}) returned {typ:?} with ordinal {}",
                    typ.ordinal()
                );
            }
        }
    }
}
