//! Binary data types (i.e., bytes).
//!
//! ## Unstable: Text Encoding
//!
//! This module provides unstable utilities
//! for encoding and decoding byte slices in:
//!
//! - Hexadecimal (4 bits per character).
//! - Crockford Base32 (5 bits per character).
//!
//! In general, Hexadecimal is preferable for
//! debugging (it's a _bit_ more human readable),
//! while Base32 is preferable for network
//! transmissions (it's a _bit_ more efficient).
//!
//! > _Note_: Base32 is used over Base64 due to
//! > it's simpler padding-free codec, at the
//! > cost of reduced efficiency (Base64 encodes
//! > 6 bits per character).
use core::fmt::{Debug, Formatter, Write};

use snafu::Snafu;

use crate::{
    codec::{
        CodecError, DataHeader, Decodable, Encodable, Format, ReadsDecodable, WritesEncodable,
    },
    types::Text,
};

// Fixed-size `[u8; SIZE]` codec.
impl<const SIZE: usize> Encodable for [u8; SIZE] {
    /// Encoded as a [`Format::Blob(SIZE)`](Format::Blob) containing the raw bytes.
    ///
    /// When a fixed-size byte array is a field of a
    /// [`Format::Data`], its bytes are stored directly
    /// in the data's blob section.
    const FORMAT: Format = {
        assert!(
            SIZE <= u16::MAX as usize,
            "SIZE exceeds maximum blob size (u16::MAX)"
        );
        Format::Blob(SIZE as u16)
    };

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        writer.write_all(self)?;
        Ok(())
    }
}

impl<const SIZE: usize> Decodable for [u8; SIZE] {
    fn decode(
        &mut self,
        reader: &mut (impl ReadsDecodable + ?Sized),
        header: Option<DataHeader>,
    ) -> Result<(), CodecError> {
        Self::ensure_no_header(header)?;
        reader.read_exact(self)?;
        Ok(())
    }
}

/// Fixed-size sequence of `N` values of a fixed-size type `T`.
///
/// This is a zero-cost (`#[repr(transparent)]`)
/// wrapper around `[T; N]` which implements
/// [`Default`] (and therefore the codec traits'
/// container requirements) for _any_ `N`. Rust's
/// standard library only implements `Default` for
/// arrays of up to `32` elements, which would
/// prevent types like `Option<[u8; 64]>` or
/// `Vec<[u8; 64]>` from being decodable.
///
/// Code generated for codas with `array of N T`
/// fields uses this type for those fields.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Array<T, const N: usize>(pub [T; N]);

impl<T, const N: usize> Array<T, N> {
    /// The number of elements in this array.
    pub const LEN: usize = N;
}

impl<const N: usize> Array<u8, N> {
    /// Null ("empty") bytes initialized to `0`.
    pub const NULL: Self = Self([0; N]);

    /// Decodes a `hex` string into these bytes.
    pub fn from_hex(&mut self, hex: &str) -> Result<(), BinaryError> {
        fixed_bytes_from_hex(hex, &mut self.0)
    }

    /// Encodes a hex string from these bytes.
    pub fn to_hex(&self) -> Text {
        hex_from_bytes(&self.0)
    }
}

impl<T: Default, const N: usize> Default for Array<T, N> {
    fn default() -> Self {
        Self(core::array::from_fn(|_| T::default()))
    }
}

impl<T, const N: usize> From<[T; N]> for Array<T, N> {
    fn from(values: [T; N]) -> Self {
        Self(values)
    }
}

impl<T, const N: usize> From<Array<T, N>> for [T; N] {
    fn from(array: Array<T, N>) -> Self {
        array.0
    }
}

impl<T: Copy, const N: usize> TryFrom<&[T]> for Array<T, N> {
    type Error = core::array::TryFromSliceError;

    fn try_from(values: &[T]) -> Result<Self, Self::Error> {
        <[T; N]>::try_from(values).map(Self)
    }
}

