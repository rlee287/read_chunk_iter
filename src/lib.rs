use std::io::{Read, Seek, SeekFrom, ErrorKind};
use std::io::Result as IOResult;

/// An iterator adapter for readers that yields chunks of bytes in a `Box<[u8]>`.
/// 
#[derive(Debug, Clone, Hash)]
pub struct ChunkedReaderIter<R> {
    reader: R,
    chunk_size: usize,
    buf_size: usize,
    buf: Vec<u8>
}
impl<R> ChunkedReaderIter<R>
{
    /// Instantiates a new [`ChunkedReaderIter`] that tries to read up to `buf_size` bytes at a time and that yields `chunk_size` bytes as an iterator until reaching EOF.
    /// For readers that implement `Seek`, [`ChunkedReaderIter::new_with_rewind`] rewinds the given reader.
    pub fn new(reader: R, chunk_size: usize, buf_size: usize) -> Self {
        assert!(chunk_size > 0);
        assert!(buf_size > 0);
        assert!(buf_size >= chunk_size);
        Self { reader, chunk_size, buf_size, buf: Vec::with_capacity(buf_size) }
    }

    /// Returns the wrapped reader. Warning: buffered read data will be lost, which can occur if `buf_size > chunk_size`.
    pub fn into_inner(self) -> R {
        self.reader
    }
    /// Returns the chunk size which is yielded by the iterator.
    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }
    /// Returns the size of the buffer used to read from the underlying reader.
    pub fn buf_size(&self) -> usize {
        self.buf_size
    }
    /// Returns a slice of the internal buffer used to buffer reads.
    pub fn buf(&self) -> &[u8] {
        self.buf.as_ref()
    }
}
impl<R: Seek> ChunkedReaderIter<R> {
    /// Constructs a new [`ChunkedReaderIter`] that rewinds the reader to ensure that all data is yielded by the iterator.
    /// See [`ChunkedReaderIter::new`] for descriptions of the other parameters.
    pub fn new_with_rewind(mut reader: R, chunk_size: usize, buf_size: usize) -> Self {
        reader.seek(SeekFrom::Start(0)).unwrap();
        Self::new(reader, chunk_size, buf_size)
    }
}

impl<R: Read> Iterator for ChunkedReaderIter<R> {
    type Item = IOResult<Box<[u8]>>;

    /// Yields `self.chunk_size` bytes at a time until reaching EOF, after which it yields the remaining bytes before returning `None`.
    /// All bytes successfully read are eventually returned: if reads into the buffer result in an error, it is passed up, but the successfully read data will be returned the next time the buffer is successfully filled.
    fn next(&mut self) -> Option<Self::Item> {
        let mut read_offset = self.buf.len();
        self.buf.resize(self.buf_size, 0x00);
        while read_offset < self.chunk_size {
            match self.reader.read(&mut self.buf[read_offset..]) {
                Ok(0) => { break; }
                Ok(n) => { read_offset += n; },
                Err(e) if e.kind() == ErrorKind::Interrupted => { /* continue */ }
                Err(e) => { return Some(Err(e)) },
            }
        }
        if read_offset == 0 {
            // We hit EOF and ran out of buffer contents
            return None;
        }
        self.buf.resize(read_offset, 0x00);
        if self.chunk_size > self.buf.len() {
            let boxed_data = self.buf.clone().into_boxed_slice();
            self.buf.clear();
            Some(Ok(boxed_data))
        } else {
            Some(Ok(self.buf.drain(..self.chunk_size).collect()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;

    #[derive(Debug, Default)]
    struct FunnyRead {
        counter: usize
    }
    impl Read for FunnyRead {
        fn read(&mut self, buf: &mut [u8]) -> IOResult<usize> {
            let mut actual_count = 0;
            for byte in buf.into_iter().take(3) {
                actual_count += 1;
                *byte = (self.counter%256).try_into().unwrap();
                self.counter += 1;
            }
            Ok(actual_count)
        }
    }

    #[test]
    fn chunked_read_iter_funnyread() {
        let funny_read = FunnyRead::default();
        let mut funny_read_iter = ChunkedReaderIter::new(funny_read, 4, 5);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[0,1,2,3]);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[4,5,6,7]);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[8,9,10,11]);
    }

    #[test]
    fn chunked_read_iter_cursor_large() {
        let data_buf = [1,2,3,4,5,6,7,8,9];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ChunkedReaderIter::new(data_cursor, 4, 8);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[1,2,3,4]);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[5,6,7,8]);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[9]);
        assert!(data_chunk_iter.next().is_none());
    }
    #[test]
    fn chunked_read_iter_cursor_large_buf_eq_chunk() {
        let data_buf = [1,2,3,4,5,6,7,8,9];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ChunkedReaderIter::new(data_cursor, 4, 4);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[1,2,3,4]);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[5,6,7,8]);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[9]);
        assert!(data_chunk_iter.next().is_none());
    }
    #[test]
    fn chunked_read_iter_cursor_smol() {
        let data_buf = [1,2,3];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ChunkedReaderIter::new(data_cursor, 4, 4);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[1,2,3]);
        assert!(data_chunk_iter.next().is_none());
    }
}
