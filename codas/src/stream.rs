//! Utilities for reading and writing streams of
//! binary data, with optional support for `std::io::Read`
//! and `std::io::Write` on platforms supporting them.
use snafu::Snafu;

/// A thing that reads from a stream of bytes.
pub trait Reads {
    /// Reads bytes into `buf`, returning the number
    /// of bytes read.
    ///
    /// No more than `buf.len()` bytes will be read.
    ///
    /// If an error occurs, the state of `buf` and
    /// the number of bytes read is undefined.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError>;

    /// Reads _exactly_ `buf.len()` bytes into `buf`.
    ///
    /// If an error occurs, the state of `buf` and
    /// the number of bytes read is undefined.
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), StreamError> {
        let mut read = 0;

        while read < buf.len() {
            read += self.read(&mut buf[read..])?;
        }

        Ok(())
    }
}

/// Implementation taken from
/// <https://doc.rust-lang.org/src/std/io/impls.rs.html#233>.
#[cfg(not(any(feature = "std", test)))]
impl Reads for &[u8] {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
        let amt = core::cmp::min(buf.len(), self.len());
        let (a, b) = self.split_at(amt);

        // First check if the amount of bytes we want to read is small:
        // `copy_from_slice` will generally expand to a call to `memcpy`, and
        // for a single byte the overhead is significant.
        if amt == 1 {
            buf[0] = a[0];
        } else {
            buf[..amt].copy_from_slice(a);
        }

        *self = b;
        Ok(amt)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), StreamError> {
        if buf.len() > self.len() {
            return Err(StreamError::Closed);
        }
        let (a, b) = self.split_at(buf.len());

        // First check if the amount of bytes we want to read is small:
        // `copy_from_slice` will generally expand to a call to `memcpy`, and
        // for a single byte the overhead is significant.
        if buf.len() == 1 {
            buf[0] = a[0];
        } else {
            buf.copy_from_slice(a);
        }

        *self = b;
        Ok(())
    }
}

#[cfg(not(any(feature = "std", test)))]
impl<R: Reads + ?Sized> Reads for &mut R {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
        (**self).read(buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), StreamError> {
        (**self).read_exact(buf)
    }
}

#[cfg(any(feature = "std", test))]
impl<T> Reads for T
where
    T: std::io::Read,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
        self.read(buf).map_err(|e| match e.kind() {
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe => StreamError::Closed,
            std::io::ErrorKind::UnexpectedEof => StreamError::Empty,
            _ => StreamError::Other {
                message: "Unexpected IO Error",
            },
        })
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), StreamError> {
        self.read_exact(buf).map_err(|e| match e.kind() {
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe => StreamError::Closed,
            std::io::ErrorKind::UnexpectedEof => StreamError::Empty,
            _ => StreamError::Other {
                message: "Unexpected IO Error",
            },
        })
    }
}

/// A thing that writes to a stream of bytes.
pub trait Writes {
    /// Writes bytes from `buf`, returning the number
    /// of bytes written.
    ///
    /// No more than `buf.len()` bytes will be written.
    ///
    /// If an error occurs, the number of bytes written
    /// is undefined.
    fn write(&mut self, buf: &[u8]) -> Result<usize, StreamError>;

    /// Writes _all_ bytes from `buf`.
    ///
    /// If an error occurs, the number of bytes written
    /// is undefined.
    fn write_all(&mut self, buf: &[u8]) -> Result<(), StreamError> {
        let mut written = 0;

        while written < buf.len() {
            written += self.write(&buf[written..])?;
        }

        Ok(())
    }
}

/// [`core::fmt::Write`] wrapper for any [`Writes`].
#[cfg_attr(
    not(any(
        feature = "langs-python",
        feature = "langs-duckdb",
        feature = "langs-sqlite",
        feature = "langs-typescript",
        feature = "langs-open-api",
        test
    )),
    allow(dead_code)
)]
pub(crate) struct FmtWriter<'w, W: Writes> {
    writes: &'w mut W,
}

impl<'w, W: Writes> From<&'w mut W> for FmtWriter<'w, W> {
    fn from(value: &'w mut W) -> Self {
        Self { writes: value }
    }
}

impl<W: Writes> core::fmt::Write for FmtWriter<'_, W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        match self.writes.write_all(s.as_bytes()) {
            Ok(_) => Ok(()),
            Err(_) => Err(core::fmt::Error),
        }
    }
}

#[cfg(not(any(feature = "std", test)))]
impl Writes for alloc::vec::Vec<u8> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, StreamError> {
        self.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<(), StreamError> {
        self.extend_from_slice(buf);
        Ok(())
    }
}

#[cfg(any(feature = "std", test))]
impl<T> Writes for T
where
    T: std::io::Write,
{
    fn write(&mut self, buf: &[u8]) -> Result<usize, StreamError> {
        let written = self.write(buf).map_err(|e| match e.kind() {
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof => StreamError::Closed,
            _ => StreamError::Other {
                message: "Unexpected IO Error",
            },
        })?;

        // If an implementor of std::io::Write returns
        // `0` for the number of written bytes, it is
        // likely the implementor is no longer accepting
        // writes.
        if written == 0 {
            Err(StreamError::Closed)
        } else {
            Ok(written)
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<(), StreamError> {
        self.write_all(buf).map_err(|e| match e.kind() {
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof => StreamError::Closed,
            _ => StreamError::Other {
                message: "Unexpected IO Error",
            },
        })
    }
}

/// Enumeration of errors that may occur while
/// reading and/or writing streams of data.
#[derive(Debug, Clone, PartialEq, Eq, Snafu)]
pub enum StreamError {
    /// The stream is empty and will not
    /// receive any more data.
    Empty,

    /// The stream is closed and will not
    /// receive or accept any more data.
    Closed,

    /// Uncategorized error.
    #[snafu(display("{message}"))]
    Other { message: &'static str },
}