impl<T, const N: usize> core::ops::Deref for Array<T, N> {
    type Target = [T; N];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, const N: usize> core::ops::DerefMut for Array<T, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T, const N: usize> core::borrow::Borrow<[T; N]> for Array<T, N> {
    fn borrow(&self) -> &[T; N] {
        &self.0
    }
}

impl<T, const N: usize> core::borrow::BorrowMut<[T; N]> for Array<T, N> {
    fn borrow_mut(&mut self) -> &mut [T; N] {
        &mut self.0
    }
}

impl<T: PartialEq, const N: usize> PartialEq<[T; N]> for Array<T, N> {
    fn eq(&self, other: &[T; N]) -> bool {
        self.0 == *other
    }
}

impl<T: PartialEq, const N: usize> PartialEq<Array<T, N>> for [T; N] {
    fn eq(&self, other: &Array<T, N>) -> bool {
        *self == other.0
    }
}

impl<T: Debug, const N: usize> Debug for Array<T, N> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

/// Byte arrays display as lowercase hexadecimal.
impl<const N: usize> core::fmt::Display for Array<u8, N> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        format_bytes_as_hex(f, &self.0)
    }
}

impl<T: Encodable, const N: usize> Encodable for Array<T, N> {
    /// Encoded as a [`Format::Blob`] containing each
    /// element's encoding, in order, with no headers.
    const FORMAT: Format = match T::FORMAT {
        Format::Blob(size) => {
            let total = size as usize * N;
            assert!(
                total <= u16::MAX as usize,
                "array size exceeds maximum blob size (u16::MAX)"
            );
            Format::Blob(total as u16)
        }
        _ => panic!("array elements must have a fixed size"),
    };

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        for value in &self.0 {
            value.encode(writer)?;
        }
        Ok(())
    }
}

impl<T: Encodable + Decodable, const N: usize> Decodable for Array<T, N> {
    fn decode(
        &mut self,
        reader: &mut (impl ReadsDecodable + ?Sized),
        header: Option<DataHeader>,
    ) -> Result<(), CodecError> {
        Self::ensure_no_header(header)?;
        for value in &mut self.0 {
            value.decode(reader, None)?;
        }
        Ok(())
    }
}

/// Serialized as a sequence of elements, matching
/// serde's own representation of arrays.
#[cfg(feature = "serde")]
impl<T: serde::Serialize, const N: usize> serde::Serialize for Array<T, N> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(&self.0)
    }
}

#[cfg(feature = "serde")]
impl<'de, T: serde::Deserialize<'de>, const N: usize> serde::Deserialize<'de> for Array<T, N> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        let values = alloc::vec::Vec::<T>::deserialize(deserializer)?;
        let length = values.len();
        <[T; N]>::try_from(values)
            .map(Self)
            .map_err(|_| D::Error::invalid_length(length, &"an array of length N"))
    }
}

