use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::codec::{
    CodecError, DataHeader, Decodable, Encodable, FieldReader, Format, ReadsDecodable,
    UnexpectedDataFormatSnafu, WritesEncodable,
};

impl<K, V> Encodable for BTreeMap<K, V>
where
    K: Encodable + Ord + Clone + 'static,
    V: Encodable + Clone + 'static,
{
    /// Maps are encoded as a sorted vector of keys
    /// followed by a sorted vector of corresponding
    /// values.
    const FORMAT: Format = Format::data(0)
        .with(Vec::<K>::FORMAT)
        .with(Vec::<V>::FORMAT);

    fn encode(&self, writer: &mut (impl WritesEncodable + ?Sized)) -> Result<(), CodecError> {
        writer.write_data(&self.keys().cloned().collect::<Vec<K>>())?;
        writer.write_data(&self.values().cloned().collect::<Vec<V>>())?;

        Ok(())
    }
}

impl<K, V> Decodable for BTreeMap<K, V>
where
    K: Default + Decodable + Ord + Clone + 'static,
    V: Default + Decodable + Clone + 'static,
{
    fn decode(
        &mut self,
        reader: &mut (impl ReadsDecodable + ?Sized),
        header: Option<DataHeader>,
    ) -> Result<(), CodecError> {
        let header = Self::ensure_header(header)?;
        let mut fields = FieldReader::new(reader, header);

        // Reset the map.
        self.clear();

        // Collect all keys and values.
        let mut keys: Vec<K> = Vec::new();
        fields.read(&mut keys)?;
        let mut values: Vec<V> = Vec::new();
        fields.read(&mut values)?;
        fields.finish()?;

        // Reject mismatched lengths to avoid silent data loss from zip().
        if keys.len() != values.len() {
            return UnexpectedDataFormatSnafu {
                expected: Self::FORMAT,
                actual: None,
            }
            .fail();
        }

        // Insert (key, value) pairs.
        for (key, value) in keys.into_iter().zip(values) {
            self.insert(key, value);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use crate::{
        codec::{ReadsDecodable, WritesEncodable},
        types::Text,
    };

    #[test]
    fn structured_to_structured() {
        let mut map = BTreeMap::new();
        map.insert(Text::from("a"), Text::from("c"));
        map.insert(Text::from("b"), Text::from("d"));

        let mut encoded = vec![];
        encoded.write_data(&map).expect("encoded");

        let decoded = encoded.as_slice().read_data().expect("decoded");

        assert_eq!(map, decoded);
    }

    #[test]
    fn structured_to_unstructured() {
        let mut map = BTreeMap::new();
        map.insert(Text::from("a"), 31u64);
        map.insert(Text::from("b"), 42u64);

        let mut encoded = vec![];
        encoded.write_data(&map).expect("encoded");

        let decoded = encoded.as_slice().read_data().expect("decoded");

        assert_eq!(map, decoded);
    }

    #[test]
    fn unstructured_to_structured() {
        let mut map = BTreeMap::new();
        map.insert(9001u64, Text::from("a"));
        map.insert(1337, Text::from("b"));

        let mut encoded = vec![];
        encoded.write_data(&map).expect("encoded");

        let decoded = encoded.as_slice().read_data().expect("decoded");

        assert_eq!(map, decoded);
    }

    #[test]
    fn unstructured_to_unstructured() {
        let mut map = BTreeMap::new();
        map.insert(42u64, 9001u64);
        map.insert(31u64, 1337u64);

        let mut encoded = vec![];
        encoded.write_data(&map).expect("encoded");

        let decoded = encoded.as_slice().read_data().expect("decoded");

        assert_eq!(map, decoded);
    }
}
