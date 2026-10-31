//! ## Unstable
//!
//! Lossless byte stream compression based on
//! [Cap'n Proto's](https://capnproto.org/encoding.html#packing)
//! packing algorithm.
//!
//! [`PackedWriter`]s read a raw stream of bytes as a sequence
//! of `8`-byte words, and write each word as a group of
//! bytes to the packed stream. [`PackedReader`]s reverse this process.
//!
//! Each target group of packed bytes is comprised of one
//! "tag" byte followed by the source word's non-zero bytes.
//!
//! The `8` bits of the tag correspond to the `8` bytes of the
//! source word, least-significant bit first, with a `1` bit
//! for each non-zero byte.
//!
//! For example, the source word `08 00 00 00 03 00 02 00`
//! packs into a `4`-byte target group of `51 08 03 02`.
//!
//! Tags of `0x00` or `0xFF` append one additional "count"
//! byte to the end of their target group:
//!
//! - `0x00` tags an all-zero source word. The trailing count
//!   byte indicates how many additional all-zero source words
//!   the group represents. For example, a target group of `00 03`
//!   unpacks to `4` all-zero source words.
//!
//! - `0xFF` tags an all-non-zero source word. The trailing count
//!   byte indicates how many additional source words were copied to
//!   the target stream verbatim, with no tags, stopping after `255`
//!   source words are copied verbatim in this way, _or_ stopping before
//!   the first source word containing `2` or more zero-value bytes.
//!
//! The source stream is padded to a whole word before packing;
//! up to `7` additional trailing zero-value bytes may be read
//! back during unpacking.
use alloc::vec::Vec;

use super::{Reads, StreamError, Writes};

/// A [`Writes`] wrapper which packs everything
/// written to it into the wrapped stream.
///
/// [`finish`](Self::finish) must be called after
/// the last write to pack the final word.
pub struct PackedWriter<'w, W: Writes> {
    writes: &'w mut W,
    /// The next word, filled to `filled` bytes.
    word: [u8; 8],
    filled: usize,
    run: Run,
}

/// Words whose count is still being tallied.
enum Run {
    None,
    /// The all-zero words following an all-zero word.
    Zeros(u8),
    /// An all-non-zero word, and the words copied verbatim after it.
    Verbatim(Vec<u8>),
}

impl<'w, W: Writes> PackedWriter<'w, W> {
    /// Returns a writer packing into `writes`.
    pub fn new(writes: &'w mut W) -> Self {
        Self {
            writes,
            word: [0; 8],
            filled: 0,
            run: Run::None,
        }
    }

    /// Packs the final word, padded with zeros.
    pub fn finish(mut self) -> Result<(), StreamError> {
        if self.filled > 0 {
            self.word[self.filled..].fill(0);
            self.pack_word()?;
        }
        self.write_run()
    }

    /// Packs the filled word, or adds it to a run.
    fn pack_word(&mut self) -> Result<(), StreamError> {
        let word = self.word;
        self.filled = 0;
        let zeros = word.iter().filter(|byte| **byte == 0).count();

        match &mut self.run {
            Run::Zeros(count) if zeros == 8 && *count < u8::MAX => {
                *count += 1;
                return Ok(());
            }
            Run::Verbatim(words) if zeros < 2 && words.len() < 8 * (1 + u8::MAX as usize) => {
                words.extend_from_slice(&word);
                return Ok(());
            }
            _ => self.write_run()?,
        }

        if zeros == 8 {
            self.run = Run::Zeros(0);
        } else if zeros == 0 {
            self.run = Run::Verbatim(word.to_vec());
        } else {
            let mut packed = [0u8; 9];
            let mut len = 1;
            for (i, byte) in word.iter().enumerate() {
                if *byte != 0 {
                    packed[0] |= 1 << i;
                    packed[len] = *byte;
                    len += 1;
                }
            }
            self.writes.write_all(&packed[..len])?;
        }

        Ok(())
    }

    /// Writes the tallied run, if any.
    fn write_run(&mut self) -> Result<(), StreamError> {
        match core::mem::replace(&mut self.run, Run::None) {
            Run::None => Ok(()),
            Run::Zeros(count) => self.writes.write_all(&[0x00, count]),
            Run::Verbatim(words) => {
                self.writes.write_all(&[0xFF])?;
                self.writes.write_all(&words[..8])?;
                self.writes.write_all(&[(words.len() / 8 - 1) as u8])?;
                self.writes.write_all(&words[8..])
            }
        }
    }
}

impl<W: Writes> Writes for PackedWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, StreamError> {
        for byte in buf {
            self.word[self.filled] = *byte;
            self.filled += 1;
            if self.filled == 8 {
                self.pack_word()?;
            }
        }

        Ok(buf.len())
    }
}

/// A [`Reads`] wrapper which unpacks everything
/// read from the wrapped stream.
pub struct PackedReader<'r, R: Reads> {
    reads: &'r mut R,
    /// The current word, read out to `read` bytes.
    word: [u8; 8],
    read: usize,
    /// The all-zero words remaining in a run.
    zeros: u8,
    /// The verbatim words remaining in a run.
    verbatim: u8,
}

impl<'r, R: Reads> PackedReader<'r, R> {
    /// Returns a reader unpacking from `reads`.
    pub fn new(reads: &'r mut R) -> Self {
        Self {
            reads,
            word: [0; 8],
            read: 8,
            zeros: 0,
            verbatim: 0,
        }
    }

