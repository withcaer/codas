//! Codec decoder implementations.
use snafu::ensure;

use crate::stream::Reads;

use super::{
    encode::Encodable, CodecError, DataFormat, DataHeader, Format, UnexpectedDataFormatSnafu,
};

/// Default size used for temporary,
/// stack-allocated buffers.
pub const TEMP_BUFFER_SIZE: usize = 1024;

/// A thing that decodes from
/// [`codec`](super)-compliant data.
pub trait Decodable: Encodable {
    /// Decodes data with `header` from `reader` into this thing.
    ///
    /// In most cases, [`ReadsDecodable::read_data`] or
    /// [`ReadsDecodable::read_data_into`] should be used
    /// instead of calling this function directly.
    ///
    /// If `Self`'s [`Encodable::FORMAT`] is not
    /// [`structured`](`crate::codec::Format::is_structured`),
    /// `header` will be `None`, and this function
    /// should decode a number of bytes equal to
    /// it's blob size.
    fn decode(
        &mut self,
        reader: &mut (impl ReadsDecodable + ?Sized),
        header: Option<DataHeader>,
    ) -> Result<(), CodecError>;

    /// Returns `Ok(header)` iff `header` exists and
    /// matches `Self`'s [`Encodable::FORMAT`].
    ///
    /// Formats with a non-zero ordinal must match
    /// the `header`'s ordinal, anonymous formats
    /// must match `header` entirely, and fluid
    /// formats match any header.
    ///
    /// [`Format::Data`] headers must have a `count` of `1`.
    #[inline(always)]
    fn ensure_header(header: Option<DataHeader>) -> Result<DataHeader, CodecError> {
        let header = header.ok_or_else(|| {
            UnexpectedDataFormatSnafu {
                expected: Self::FORMAT,
                actual: header,
            }
            .build()
        })?;

        let matches = match Self::FORMAT {
            Format::Fluid => true,
            Format::Blob(_) => false,
            Format::Data(expected) | Format::Sequence(expected) => {
                if expected.ordinal != 0 {
                    header.format.ordinal == expected.ordinal
                } else {
                    header.format == expected
                }
            }
        };
        ensure!(
            matches,
            UnexpectedDataFormatSnafu {
                expected: Self::FORMAT,
                actual: Some(header),
            }
        );
        ensure!(
            !matches!(Self::FORMAT, Format::Data(_)) || header.count == 1,
            super::UnsupportedCountSnafu {
                count: header.count
            }
        );

        Ok(header)
    }

    /// Returns `Ok(())` iff `header` is `None`.
    #[inline(always)]
    fn ensure_no_header(header: Option<DataHeader>) -> Result<(), CodecError> {
        ensure!(
            header.is_none(),
            UnexpectedDataFormatSnafu {
                expected: Self::FORMAT,
                actual: header,
            }
        );

        Ok(())
    }
}

/// A thing that reads [`Decodable`] data.
///
/// This trait is automatically implemented for all [`Reads`].
/// This automatic implementation wraps each top-level decoder in
/// a [`LimitedReader`] with default limits of [`DEFAULT_MAX_BYTES`]
/// and [`DEFAULT_MAX_DEPTH`].
///
/// For custom limits, construct a [`LimitedReader`] explicitly
/// instead of using this trait's blanket implementation.
pub trait ReadsDecodable {
    /// Reads bytes into `buf`, returning the number
    /// of bytes read.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, CodecError>;

