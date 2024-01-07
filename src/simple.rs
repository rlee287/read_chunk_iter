use std::io::Error as IOError;
use std::io::Result as IOResult;
use std::io::{ErrorKind, Read, Seek};

/// An iterator adapter for readers that yields chunks of bytes in a `Box<[u8]>`.
///
#[derive(Debug)]
pub struct ChunkedReaderIter<R> {
    reader: R,
    chunk_size: usize,
    buf_size: usize,
    buf: Vec<u8>,
    undrained_byte_count: usize,
    io_error_stash: Option<IOError>,
}
impl<R> ChunkedReaderIter<R> {
    /// Instantiates a new [`ChunkedReaderIter`] that tries to read up to `buf_size` bytes at a time and that yields `chunk_size` bytes as an iterator until reaching EOF.
    /// For readers that implement `Seek`, [`Self::new_with_rewind`] rewinds the given reader.
    ///
    /// # Panics
    /// Panics if `buf_size` is smaller than `chunk_size` or if either are 0.
    pub fn new(reader: R, chunk_size: usize, buf_size: usize) -> Self {
        assert!(chunk_size > 0);
        assert!(buf_size > 0);
        assert!(buf_size >= chunk_size);
        Self {
            reader,
            chunk_size,
            buf_size,
            buf: Vec::with_capacity(buf_size),
            undrained_byte_count: 0,
            io_error_stash: None,
        }
    }

    /// Returns the wrapped reader. Warning: buffered read data will be lost, which can occur if `buf_size > chunk_size`.
    #[inline]
    pub fn into_inner(self) -> R {
        self.reader
    }
    /// Returns the chunk size which is yielded by the iterator.
    #[inline]
    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }
    /// Returns the size of the buffer used to read from the underlying reader.
    #[inline]
    pub fn buf_size(&self) -> usize {
        self.buf_size
    }
    /// Returns a slice of the internal buffer used to buffer reads. The slice only contains valid buffered data, so it will be smaller than the value returned by [`Self::buf_size`].
    #[inline]
    pub fn buf(&self) -> &[u8] {
        self.buf.as_ref()
    }
}
impl<R: Seek> ChunkedReaderIter<R> {
    /// Constructs a new [`ChunkedReaderIter`] that rewinds the reader to ensure that all data is yielded by the iterator.
    /// See [`ChunkedReaderIter::new`] for descriptions of the other parameters.
    pub fn new_with_rewind(mut reader: R, chunk_size: usize, buf_size: usize) -> Self {
        reader.rewind().unwrap();
        Self::new(reader, chunk_size, buf_size)
    }
}

impl<R: Read> Iterator for ChunkedReaderIter<R> {
    type Item = IOResult<Box<[u8]>>;