/// Macro which generates a [new type](https://doc.rust-lang.org/rust-by-example/generics/new_types.html)
/// struct wrapping a fixed-size `[u8]` array, enabling sype-safe sharing.
#[macro_export]
macro_rules! sized_byte_array {
    (
        // Optional type metadata (e.g., docs).
        $(#[$meta:meta])*

        // Type name of the array
        $type_name:ident,

        // Fixed size of the array
        $array_size:expr
    ) => {

        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Copy, Clone)]
        pub struct $type_name([core::primitive::u8; $array_size]);

        impl $type_name {
            /// Size of this type, in bytes.
            pub const SIZE: usize = $array_size;

            /// Null ("empty") bytes initialized to `0`.
            pub const NULL: Self = Self([0; $array_size]);

            /// Decodes a `hex` string into these bytes.
            pub fn from_hex(&mut self, hex: &str) -> Result<(), $crate::types::binary::BinaryError> {
                $crate::types::binary::fixed_bytes_from_hex(hex, &mut self.0)
            }

            /// Encodes a hex string from these bytes.
            pub fn to_hex(&self) -> $crate::types::Text {
                $crate::types::binary::hex_from_bytes(&self.0)
            }
        }

        impl core::default::Default for $type_name {
            fn default() -> Self {
                Self::NULL
            }
        }

        impl core::convert::TryFrom<&[core::primitive::u8]> for $type_name {
            type Error = &'static str;

            fn try_from(bytes: &[core::primitive::u8]) -> Result<Self, Self::Error> {
                if bytes.len() == $array_size {
                    let mut this = Self::NULL;
                    this.copy_from_slice(bytes);
                    Ok(this)
                } else {
                    Err(stringify!(source bytes must be exactly $array_size long))
                }
            }
        }

        impl core::convert::From<[core::primitive::u8; $array_size]> for $type_name {
            fn from(bytes: [core::primitive::u8; $array_size]) -> Self {
                $type_name(bytes)
            }
        }

        impl core::convert::From<$type_name> for [core::primitive::u8; $array_size] {
            fn from(bytes: $type_name) -> Self {
                bytes.0
            }
        }

        impl core::borrow::Borrow<[core::primitive::u8; $array_size]> for $type_name {
            fn borrow(&self) -> &[core::primitive::u8; $array_size] {
                &self.0
            }
        }

        impl core::borrow::BorrowMut<[core::primitive::u8; $array_size]> for $type_name {
            fn borrow_mut(&mut self) -> &mut [core::primitive::u8; $array_size] {
                &mut self.0
            }
        }

        impl core::ops::Deref for $type_name {
            type Target = [core::primitive::u8];

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl core::ops::DerefMut for $type_name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl core::cmp::Eq for $type_name {}
        impl core::cmp::PartialEq for $type_name {
            fn eq(&self, other: &Self) -> core::primitive::bool {
                self.0 == other.0
            }
        }

        impl core::cmp::Ord for $type_name {
            fn cmp(&self, other: &Self) -> core::cmp::Ordering {
                self.0.cmp(&other.0)
            }
        }

        impl core::cmp::PartialOrd for $type_name {
            fn partial_cmp(&self, other: &Self) -> core::option::Option<core::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl core::hash::Hash for $type_name {
            fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
                self.0.hash(state);
            }
        }

        impl core::fmt::Display for $type_name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                $crate::types::binary::format_bytes_as_hex(f, &self.0)
            }
        }

        impl core::fmt::Debug for $type_name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                $crate::types::binary::format_bytes_as_hex(f, &self.0)
            }
        }

        impl $crate::codec::Encodable for $type_name {
            const FORMAT: $crate::codec::Format = <[core::primitive::u8; $array_size]>::FORMAT;

            fn encode(&self, writer: &mut (impl $crate::codec::WritesEncodable + ?Sized)) -> Result<(), $crate::codec::CodecError> {
                self.0.encode(writer)
            }
        }

        impl $crate::codec::Decodable for $type_name {
            fn decode(
                &mut self,
                reader: &mut (impl $crate::codec::ReadsDecodable + ?Sized),
                header: Option<$crate::codec::DataHeader>,
            ) -> Result<(), $crate::codec::CodecError> {
                self.0.decode(reader, header)
            }
        }
    };
}

/// Decodes a vector of bytes from a `hex` into `bytes`.
///
/// If an error is returned, the contents of `bytes` are undefined.
pub fn fixed_bytes_from_hex<const SIZE: usize>(
    hex: &str,
    bytes: &mut [u8; SIZE],
) -> Result<(), BinaryError> {
    let length = hex.len();
    if !length.is_multiple_of(2) {
        return Err(BinaryError::UnevenHex { actual: length });
    } else if length / 2 != SIZE {
        return Err(BinaryError::WrongHexLength {
            expected: length / 2,
            actual: length,
        });
    }

    let mut hex_bytes = hex.as_bytes().iter();
    let mut i = 0;
    while let (Some(h), Some(l)) = (hex_bytes.next(), hex_bytes.next()) {
        let h = match h {
            b'0'..=b'9' => h - b'0',
            b'a'..=b'f' => h - b'a' + 10,
            b'A'..=b'F' => h - b'A' + 10,
            character => {
                return Err(BinaryError::UnexpectedHexCharacter {
                    character: *character,
                })
            }
        };

        let l = match l {
            b'0'..=b'9' => l - b'0',
            b'a'..=b'f' => l - b'a' + 10,
            b'A'..=b'F' => l - b'A' + 10,
            character => {
                return Err(BinaryError::UnexpectedHexCharacter {
                    character: *character,
                })
            }
        };

        bytes[i] = (h << 4) | l;
        i += 1;
    }

    Ok(())
}

