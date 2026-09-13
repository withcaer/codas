//! List data types (including `[u8]` and `Option`).

use alloc::vec::Vec;

use crate::codec::{
    self, CodecError, DataFormat, DataHeader, Decodable, Encodable, Format, ReadsDecodable,
    WritesEncodable,
};

impl Encodable for [u8] {
    /// Encoded as a [`Format::Sequence`] of [`u8`]s.
    const FORMAT: Format = u8::FORMAT.as_sequence();

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        writer.write_all(self)?;
        Ok(())
    }

    fn encode_header(
        &self,
        writer: &mut (impl WritesEncodable + ?Sized),
    ) -> Result<(), CodecError> {
        DataHeader {
            count: codec::try_count(self.len())?,
            format: DataFormat {
                blob_size: 1,
                data_fields: 0,
                ordinal: 0,
            },
        }
        .encode(writer)
    }
}

impl<T> Encodable for Vec<T>
where
    T: Encodable + 'static,
{
    /// Encoded as a [`Format::Sequence`] of `T`s.
    ///
    /// A `Vec<u8>` has the same encoding as a `[u8]`.
    const FORMAT: Format = T::FORMAT.as_sequence();

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        for item in self {
            writer.write_sequence_data(item)?;
        }

        Ok(())
    }

    fn encode_header(
        &self,
        writer: &mut (impl WritesEncodable + ?Sized),
    ) -> Result<(), CodecError> {
        DataHeader {
            count: codec::try_count(self.len())?,
            format: Self::FORMAT.as_data_format(),
        }
        .encode(writer)
    }
}

impl<T> Decodable for Vec<T>
where
    T: Decodable + Default + 'static,
{
    fn decode(
        &mut self,
        reader: &mut (impl ReadsDecodable + ?Sized),
        header: Option<DataHeader>,
    ) -> Result<(), CodecError> {
        let header = Self::ensure_header(header)?;

        // To mitigate repeat allocations, reserve
        // space for any elements in excess of this
        // vector's current capacity.
        let count = header.count as usize;
        if self.capacity() < count {
            self.reserve_exact(count - self.capacity());
        }

        // Decode all elements.
        for i in 0..count {
            let mut item = self.get_mut(i);
            if item.is_none() {
                self.push(T::default());
                item = Some(self.get_mut(i).expect("must exist"));
            }
            let item = item.unwrap();

            reader.read_next_data_into(header, item)?;
        }

        self.truncate(count);

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::{
        codec::{DataFormat, Encodable, Format, ReadsDecodable, WritesEncodable},
        types::Text,
    };

    #[test]
    fn codes_u8_slices() {
        assert_eq!(<[u8]>::FORMAT, <Vec<u8>>::FORMAT);
        assert_eq!(Format::Blob(1).as_sequence(), <Vec<u8>>::FORMAT);

        let value = &[8u8, 3, 7][..];
        let mut encoded = vec![];
        encoded.write_data(value).expect("encoded");
        let decoded: Vec<u8> = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded.as_slice());
    }

    #[test]
    fn codes_unstructured_vecs() {
        assert_eq!(Format::Blob(4).as_sequence(), <Vec<u32>>::FORMAT);

        let value = vec![7u32, 8, 9];
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        assert_eq!(8 + 3 * 4, encoded.len(), "scalars are packed");
        let decoded: Vec<u32> = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded);
    }

    #[test]
    fn codes_structured_vecs() {
        assert_eq!(
            Format::Sequence(DataFormat {
                blob_size: 0,
                data_fields: 1,
                ordinal: 0
            }),
            <Vec<Text>>::FORMAT
        );

        let value = vec![Text::from("Hello, world!")];
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        assert_eq!(8 + 8 + 13, encoded.len(), "text is prefixed");
        let decoded: Vec<Text> = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded);
    }

    #[test]
    fn empty_vecs_keep_their_format() {
        let value: Vec<Text> = vec![];
        let mut encoded = vec![];
        encoded.write_data(&value).expect("encoded");
        assert_eq!(
            [0, 0, 0, 0, 0, 0, 1, 0],
            encoded.as_slice(),
            "empty sequences carry their data's format"
        );
        let decoded: Vec<Text> = encoded.as_slice().read_data().expect("decoded");
        assert_eq!(value, decoded);

        // An all-zero header denotes _absence_, not an empty list.
        let zeroed = [0u8; 8];
        assert!(zeroed.as_slice().read_data::<Vec<Text>>().is_err());
    }
}
