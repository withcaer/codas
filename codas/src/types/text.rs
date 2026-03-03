//! UTF-8 encoded text data types.
use core::{
    fmt::{Debug, Display},
    hash::Hash,
    ops::Deref,
};

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::string::ToString;
use alloc::sync::Arc;

use crate::codec::{
    CodecError, DataHeader, Decodable, Encodable, Format, UnexpectedDataFormatSnafu,
    WritesEncodable,
};

/// UTF-8 encoded text data.
///
/// This type may be used anywhere a `&str`
/// works. To get a _mutable_ reference to
/// the text (i.e., a `&mut String`), use
/// [`Self::to_mut`].
pub enum Text {
    /// Statically allocated text (`&'static str`).
    Static(&'static str),

    /// Dynamic heap-allocated text.
    Dynamic(Arc<String>),
}

impl Text {
    /// Empty text.
    pub const EMPTY: Text = Text::Static("");

    /// Returns a [`Text::Static`] containing
    /// the provided string.
    pub const fn from(s: &'static str) -> Self {
        Text::Static(s)
    }

    /// Returns a string slice containing the entire text.
    pub fn as_str(&self) -> &str {
        match self {
            Text::Static(t) => t,
            Text::Dynamic(t) => t.as_str(),
        }
    }

    /// Returns a mutable reference to this text.
    ///
    /// If this text is not [`Text::Static`], the
    /// contents of this text _may_ be cloned to
    /// obtain a mutable reference. Use
    /// [`Self::to_cleared_mut`] when wanting to
    /// reuse the same allocation for _new_ text.
    pub fn to_mut(&mut self) -> &mut String {
        match self {
            Text::Static(t) => {
                *self = Text::Dynamic(t.to_string().into());
                self.to_mut()
            }
            Text::Dynamic(t) => Arc::make_mut(t),
        }
    }

    /// Returns a mutable reference to this text,
    /// clearing the contents and reserving _at least_
    /// `capacity` bytes in the text buffer.
    ///
    /// Use this function instead of [`Self::to_mut`]
    /// to reuse this text for a _new_ string, potentially
    /// avoiding unneeded allocations.
    pub fn to_cleared_mut(&mut self, capacity: usize) -> &mut String {
        match self {
            Text::Static(_) => {
                *self = Text::Dynamic(String::with_capacity(capacity).into());
                self.to_mut()
            }
            Text::Dynamic(t) => {
                // Clear the borrowed string if we're the only owner.
                if let Some(t) = Arc::get_mut(t) {
                    t.clear();
                    t.reserve(capacity);
                }

                self.to_mut()
            }
        }
    }
}

// Codec Traits ///////////////////////

impl Encodable for Text {
    /// Encoded as a \[[`u8`]\] containing the
    /// result of [`str::as_bytes`].
    const FORMAT: Format = <[u8]>::FORMAT;

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        self.as_bytes().encode(writer)
    }

    fn encode_header(
        &self,
        writer: &mut (impl WritesEncodable + ?Sized),
    ) -> Result<(), CodecError> {
        self.as_bytes().encode_header(writer)
    }
}