    /// Unpacks the next word, returning `false`
    /// if the stream has no more words.
    fn unpack_word(&mut self) -> Result<bool, StreamError> {
        self.read = 0;

        if self.zeros > 0 {
            self.zeros -= 1;
            self.word = [0; 8];
            return Ok(true);
        } else if self.verbatim > 0 {
            self.verbatim -= 1;
            self.reads.read_exact(&mut self.word)?;
            return Ok(true);
        }

        let mut tag = [0u8];
        if self.reads.read(&mut tag)? == 0 {
            self.read = 8;
            return Ok(false);
        }

        match tag[0] {
            0x00 => {
                self.word = [0; 8];
                self.zeros = self.read_byte()?;
            }
            0xFF => {
                self.reads.read_exact(&mut self.word)?;
                self.verbatim = self.read_byte()?;
            }
            tag => {
                let mut bytes = [0u8; 8];
                self.reads
                    .read_exact(&mut bytes[..tag.count_ones() as usize])?;
                let mut bytes = bytes.iter();
                for (i, byte) in self.word.iter_mut().enumerate() {
                    *byte = match tag & (1 << i) {
                        0 => 0,
                        _ => *bytes.next().unwrap_or(&0),
                    };
                }
            }
        }

        Ok(true)
    }

    fn read_byte(&mut self) -> Result<u8, StreamError> {
        let mut byte = [0u8];
        self.reads.read_exact(&mut byte)?;
        Ok(byte[0])
    }
}

impl<R: Reads> Reads for PackedReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
        if self.read == 8 && !self.unpack_word()? {
            return Ok(0);
        }

        let len = buf.len().min(8 - self.read);
        buf[..len].copy_from_slice(&self.word[self.read..self.read + len]);
        self.read += len;
        Ok(len)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), StreamError> {
        let mut read = 0;

        while read < buf.len() {
            match self.read(&mut buf[read..])? {
                0 => return Err(StreamError::Empty),
                len => read += len,
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::codec::{ReadsDecodable, WritesEncodable};

    use super::*;

    fn pack(bytes: &[u8]) -> Vec<u8> {
        let mut packed = Vec::new();
        let mut writer = PackedWriter::new(&mut packed);
        writer.write_all(bytes).expect("packed");
        writer.finish().expect("finished");
        packed
    }

    fn unpack(mut packed: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut reader = PackedReader::new(&mut packed);
        let mut buf = [0u8; 3];
        loop {
            match Reads::read(&mut reader, &mut buf).expect("unpacked") {
                0 => return bytes,
                len => bytes.extend_from_slice(&buf[..len]),
            }
        }
    }

    #[test]
    fn test_packing_examples() {
        // Examples from https://capnproto.org/encoding.html#packing.
        let examples: [(&[u8], &[u8]); 3] = [
            (
                &[
                    0x08, 0x00, 0x00, 0x00, 0x03, 0x00, 0x02, 0x00, 0x19, 0x00, 0x00, 0x00, 0xAA,
                    0x01, 0x00, 0x00,
                ],
                &[0x51, 0x08, 0x03, 0x02, 0x31, 0x19, 0xAA, 0x01],
            ),
            (&[0x00; 32], &[0x00, 0x03]),
            (
                &[0x8A; 32],
                &[&[0xFF], &[0x8A; 8][..], &[0x03], &[0x8A; 24][..]].concat(),
            ),
        ];

        for (unpacked, packed) in examples {
            assert_eq!(packed, pack(unpacked));
            assert_eq!(unpacked, unpack(packed));
        }
    }

    #[test]
    fn test_packing_runs() {
        // Verbatim runs end at a word with two zero bytes.
        let mut bytes = vec![0x8A; 16];
        bytes.extend_from_slice(&[0x01, 0x00, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
        bytes.extend_from_slice(&[0x01, 0x00, 0x02, 0x00, 0x04, 0x05, 0x06, 0x07]);
        let packed = pack(&bytes);
        assert_eq!(0xFF, packed[0]);
        assert_eq!(2, packed[9], "two verbatim words follow");
        assert_eq!(0xF5, packed[26], "then a tagged word");
        assert_eq!(bytes, unpack(&packed));

        // Runs longer than a count byte split.
        let bytes = vec![0x00; 8 * 300];
        assert_eq!(vec![0x00, 0xFF, 0x00, 0x2B], pack(&bytes));
        assert_eq!(bytes, unpack(&pack(&bytes)));
        let bytes = vec![0x8A; 8 * 300];
        let packed = pack(&bytes);
        assert_eq!(8 * 300 + 4, packed.len());
        assert_eq!(bytes, unpack(&packed));

        // Partial final words are padded with zeros.
        let bytes = [0x01, 0x02, 0x03];
        assert_eq!(vec![0x07, 0x01, 0x02, 0x03], pack(&bytes));
        assert_eq!([0x01, 0x02, 0x03, 0, 0, 0, 0, 0], unpack(&pack(&bytes))[..]);
        assert!(pack(&[]).is_empty());
        assert!(unpack(&[]).is_empty());
    }

    #[test]
    fn test_packed_codec() {
        let value: Vec<u64> = vec![1, 2, 3, 4];
        let mut packed = Vec::new();
        let mut writer = PackedWriter::new(&mut packed);
        writer.write_data(&value).expect("encoded");
        writer.finish().expect("finished");
        assert!(packed.len() < 8 + 4 * 8);

        let mut packed = packed.as_slice();
        let decoded: Vec<u64> = PackedReader::new(&mut packed).read_data().expect("decoded");
        assert_eq!(value, decoded);
    }
}