    /// Reads _exactly_ `buf.len()` bytes into `buf`.
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), CodecError> {
        let mut read = 0;
        while read < buf.len() {
            let n = self.read(&mut buf[read..])?;
            if n == 0 {
                return Err(CodecError::UnexpectedEof);
            }
            read += n;
        }
        Ok(())
    }

    /// Called when entering a nested data scope during decoding.
    fn enter_scope(&mut self) -> Result<(), CodecError> {
        Ok(())
    }

    /// Called when exiting a nested data scope during decoding.
    fn exit_scope(&mut self) {}

    /// Reads and decodes a sequence of data into
    /// a new, default instance of `T`.
    ///
    /// This function will attempt to read a [`DataHeader`]
    /// if the `data`'s [`Format::is_structured`](crate::codec::Format::is_structured).
    fn read_data<T: Decodable + Default>(&mut self) -> Result<T, CodecError> {
        let mut default = T::default();
        self.read_data_into(&mut default)?;
        Ok(default)
    }

    /// Reads and decodes a sequence of data into `data`.
    ///
    /// This function will attempt to read a [`DataHeader`]
    /// if the `data`'s [`Format::is_structured`](crate::codec::Format::is_structured).
    fn read_data_into<T: Decodable>(&mut self, data: &mut T) -> Result<(), CodecError> {
        if T::FORMAT.is_structured() {
            let mut guard = DecodingScope::enter(self)?;
            let header: DataHeader = guard.read_data()?;
            data.decode(&mut *guard, Some(header))?;
        } else {
            data.decode(self, None)?;
        }

        Ok(())
    }

    /// Reads and decodes one sequence of `data` within a greater
    /// sequence with `header`, per [`Format::as_sequence`].
    fn read_next_data_into<T: Decodable>(
        &mut self,
        header: DataHeader,
        data: &mut T,
    ) -> Result<(), CodecError> {
        match T::FORMAT {
            Format::Blob(_) => data.decode(self, None),
            Format::Data(_) => data.decode(
                self,
                Some(DataHeader {
                    count: 1,
                    format: header.format,
                }),
            ),
            Format::Sequence(_) | Format::Fluid => self.read_data_into(data),
        }
    }

    /// Skips to the end of the next `length` bytes of data.
    fn skip_blob(&mut self, length: usize) -> Result<(), CodecError> {
        let mut skipped = 0;
        let mut buf = [0; TEMP_BUFFER_SIZE];
        while skipped < length {
            let remaining = length - skipped;
            let n = if remaining < TEMP_BUFFER_SIZE {
                self.read(&mut buf[..remaining])?
            } else {
                self.read(&mut buf)?
            };
            if n == 0 {
                return Err(CodecError::UnexpectedEof);
            }
            skipped += n;
        }
        Ok(())
    }

    /// Skips to the end of the next encoded sequence of data,
    /// returning the total number of bytes skipped.
    fn skip_data(&mut self) -> Result<usize, CodecError> {
        let mut guard = DecodingScope::enter(self)?;
        let mut read = 0;

        // Decode data header.
        let header: DataHeader = guard.read_data()?;
        read += DataHeader::FORMAT.as_data_format().blob_size as usize;
        let data_format = header.format;

        // Decode all data in the sequence, skipping
        // their blobs and recursively skipping data fields.
        for _ in 0..header.count {
            read += guard.skip_data_with_format(data_format)?;
        }

        Ok(read)
    }

    /// Skips to the end of the next encoded instance
    /// of data with `format`, returning the total number
    /// of bytes skipped.
    fn skip_data_with_format(&mut self, format: DataFormat) -> Result<usize, CodecError> {
        let mut read = 0;

        // Skip the blob.
        self.skip_blob(format.blob_size as usize)?;
        read += format.blob_size as usize;

        // Skip all data fields recursively.
        for _ in 0..format.data_fields {
            read += self.skip_data()?;
        }

        Ok(read)
    }
}