/// Decodes a vector of bytes from a `hex`.
pub fn bytes_from_hex(hex: &str) -> Result<alloc::vec::Vec<u8>, BinaryError> {
    let length = hex.len();
    if !length.is_multiple_of(2) {
        return Err(BinaryError::UnevenHex { actual: length });
    }

    let mut hex_bytes = hex.as_bytes().iter();
    let mut bytes = alloc::vec::Vec::with_capacity(length / 2);
    while let (Some(h), Some(l)) = (hex_bytes.next(), hex_bytes.next()) {
        let h = match h {
            b'0'..=b'9' => h - b'0',
            b'a'..=b'f' => h - b'a' + 10,
            b'A'..=b'F' => h - b'A' + 10,
            character => {
                return Err(BinaryError::UnexpectedHexCharacter {
                    character: *character,
                })
            }
        };

        let l = match l {
            b'0'..=b'9' => l - b'0',
            b'a'..=b'f' => l - b'a' + 10,
            b'A'..=b'F' => l - b'A' + 10,
            character => {
                return Err(BinaryError::UnexpectedHexCharacter {
                    character: *character,
                })
            }
        };

        bytes.push((h << 4) | l)
    }

    Ok(bytes)
}

/// Lookup table for hexadecimal character codes.
#[rustfmt::skip]
static HEX_LUT: [u8; 16] = [
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7',
    b'8', b'9', b'a', b'b', b'c', b'd', b'e', b'f',
];

/// Returns a lowercase hexadecimal string encoded from `bytes`.
pub fn hex_from_bytes(bytes: &[u8]) -> Text {
    let mut text = alloc::string::String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(HEX_LUT[(byte >> 4) as usize] as char);
        text.push(HEX_LUT[(byte & 0xF) as usize] as char);
    }

    text.into()
}

/// Encodes `bytes` to `fmt` as a lowercase hexadecimal string.
pub fn format_bytes_as_hex(fmt: &mut Formatter, bytes: &[u8]) -> Result<(), core::fmt::Error> {
    for byte in bytes {
        fmt.write_char(HEX_LUT[(byte >> 4) as usize] as char)?;
        fmt.write_char(HEX_LUT[(byte & 0xF) as usize] as char)?;
    }

    Ok(())
}

/// ## Unstable
///
/// Encodes `bytes` into a new Base32-encoded text.
///
/// This implementation (and the corresponding [`decoder`](base32_to_bytes))
/// is based off of a combination of:
///
/// - https://www.crockford.com/base32.html
/// - https://ptrchm.com/posts/base32-explained/
/// - https://github.com/andreasots/base32
///
/// With a specific focus on supporting _only_ the `Crockford` variant of Base32.
pub fn base32_from_bytes(bytes: &[u8]) -> Text {
    let mut base32 = alloc::vec::Vec::with_capacity(bytes.len().div_ceil(4) * BASE32_BITS_PER_CHAR);

    for chunk in bytes.chunks(BASE32_BITS_PER_CHAR) {
        // Compress the chunks into a number (40 bits).
        let mut condensed_chunk: u64 = 0;
        for i in 0..BASE32_BITS_PER_CHAR {
            condensed_chunk = (condensed_chunk << 8) + *chunk.get(i).unwrap_or(&0) as u64;
        }

        // Push ASCII character codes, `BASE32_CHUNK_SIZE`-bits at a time.
        for i in (0..8).rev() {
            base32.push(
                BASE32_LUT[((condensed_chunk >> (BASE32_BITS_PER_CHAR * i)) & 0x1F) as usize],
            );
        }
    }

    // Trim padding characters.
    if !bytes.len().is_multiple_of(BASE32_BITS_PER_CHAR) {
        let len = base32.len();
        let num_extra = 8 - (bytes.len() % BASE32_BITS_PER_CHAR * 8 + 4) / BASE32_BITS_PER_CHAR;
        base32.truncate(len - num_extra);
    }

    unsafe { alloc::string::String::from_utf8_unchecked(base32) }.into()
}

/// ## Unstable
///
/// Decodes `base32`-encoded text into bytes.
pub fn base32_to_bytes(base32: Text) -> Result<alloc::vec::Vec<u8>, BinaryError> {
    if !base32.is_ascii() {
        todo!()
    }
    let base32 = base32.as_bytes();

    let output_length = base32.len() * BASE32_BITS_PER_CHAR / 8;
    let mut bytes = alloc::vec::Vec::with_capacity(
        (output_length + 4) / BASE32_BITS_PER_CHAR * BASE32_BITS_PER_CHAR,
    );

    for chunk in base32.chunks(8) {
        // Compress the chunks into a number.
        let mut condensed_chunk: u64 = 0;
        for i in 0..8 {
            condensed_chunk = (condensed_chunk << BASE32_BITS_PER_CHAR)
                + base32_char_index(*chunk.get(i).unwrap_or(&b'0'))? as u64;
        }

        // Extract bytes from the number.
        for i in (0..BASE32_BITS_PER_CHAR).rev() {
            bytes.push(((condensed_chunk >> (8 * i)) & 0xFF) as u8);
        }
    }

    // Trim implicit padding.
    bytes.truncate(output_length);

    Ok(bytes)
}