impl Decodable for Text {
    fn decode(
        &mut self,
        reader: &mut (impl crate::codec::ReadsDecodable + ?Sized),
        header: Option<crate::codec::DataHeader>,
    ) -> Result<(), crate::codec::CodecError> {
        let header = Self::ensure_header(header, &[0])?;

        // Text is always blob_size=1, data_fields=0.
        if header.format.blob_size != 1 || header.format.data_fields != 0 {
            return UnexpectedDataFormatSnafu {
                expected: Self::FORMAT,
                actual: Some(header),
            }
            .fail();
        }

        match self {
            Text::Static(_) => {
                // Reference the empty text for empty strings.
                if header.count == 0 {
                    *self = Text::EMPTY;

                // Dynamically allocate for non-empty strings.
                } else {
                    let mut string = String::with_capacity(header.count as usize);
                    try_decode_string(reader, header, &mut string)?;
                    *self = Text::Dynamic(string.into());
                }
            }
            Text::Dynamic(text) => {
                // Clear the text contents, or reference
                // the empty text, for empty strings.
                if header.count == 0 {
                    if let Some(text) = Arc::get_mut(text) {
                        text.clear();
                    } else {
                        *self = Text::EMPTY;
                    }
                } else {
                    match Arc::get_mut(text) {
                        // Read data directly into the string buffer if
                        // it's not shared.
                        Some(text) => try_decode_string(reader, header, text)?,

                        // Dynamically allocate a new string.
                        None => {
                            let mut string = String::with_capacity(header.count as usize);
                            try_decode_string(reader, header, &mut string)?;
                            *self = Text::Dynamic(string.into());
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Tries to decode the remaining string data
/// from `reader` for `header` into `string`.
///
/// If decoding fails for any reason, the returned
/// `string` will be empty.
fn try_decode_string(
    reader: &mut (impl crate::codec::ReadsDecodable + ?Sized),
    header: DataHeader,
    string: &mut String,
) -> Result<(), CodecError> {
    unsafe {
        // Truncate and pad the bytes to fit the new text.
        let bytes = string.as_mut_vec();
        bytes.resize(header.count as usize, 0);

        // Read in the raw bytes.
        if let Err(e) = reader.read_exact(bytes) {
            bytes.clear();
            return Err(e);
        }

        // Validate the bytes.
        if alloc::str::from_utf8(bytes).is_err() {
            bytes.clear();

            // FIXME: If a string contains malformed UTF-8 bytes,
            //        should decoding fail? Or should the string
            //        be silently interpreted as empty data, which
            //        could lead to data loss?
        }
    }

    Ok(())
}

// Common Traits //////////////////////
impl Clone for Text {
    fn clone(&self) -> Self {
        match self {
            Self::Static(text) => Self::Static(text),
            Self::Dynamic(text) => Self::Dynamic(text.clone()),
        }
    }
}

impl Default for Text {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        match self {
            Text::Static(t) => t,
            Text::Dynamic(t) => t,
        }
    }
}

impl From<&str> for Text {
    fn from(value: &str) -> Self {
        Self::Dynamic(Arc::new(value.to_owned()))
    }
}

impl From<String> for Text {
    fn from(value: String) -> Self {
        Self::Dynamic(value.into())
    }
}

// Self-equivalency traits ////////////

impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl Eq for Text {}

impl PartialOrd for Text {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Text {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.deref().cmp(other.deref())
    }
}

impl Hash for Text {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

// Other equivalency traits ///////////

impl PartialEq<str> for Text {
    fn eq(&self, other: &str) -> bool {
        self.deref() == other
    }
}

impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        self.deref() == *other
    }
}

impl PartialEq<Text> for &str {
    fn eq(&self, other: &Text) -> bool {
        *self == other.deref()
    }
}

impl PartialEq<Text> for str {
    fn eq(&self, other: &Text) -> bool {
        self == other.deref()
    }
}

impl PartialEq<String> for Text {
    fn eq(&self, other: &String) -> bool {
        self.deref() == other
    }
}

impl PartialEq<Text> for String {
    fn eq(&self, other: &Text) -> bool {
        self == other.deref()
    }
}

// Formatting traits //////////////////

impl Display for Text {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        <str as Display>::fmt(self, f)
    }
}

impl Debug for Text {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        <str as Debug>::fmt(self, f)
    }
}

// Serde traits ///////////////////////

#[cfg(feature = "serde")]
impl serde::Serialize for Text {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Text::Static(text) => text.serialize(serializer),
            Text::Dynamic(text) => text.as_str().serialize(serializer),
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Text {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::Dynamic(String::deserialize(deserializer)?.into()))
    }
}

#[cfg(test)]
mod test {
    use crate::codec::ReadsDecodable;

    use super::*;

    #[test]
    fn test_text_string_eq() {
        let text = Text::from("Hello");
        let string = String::from("Hello");
        assert_eq!(text, string);
        assert_eq!(string, text);
        assert_eq!(text, string.as_str());
        assert_eq!(string.as_str(), text);

        let text = Text::from("World ♥️");
        let string = String::from("World ♥️");
        assert_eq!(text, string);
        assert_eq!(string, text);
        assert_eq!(text, string.as_str());
        assert_eq!(string.as_str(), text);

        let text = Text::from("We Like Text");
        let string = String::from("Because Strings are Worse?");
        assert_ne!(text, string);
        assert_ne!(string, text);
        assert_ne!(text, string.as_str());
        assert_ne!(string.as_str(), text);
    }

    #[test]
    fn test_text_codec() {
        let value = Text::from("Hello, world!");
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        let decoded: Text = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded);
    }
}