/// [`Format::Data`] field decoder.
///
/// Fields must be read in their encoded order, which means
/// that every fixed-size "blob" field must be read in-order,
/// followed by every structured "data" field in-order.
///
/// This decoder reconciles fields known to the decoder
/// with the actual fields in a decoded data's [`DataHeader`]:
///
/// - Known fields present in the header are decoded.
/// - Known fields missing from the header are "decoded"
///   as their default values.
/// - Unknown fields present in the header are skipped
///   when [`FieldReader::finish`] is called.
///
/// Here's an example:
///
/// ```rust
/// # use codas::codec::*;
/// # use codas::types::Text;
///
/// /// Example data.
/// #[derive(Default)]
/// struct Data { number: u32, text: Text }
///
/// impl Encodable for Data {
///     const FORMAT: Format = Format::data(1).with(u32::FORMAT).with(Text::FORMAT);
///     fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
///
///         // Encode the `number` then the `text`.
///         writer.write_data(&self.number)?;
///         writer.write_data(&self.text)
///     }
/// }
///
/// impl Decodable for Data {
///     fn decode(
///         &mut self,
///         reader: &mut (impl ReadsDecodable + ?Sized),
///         header: Option<DataHeader>,
///     ) -> Result<(), CodecError> {
///
///         // Verify the header is valid.
///         let header = Self::ensure_header(header)?;
///         let mut fields = FieldReader::new(reader, header);
///
///         // Decode the `number` then the `text`.
///         fields.read(&mut self.number)?;
///         fields.read(&mut self.text)?;
///
///         // Finish decoding, skipping any unknown fields.
///         fields.finish()
///     }
/// }
/// ```
#[must_use = "call `finish` to skip any fields unknown to the decoder"]
pub struct FieldReader<'a, R: ReadsDecodable + ?Sized> {
    reader: &'a mut R,

    /// Blob bytes in the header that haven't been read.
    blob_remaining: u16,

    /// Structured fields in the header that haven't been read.
    data_remaining: u8,

    /// True once a structured field has been read,
    /// after which no blob fields may be read.
    reading_data: bool,
}

impl<'a, R: ReadsDecodable + ?Sized> FieldReader<'a, R> {
    /// Begins reading the fields of a single data
    /// with `header` from `reader`.
    ///
    /// `header` should be validated by
    /// [`Decodable::ensure_header`] first.
    pub fn new(reader: &'a mut R, header: DataHeader) -> Self {
        Self {
            reader,
            blob_remaining: header.format.blob_size,
            data_remaining: header.format.data_fields,
            reading_data: false,
        }
    }

    /// Reads the next `field`, or sets it to its
    /// default value if the header has no more fields
    /// of its kind.
    pub fn read<T: Decodable + Default>(&mut self, field: &mut T) -> Result<(), CodecError> {
        match T::FORMAT {
            Format::Blob(size) => {
                debug_assert!(
                    !self.reading_data,
                    "blob fields must be read before structured fields"
                );

                if self.blob_remaining >= size {
                    field.decode(self.reader, None)?;
                    self.blob_remaining -= size;
                } else if self.blob_remaining == 0 {
                    *field = T::default();
                } else {
                    // A blob ending partway through a field is malformed.
                    return Err(CodecError::MissingBlobLength {
                        length: size - self.blob_remaining,
                    });
                }
            }

            Format::Data(_) | Format::Sequence(_) | Format::Fluid => {
                self.reading_data = true;

                if self.data_remaining > 0 {
                    self.reader.read_data_into(field)?;
                    self.data_remaining -= 1;
                } else {
                    *field = T::default();
                }
            }
        }

        Ok(())
    }

    /// Finish decoding, skipping any fields in the
    /// header which weren't read.
    pub fn finish(self) -> Result<(), CodecError> {
        if self.blob_remaining > 0 {
            self.reader.skip_blob(self.blob_remaining as usize)?;
        }

        for _ in 0..self.data_remaining {
            self.reader.skip_data()?;
        }

        Ok(())
    }
}

/// RAII guard around a [`ReadsDecodable`] that calls
/// [`ReadsDecodable::exit_scope`] on drop.
///
/// Created via [`DecodingScope::enter`], which calls
/// [`enter_scope`](ReadsDecodable::enter_scope) on construction.
pub(crate) struct DecodingScope<'a, R: ReadsDecodable + ?Sized> {
    reader: &'a mut R,
}

impl<'a, R: ReadsDecodable + ?Sized> DecodingScope<'a, R> {
    /// Enters a scope on `reader` and returns a guard
    /// that exits the scope when dropped.
    pub(crate) fn enter(reader: &'a mut R) -> Result<Self, CodecError> {
        reader.enter_scope()?;
        Ok(Self { reader })
    }
}