/// The number of bits encoded by each
/// character in Base32-encoded text.
const BASE32_BITS_PER_CHAR: usize = 5;

/// Lookup table for Base32 character codes.
#[rustfmt::skip]
static BASE32_LUT: [u8; 32] = [
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7',
    b'8', b'9', b'A', b'B', b'C', b'D', b'E', b'F',
    b'G', b'H', b'J', b'K', b'M', b'N', b'P', b'Q',
    b'R', b'S', b'T', b'V', b'W', b'X', b'Y', b'Z',
];

/// Returns the index in [`BASE32_LUT`]
/// corresponding to `character`.
const fn base32_char_index(character: u8) -> Result<u8, BinaryError> {
    match character {
        // Numbers.
        c @ b'0'..=b'9' => Ok(c - b'0'),

        // Letters.
        c @ b'A'..=b'H' => Ok(c - b'A' + 10),
        c @ b'J'..=b'K' => Ok(c - b'J' + 18),
        c @ b'M'..=b'N' => Ok(c - b'M' + 20),
        c @ b'P'..=b'T' => Ok(c - b'P' + 22),
        c @ b'V'..=b'Z' => Ok(c - b'V' + 27),

        // Lowercase letters.
        c @ b'a'..=b'h' => Ok(c - b'a' + 10),
        c @ b'j'..=b'k' => Ok(c - b'j' + 18),
        c @ b'm'..=b'n' => Ok(c - b'm' + 20),
        c @ b'p'..=b't' => Ok(c - b'p' + 22),
        c @ b'v'..=b'z' => Ok(c - b'v' + 27),

        // Unsupported characters.
        _ => Err(BinaryError::UnexpectedHexCharacter { character }),
    }
}

#[derive(Snafu, Debug)]
pub enum BinaryError {
    #[snafu(display(
        "expected a hexadecimal string containing an even number of bytes, not {actual} byte(s)"
    ))]
    UnevenHex { actual: usize },

    #[snafu(display(
        "expected a hexadecimal string containing {expected} bytes, not {actual} byte(s)"
    ))]
    WrongHexLength { expected: usize, actual: usize },

    #[snafu(display("hexadecimal string contained an unexpected character code: {character}"))]
    UnexpectedHexCharacter { character: u8 },
}

#[cfg(test)]
mod test {
    use crate::codec::{ReadsDecodable, WritesEncodable};

    use super::*;

    #[test]
    fn test_sized_byte_array_codec() {
        sized_byte_array!(TestArray, 9);
        let value: TestArray = TestArray([1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        let mut decoded = TestArray::NULL;
        encoded
            .as_slice()
            .read_data_into(&mut decoded)
            .expect("decoded");
        assert_eq!(value, decoded);
    }

    #[test]
    fn test_u8_array_codec() {
        let value: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");

        // Fixed-size arrays encode as raw blobs, with no header.
        assert_eq!(value.len(), encoded.len());

        let mut decoded = [0u8; 8];
        encoded
            .as_slice()
            .read_data_into(&mut decoded)
            .expect("decoded");
        assert_eq!(value, decoded);
    }

    #[test]
    fn test_array_codec() {
        // Arrays encode as headerless raw elements.
        let value = Array([1u8, 2, 3, 4, 5, 6, 7, 8]);
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        assert_eq!(Array::<u8, 8>::LEN, encoded.len());
        let decoded: Array<u8, 8> = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded);

        // Non-byte elements encode in order, each
        // in little-endian, with no headers.
        let value = Array([1.0f32, 2.0, 3.0]);
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        assert_eq!(12, encoded.len());
        assert_eq!(&1.0f32.to_le_bytes(), &encoded[0..4]);
        assert_eq!(&2.0f32.to_le_bytes(), &encoded[4..8]);
        assert_eq!(&3.0f32.to_le_bytes(), &encoded[8..12]);
        let decoded: Array<f32, 3> = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded);

        // Nested arrays are flattened raw bytes.
        let value: Array<Array<f32, 3>, 3> =
            Array([Array([1.0; 3]), Array([2.0; 3]), Array([3.0; 3])]);
        assert_eq!(Format::Blob(36), Array::<Array<f32, 3>, 3>::FORMAT);
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        assert_eq!(36, encoded.len());
        let decoded: Array<Array<f32, 3>, 3> = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded);