    /// Yields `self.chunk_size` bytes at a time until reaching EOF, after which it yields the remaining bytes before returning `None`.
    /// All bytes successfully read are eventually returned: if reads into the buffer result in an error, the previously read data is yielded first, and then the error is passed up.
    ///
    /// Note: If reading from a readable object that is being concurrently modified (e.g. a file that is being appended to by another process),
    /// EOF may be hit more than once, resulting in more chunks after a chunk smaller than `self.chunk_size` or more chunks after yielding `None`.
    /// (This is also a concern with the base [`Read`] trait, which may return more data even after returning `Ok(0)`).
    fn next(&mut self) -> Option<Self::Item> {
        if self.io_error_stash.is_some() {
            let err_obj = self.io_error_stash.take().unwrap();
            return Some(Err(err_obj));
        }
        let mut read_offset = self.buf.len();
        assert!(self.undrained_byte_count <= read_offset);
        // Temporarily resize Vec to try to fill it
        // Due to initialization with `with_capacity` we do not need to reallocate
        self.buf.resize(self.buf_size, 0x00);
        // Try to fill entire buf, but we're good if we have a whole chunk
        while read_offset < self.chunk_size {
            match self.reader.read(&mut self.buf[read_offset..]) {
                Ok(0) => {
                    break;
                }
                Ok(n) => {
                    read_offset += n;
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => { /* continue */ }
                Err(e) => {
                    // Shrink Vec back to how much was actually read
                    self.buf.truncate(read_offset);
                    // Yield currently read data before yielding Err
                    if read_offset > 0 {
                        assert!(self.io_error_stash.is_none());
                        self.io_error_stash = Some(e);
                        let boxed_data: Box<[u8]> =
                            self.buf.drain(self.undrained_byte_count..).collect();
                        self.undrained_byte_count = 0;
                        return Some(Ok(boxed_data));
                    }
                    return Some(Err(e));
                }
            }
        }
        // How much data in the buffer has not been yielded yet?
        let unyielded_count = read_offset - self.undrained_byte_count;
        if unyielded_count == 0 {
            // We hit EOF and ran out of buffer contents
            self.buf.clear();
            self.undrained_byte_count = 0;
            return None;
        }
        // Shrink Vec back to how much was actually read
        self.buf.truncate(read_offset);

        // We keep stale, already-yielded data and don't memmove every time
        // In order to reduce the performance penalty of such memory accesses
        if self.chunk_size > unyielded_count {
            // Yield the remaining data at EOF
            let boxed_data: Box<[u8]> = self.buf.drain(self.undrained_byte_count..).collect();
            self.buf.clear();
            self.undrained_byte_count = 0;
            Some(Ok(boxed_data))
        } else {
            let ret_buf = self.buf
                [self.undrained_byte_count..self.undrained_byte_count + self.chunk_size]
                .iter()
                .copied()
                .collect();
            self.undrained_byte_count += self.chunk_size;
            assert!(read_offset >= self.undrained_byte_count);
            if read_offset - self.undrained_byte_count < self.chunk_size {
                self.buf.drain(..self.undrained_byte_count);
                self.undrained_byte_count = 0;
            }
            Some(Ok(ret_buf))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;

    use crate::dev_helpers::{FunnyRead, IceCubeRead, TruncatedRead};

    #[test]
    fn chunked_read_iter_funnyread() {
        let funny_read = FunnyRead::default();
        let mut funny_read_iter = ChunkedReaderIter::new(funny_read, 4, 5);
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            &[0, 1, 2, 3]
        );
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            &[4, 5, 6, 7]
        );
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            &[8, 9, 10, 11]
        );
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            &[12, 13, 14, 15]
        );
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            &[16, 17, 18, 19]
        );
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            &[20, 21, 22, 23]
        );
    }
    #[test]
    fn chunked_read_iter_icecuberead() {
        let funny_read = IceCubeRead::default();
        let mut funny_read_iter = ChunkedReaderIter::new(funny_read, 2, 5);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[9, 99]);
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            &[0x99, 9]
        );
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            &[99, 0x99]
        );
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap_err().kind(),
            ErrorKind::Other
        );
        assert!(funny_read_iter.next().is_none());
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[9, 99]);
    }
    #[test]
    fn chunked_read_iter_truncatedread() {
        let funny_read = TruncatedRead::default();
        let mut funny_read_iter = ChunkedReaderIter::new(funny_read, 3, 3);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), b"rei");
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), b"mu");
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap_err().kind(),
            ErrorKind::Other
        );
        assert!(funny_read_iter.next().is_none());
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), b"rei");
    }
    #[test]
    fn chunked_read_iter_truncatedread_large() {
        let funny_read = TruncatedRead::default();
        let mut funny_read_iter = ChunkedReaderIter::new(funny_read, 11, 22);
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            b"reimureimu"
        );
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap_err().kind(),
            ErrorKind::Other
        );
        assert!(funny_read_iter.next().is_none());
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            b"reimureimu"
        );
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap_err().kind(),
            ErrorKind::Other
        );
        assert!(funny_read_iter.next().is_none());
        assert_eq!(
            funny_read_iter.next().unwrap().unwrap().as_ref(),
            b"reimureimu"
        );
    }

    #[test]
    fn chunked_read_iter_cursor_large() {
        let data_buf = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ChunkedReaderIter::new(data_cursor, 4, 8);
        assert_eq!(
            data_chunk_iter.next().unwrap().unwrap().as_ref(),
            &[1, 2, 3, 4]
        );
        assert_eq!(
            data_chunk_iter.next().unwrap().unwrap().as_ref(),
            &[5, 6, 7, 8]
        );
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[9]);
        assert!(data_chunk_iter.next().is_none());
    }
    #[test]
    fn chunked_read_iter_cursor_while() {
        let data_buf = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        let data_cursor = Cursor::new(data_buf);

        let data_chunks: Vec<_> = ChunkedReaderIter::new(data_cursor, 4, 8).collect();
        let data_chunks_as_slice: Vec<&[u8]> = data_chunks
            .iter()
            .map(|r| r.as_ref().unwrap().as_ref())
            .collect();
        let expected_data_chunks: &[&[u8]] = &[&[1, 2, 3, 4], &[5, 6, 7, 8], &[9]];
        assert_eq!(data_chunks_as_slice.as_slice(), expected_data_chunks);
    }
    #[test]
    fn chunked_read_iter_cursor_large_buf_eq_chunk() {
        let data_buf = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ChunkedReaderIter::new(data_cursor, 4, 4);
        assert_eq!(
            data_chunk_iter.next().unwrap().unwrap().as_ref(),
            &[1, 2, 3, 4]
        );
        assert_eq!(
            data_chunk_iter.next().unwrap().unwrap().as_ref(),
            &[5, 6, 7, 8]
        );
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[9]);
        assert!(data_chunk_iter.next().is_none());
    }
    #[test]
    fn chunked_read_iter_cursor_smol() {
        let data_buf = [1, 2, 3];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ChunkedReaderIter::new(data_cursor, 4, 4);
        assert_eq!(
            data_chunk_iter.next().unwrap().unwrap().as_ref(),
            &[1, 2, 3]
        );
        assert!(data_chunk_iter.next().is_none());
    }
}