impl<R: ReadsDecodable + ?Sized> Drop for DecodingScope<'_, R> {
    fn drop(&mut self) {
        self.reader.exit_scope();
    }
}

impl<R: ReadsDecodable + ?Sized> core::ops::Deref for DecodingScope<'_, R> {
    type Target = R;
    fn deref(&self) -> &R {
        self.reader
    }
}

impl<R: ReadsDecodable + ?Sized> core::ops::DerefMut for DecodingScope<'_, R> {
    fn deref_mut(&mut self) -> &mut R {
        self.reader
    }
}

impl<R: Reads> ReadsDecodable for R {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, CodecError> {
        Ok(Reads::read(self, buf)?)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), CodecError> {
        Ok(Reads::read_exact(self, buf)?)
    }

    fn read_data_into<T: Decodable>(&mut self, data: &mut T) -> Result<(), CodecError> {
        LimitedReader::new(&mut *self).read_data_into(data)
    }

    fn skip_data(&mut self) -> Result<usize, CodecError> {
        LimitedReader::new(&mut *self).skip_data()
    }
}

/// A [`Reads`] wrapper that enforces byte and depth limits
/// during decoding, protecting against malicious or malformed input.
///
/// The blanket [`ReadsDecodable`] implementation automatically wraps
/// each top-level decode in a limited reader with default limits.
/// Construct a `LimitedReader` explicitly to override the defaults:
///
/// ```
/// use codas::codec::LimitedReader;
/// use codas::codec::ReadsDecodable;
///
/// # fn example(encoded: &[u8]) -> Result<(), codas::codec::CodecError> {
/// // Custom limits:
/// let mut slice = encoded;
/// let data: u32 = LimitedReader::new(&mut slice)
///     .max_bytes(1024)
///     .max_depth(8)
///     .read_data()?;
///
/// // No effective limits (trusted data):
/// let mut slice = encoded;
/// let data: u32 = LimitedReader::unlimited(&mut slice)
///     .read_data()?;
/// # Ok(())
/// # }
/// ```
///
/// Limits are cumulative within the `LimitedReader`'s lifetime: every
/// sub-field's bytes and nesting depth count against the same instance.
pub struct LimitedReader<'a, R: Reads> {
    reader: &'a mut R,
    bytes_read: u64,
    max_bytes: u64,
    depth: u32,
    max_depth: u32,
}

impl<'a, R: Reads> LimitedReader<'a, R> {
    /// Creates a new `LimitedReader` with default limits
    /// ([`DEFAULT_MAX_BYTES`] and [`DEFAULT_MAX_DEPTH`]).
    pub fn new(reader: &'a mut R) -> Self {
        Self {
            reader,
            bytes_read: 0,
            max_bytes: DEFAULT_MAX_BYTES,
            depth: 0,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }

    /// Creates a new `LimitedReader` with no effective limits.
    pub fn unlimited(reader: &'a mut R) -> Self {
        Self {
            reader,
            bytes_read: 0,
            max_bytes: u64::MAX,
            depth: 0,
            max_depth: u32::MAX,
        }
    }

    /// Sets the maximum number of bytes this reader will read.
    pub fn max_bytes(mut self, max: u64) -> Self {
        self.max_bytes = max;
        self
    }

    /// Sets the maximum nesting depth this reader will allow.
    pub fn max_depth(mut self, max: u32) -> Self {
        self.max_depth = max;
        self
    }

    /// Returns the total number of bytes read so far.
    pub fn bytes_read(&self) -> u64 {
        self.bytes_read
    }
}

impl<R: Reads> ReadsDecodable for LimitedReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, CodecError> {
        let remaining = self.max_bytes.saturating_sub(self.bytes_read) as usize;
        if remaining == 0 && !buf.is_empty() {
            return Err(CodecError::ByteLimitExceeded);
        }
        let limit = buf.len().min(remaining);
        let n = self.reader.read(&mut buf[..limit])?;
        self.bytes_read += n as u64;
        Ok(n)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), CodecError> {
        let len = buf.len() as u64;
        if self.bytes_read + len > self.max_bytes {
            return Err(CodecError::ByteLimitExceeded);
        }
        self.reader.read_exact(buf)?;
        self.bytes_read += len;
        Ok(())
    }