        // Arrays implement `Default` for any length,
        // unlike raw `[T; N]` arrays (capped at 32).
        let value: Array<u8, 64> = Array::default();
        assert_eq!(value, [0u8; 64]);

        // ...which makes optional arrays of any
        // length decodable...
        let value: Option<Array<u8, 64>> = Some(Array([42u8; 64]));
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        let decoded: Option<Array<u8, 64>> = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded);

        let value: Option<Array<u8, 64>> = None;
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        let decoded: Option<Array<u8, 64>> = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded);

        // ...along with lists of arrays of any length,
        // which encode as tightly-packed elements
        // after a single header.
        let value: Vec<Array<f32, 3>> = vec![Array([1.0; 3]), Array([2.0; 3])];
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        assert_eq!(8 + 2 * 12, encoded.len());
        let decoded: Vec<Array<f32, 3>> = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded);
    }

    #[test]
    fn test_hex() {
        // "Ferrous" in hexadecimal-encoded UTF-8 encoding.
        const TEST_STR: &str = "Ferrous";
        const TEST_HEX_UTF8_STR: &str = "466572726f7573";

        // Decode hexadecimal dynamic vector.
        let bytes = bytes_from_hex(TEST_HEX_UTF8_STR).unwrap();
        assert_eq!(TEST_STR.as_bytes(), bytes);

        // Decode hexadecimal into a fixed-length array.
        let mut bytes = [0u8; 7];
        assert!(fixed_bytes_from_hex(TEST_HEX_UTF8_STR, &mut bytes).is_ok());
        assert_eq!(TEST_STR.as_bytes(), bytes);

        // Encode bytes into hexadecimal.
        let hex = hex_from_bytes(TEST_STR.as_bytes());
        assert_eq!(TEST_HEX_UTF8_STR, hex);

        // Write bytes into hexadecimal via a formatter.
        sized_byte_array!(TestBytes, 7);
        let mut bytes = TestBytes::NULL;
        assert!(bytes.from_hex(TEST_HEX_UTF8_STR).is_ok());
        assert_eq!(TEST_STR.as_bytes(), bytes.as_ref());
        let hex = bytes.to_string();
        assert_eq!(TEST_HEX_UTF8_STR, hex);
    }

    #[test]
    fn test_base32() {
        assert_eq!(
            "Z0Z0Z0Z0",
            base32_from_bytes(&[0xF8, 0x3E, 0x0F, 0x83, 0xE0]),
        );

        assert_eq!(
            "0Z0Z0Z0Z",
            base32_from_bytes(&[0x07, 0xC1, 0xF0, 0x7C, 0x1F]),
        );

        assert_eq!(
            vec![0xF8, 0x3E, 0x0F, 0x83, 0xE0],
            base32_to_bytes("Z0Z0Z0Z0".into()).unwrap(),
        );

        assert_eq!(
            vec![0x07, 0xC1, 0xF0, 0x7C, 0x1F],
            base32_to_bytes("0Z0Z0Z0Z".into()).unwrap(),
        );

        assert_eq!(
            "C5Q20XBEC5P6JSVECNJ20X35EDT20WVME9MPWSR",
            base32_from_bytes("an unaligned test string".as_bytes())
        );

        assert_eq!(
            "an unaligned test string",
            String::from_utf8_lossy(
                base32_to_bytes("C5Q20XBEC5P6JSVECNJ20X35EDT20WVME9MPWSR".into())
                    .unwrap()
                    .as_slice()
            )
        );

        assert_eq!(
            "an unaligned test string",
            String::from_utf8_lossy(
                base32_to_bytes(
                    "C5Q20XBEC5P6JSVECNJ20X35EDT20WVME9MPWSR"
                        .to_ascii_lowercase()
                        .into()
                )
                .unwrap()
                .as_slice()
            )
        );
    }
}
