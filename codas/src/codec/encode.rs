//! Codec encoder implementations.
use crate::stream::Writes;

use super::{CodecError, DataHeader, Format};

/// A thing that encodes into
/// [`codec`](super)-compliant data.
pub trait Encodable {
    /// This thing's [`Format`].
    const FORMAT: Format;

    /// Encodes this thing's data into `writer`
    /// _without_ encoding a [`DataHeader`].
    ///
    /// In most cases, [`WritesEncodable::write_data`] should
    /// be used instead of calling this function directly.
    ///
    /// ```rust
    /// # use codas::types::Text;
    /// # use crate::codas::codec::{Encodable, Decodable, DataHeader};
    ///
    /// let data = Text::from("cupcakes!");
    ///
    /// // Encode data into a vector.
    /// let mut encoded = vec![];
    /// data.encode_header(&mut encoded).unwrap();
    /// data.encode(&mut encoded).unwrap();
    /// let mut encoded_slice = encoded.as_slice();
    ///
    /// // Decode the header.
    /// let mut decoded_header = DataHeader::default();
    /// decoded_header.decode(&mut encoded_slice, None).unwrap();
    ///
    /// // Decode the data.
    /// let mut decoded_data = Text::default();
    /// decoded_data.decode(&mut encoded_slice, Some(decoded_header)).unwrap();
    ///
    /// assert_eq!(data, decoded_data);
    /// ```
    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError>;

    /// Encodes this thing's data _header_ into `writer`.
    ///
    /// If `Self`'s [`Encodable::FORMAT`] is not
    /// [`structured`](`Format::is_structured`),
    /// this function should be a no-op.
    #[inline(always)]
    fn encode_header(
        &self,
        writer: &mut (impl WritesEncodable + ?Sized),
    ) -> Result<(), CodecError> {
        match Self::FORMAT {
            Format::Blob(_) => Ok(()),
            Format::Data(format) => DataHeader { count: 1, format }.encode(writer),
            Format::Sequence(_) => {
                unimplemented!("sequence formats must manually implement `encode_header`")
            }
            Format::Fluid => {
                unimplemented!("fluid formats must manually implement `encode_header`")
            }
        }
    }
}

/// A thing that [`Writes`] [`Encodable`] data.
///
/// ```rust
/// # use codas::types::Text;
/// # use crate::codas::codec::{WritesEncodable, ReadsDecodable};
///
/// let data = Text::from("cupcakes!");
///
/// // Encode data into a vector.
/// let mut encoded = vec![];
/// encoded.write_data(&data).unwrap();
///
/// // Decode the data.
/// let decoded_data: Text = encoded.as_slice().read_data().unwrap();
///
/// assert_eq!(data, decoded_data);
/// ```
///
/// This trait is automatically implemented for
/// any type that [`Writes`].
pub trait WritesEncodable: Writes {
    /// Encodes and writes a sequence of data from `data`.
    ///
    /// This function will attempt to encode and write a
    /// [`DataHeader`] if the `data`'s [`Format::is_structured`].
    fn write_data<T: Encodable + ?Sized>(&mut self, data: &T) -> Result<(), CodecError> {
        data.encode_header(self)?;
        data.encode(self)?;

        Ok(())
    }

    /// Encodes and writes a single `data` of a
    /// sequence of `T`, per [`Format::as_sequence`].
    fn write_sequence_data<T: Encodable + ?Sized>(&mut self, data: &T) -> Result<(), CodecError> {
        if T::FORMAT.is_packed() {
            data.encode(self)
        } else {
            self.write_data(data)
        }
    }
}

impl<T: Writes + ?Sized> WritesEncodable for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::tests::*;

    #[test]
    fn encodes() -> Result<(), CodecError> {
        let mut bytes = Vec::new();

        // Allocate test data.
        let test_data = TestData::default();

        // Encode header.
        bytes.write_data(&DataHeader {
            count: 1,
            format: TestData::FORMAT.as_data_format(),
        })?;

        // Encode blob fields.
        bytes.write_data(&test_data.num_a)?;
        bytes.write_data(&test_data.num_b)?;

        // Encode data fields.
        bytes.write_data(&test_data.text)?;

        // Check encoding.
        let mut expected = Vec::new();
        encode_test_data(&mut expected);
        assert_eq!(expected, bytes);

        Ok(())
    }
}