    fn enter_scope(&mut self) -> Result<(), CodecError> {
        self.depth += 1;
        if self.depth > self.max_depth {
            return Err(CodecError::DepthLimitExceeded);
        }
        Ok(())
    }

    fn exit_scope(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}

/// Default maximum bytes a [`LimitedReader`] will read (64 MiB).
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Default maximum nesting depth a [`LimitedReader`] will allow.
pub const DEFAULT_MAX_DEPTH: u32 = 64;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{codec::tests::*, types::Text};

    #[test]
    fn decodes() -> Result<(), CodecError> {
        // Encode test data.
        let mut bytes = Vec::new();
        encode_test_data(&mut bytes);
        let mut bytes = bytes.as_slice();

        // Decode the header.
        let header: DataHeader = bytes.read_data()?;
        assert_eq!(1, header.count);
        assert_eq!(0, header.format.ordinal);
        assert_eq!(12, header.format.blob_size);
        assert_eq!(1, header.format.data_fields);

        // Decode blob fields.
        assert_eq!(TestData::default().num_a, bytes.read_data()?);
        assert_eq!(TestData::default().num_b, bytes.read_data()?);

        // Decode text.
        let mut text = Text::default();
        bytes.read_data_into(&mut text)?;
        assert_eq!(TestData::default().text, text);

        Ok(())
    }

    #[test]
    fn limited_reader_byte_limit() {
        use crate::codec::WritesEncodable;

        // Encode a Text value (header + bytes).
        let text = Text::from("hello, limited world!");
        let mut bytes = vec![];
        bytes.write_data(&text).unwrap();
        let total = bytes.len();

        // Decoding with a limit smaller than the payload fails.
        let mut slice = bytes.as_slice();
        let result = LimitedReader::new(&mut slice)
            .max_bytes(8) // only enough for the header
            .read_data::<Text>();
        assert!(
            matches!(result, Err(CodecError::ByteLimitExceeded)),
            "expected ByteLimitExceeded, got {result:?}"
        );

        // Decoding with exact limit succeeds.
        let mut slice = bytes.as_slice();
        let decoded = LimitedReader::new(&mut slice)
            .max_bytes(total as u64)
            .read_data::<Text>()
            .expect("should decode within exact limit");
        assert_eq!(text, decoded);
    }

    #[test]
    fn limited_reader_depth_limit() {
        use crate::codec::WritesEncodable;

        // Build a nested structure: Vec<Vec<u32>>.
        // Nesting: outer header → inner Vec header → u32 blobs.
        // That's 2 levels of structured data.
        let data: Vec<Vec<u32>> = vec![vec![1, 2], vec![3, 4]];
        let mut bytes = vec![];
        bytes.write_data(&data).unwrap();

        // max_depth=1 should fail (we need at least 2 levels).
        let mut slice = bytes.as_slice();
        let result = LimitedReader::new(&mut slice)
            .max_depth(1)
            .read_data::<Vec<Vec<u32>>>();
        assert!(
            matches!(result, Err(CodecError::DepthLimitExceeded)),
            "expected DepthLimitExceeded, got {result:?}"
        );

        // max_depth=2 should succeed.
        let mut slice = bytes.as_slice();
        let decoded = LimitedReader::new(&mut slice)
            .max_depth(2)
            .read_data::<Vec<Vec<u32>>>()
            .expect("should decode at depth 2");
        assert_eq!(data, decoded);
    }

    #[test]
    fn limited_reader_cumulative_bytes() {
        use crate::codec::WritesEncodable;

        // Encode a struct-like payload: two Text fields back-to-back
        // inside a data header, totaling well over 16 bytes.
        let text_a = Text::from("hello world!!");
        let text_b = Text::from("goodbye world!");
        let mut payload = vec![];
        payload.write_data(&text_a).unwrap();
        payload.write_data(&text_b).unwrap();
        let total = payload.len();

        // A limit that covers the first field but not both should fail
        // partway through the second field, proving bytes are cumulative.
        let first_field_size = {
            let mut tmp = vec![];
            tmp.write_data(&text_a).unwrap();
            tmp.len()
        };
        let tight_limit = first_field_size as u64 + 4; // enough for first, not second

        let mut slice = payload.as_slice();
        let mut limited = LimitedReader::new(&mut slice).max_bytes(tight_limit);
        let _a: Text = limited.read_data().unwrap(); // should succeed
        let result_b: Result<Text, _> = limited.read_data(); // should fail
        assert!(
            matches!(result_b, Err(CodecError::ByteLimitExceeded)),
            "expected ByteLimitExceeded on second field, got {result_b:?}"
        );

        // With enough room for both, it succeeds.
        let mut slice = payload.as_slice();
        let mut limited = LimitedReader::new(&mut slice).max_bytes(total as u64);
        let a: Text = limited.read_data().unwrap();
        let b: Text = limited.read_data().unwrap();
        assert_eq!(text_a, a);
        assert_eq!(text_b, b);
    }

    #[test]
    fn limited_reader_auto_wrap_succeeds() -> Result<(), CodecError> {
        use crate::codec::WritesEncodable;

        // Normal decode through the blanket impl (auto-wrapping)
        // should work for well-formed data under default limits.
        let data: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5]];
        let mut bytes = vec![];
        bytes.write_data(&data).unwrap();
        let decoded: Vec<Vec<u32>> = bytes.as_slice().read_data()?;
        assert_eq!(data, decoded);
        Ok(())
    }

    /// The byte budget must accumulate across all fields within
    /// a single decode — not reset per field. This test decodes
    /// a nested structure through a [`LimitedReader`] with a
    /// tight budget, proving that all sub-field bytes count
    /// against one shared limit.
    #[test]
    fn limited_reader_struct_byte_accumulation() {
        use crate::codec::WritesEncodable;

        let data: Vec<Vec<u32>> = vec![vec![1, 2], vec![3, 4]];
        let mut bytes = vec![];
        bytes.write_data(&data).unwrap();
        let total = bytes.len();

        // One byte short of the total should fail, proving
        // the budget is shared across all nested decode calls.
        let mut slice = bytes.as_slice();
        let result = LimitedReader::new(&mut slice)
            .max_bytes(total as u64 - 1)
            .read_data::<Vec<Vec<u32>>>();
        assert!(
            matches!(result, Err(CodecError::ByteLimitExceeded)),
            "expected ByteLimitExceeded with budget {}, got {result:?}",
            total - 1,
        );

        // Exact budget succeeds.
        let mut slice = bytes.as_slice();
        let decoded = LimitedReader::new(&mut slice)
            .max_bytes(total as u64)
            .read_data::<Vec<Vec<u32>>>()
            .expect("exact budget should succeed");
        assert_eq!(data, decoded);
    }

    #[test]
    fn splits_off_group_sequences() -> Result<(), CodecError> {
        // Pre encode a sequence of expected data.
        let mut expected = vec![];
        encode_test_data(&mut expected);

        // Encode two group sequences into `bytes`.
        let mut bytes = vec![];
        encode_test_data(&mut bytes);
        encode_test_data(&mut bytes);

        // Keep a slice of the full bytes, in
        // addition to the slice we'll traverse
        // during decoding.
        let original_bytes = bytes.as_slice();
        let mut bytes = bytes.as_slice();

        // Split off and decode the first sequence.
        let data_one_length = bytes.skip_data()?;
        let (data_one, original_bytes) = original_bytes.split_at(data_one_length);
        assert_eq!(expected, data_one);

        // Split off and decode the second sequence.
        let data_two_length = bytes.skip_data()?;
        let (data_two, _) = original_bytes.split_at(data_two_length);
        assert_eq!(expected, data_two);

        Ok(())
    }
}
