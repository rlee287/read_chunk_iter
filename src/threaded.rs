use std::io::{Read, Seek, SeekFrom, ErrorKind};
use std::io::Result as IOResult;

use std::thread::JoinHandle;
use std::thread::spawn as thread_spawn;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};

use atomic_wait::{wait, wake_all};
use std::ops::Deref;

/// An iterator adapter for readers that yields chunks of bytes in a `Box<[u8]>` and performs reads asynchronously via a thread.
/// 
#[derive(Debug)]
pub struct ThreadedChunkedReaderIter<R: Send> {
    // Thread yields back the reader so we can still do into_inner
    // This is an Option<> so that we can take() from it in destructor
    reader_thread_handle: Option<JoinHandle<R>>,
    chunk_size: usize,
    buf_count: usize,
    // This is an Option<> so that we can take() from it in destructor
    channel_receiver: Option<Receiver<IOResult<Box<[u8]>>>>,
    stop_flag: Arc<AtomicBool>,
    // Semantically this is a bool but atomic_wait only supports U32
    unpause_flag: Arc<AtomicU32>
}
impl<R: Send> ThreadedChunkedReaderIter<R>
{
    /// Returns the wrapped reader. Warning: buffered read data will be lost, which can occur if `buf_size > chunk_size`.
    pub fn into_inner(mut self) -> R {
        self.stop_flag.store(true, Ordering::Release);
        // Unpause and wake the thread if necessary
        if self.unpause_flag.swap(1, Ordering::AcqRel) == 0 {
            wake_all(self.unpause_flag.deref());
        }
        // Take and drop the receiver so that thread sends error
        drop(self.channel_receiver.take());
        self.reader_thread_handle.take().unwrap().join().unwrap()
    }
    /// Returns the chunk size which is yielded by the iterator.
    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }
    /// Returns the size of the buffer used to read from the underlying reader.
    pub fn buf_size(&self) -> usize {
        match self.buf_count {
            0 => self.chunk_size,
            len => len*self.chunk_size
        }
    }
    // Cannot provide buf view due to multithreading
}
impl<R: Read+Send+'static> ThreadedChunkedReaderIter<R>
{
    /// Instantiates a new [`ThreadedChunkedReaderIter`] that tries to read up to `chunk_size*buf_count` bytes at a time and that yields `chunk_size` bytes as an iterator until reaching EOF.
    /// For readers that implement `Seek`, [`Self::new_with_rewind`] rewinds the given reader.
    /// 
    /// # Panics
    /// Panics if `chunk_size` is 0.
    pub fn new(mut reader: R, chunk_size: usize, buf_count: usize) -> Self {
        assert!(chunk_size > 0);

        let stop_flag = Arc::new(AtomicBool::new(false));
        let unpause_flag = Arc::new(AtomicU32::new(1));
        let (tx, rx) = sync_channel(buf_count);

        let reader_thread_handle = {
            let stop_flag = stop_flag.clone();
            let unpause_flag = unpause_flag.clone();

            thread_spawn(move || {
                let buf_size = match buf_count {
                    0 => chunk_size,
                    len => len*chunk_size
                };
                let mut buf = Vec::with_capacity(buf_size);
                // Code modified from ChunkedReaderIter::next()
                'read_loop: while !stop_flag.load(Ordering::Acquire) {
                    // unpause_flag only gets set if we hit EOF earlier
                    // If we did, we don't want to retry until newly requested
                    // Otherwise we get stuck in a spinloop of reading nothing
                    wait(&unpause_flag, 0);

                    let mut read_offset = buf.len();
                    // Temporarily resize Vec to try to fill it
                    // Due to initialization with `with_capacity` we do not need to reallocate
                    buf.resize(buf_size, 0x00);
                    // Try to fill entire buf, but we're good if we have a whole chunk
                    while read_offset < chunk_size {
                        match reader.read(&mut buf[read_offset..]) {
                            Ok(0) => { break; }
                            Ok(n) => { read_offset += n; },
                            Err(e) if e.kind() == ErrorKind::Interrupted => { /* continue */ }
                            Err(e) => {
                                // Shrink Vec back to how much was actually read
                                buf.truncate(read_offset);
                                // On send error we no longer have anyone listening
                                match tx.send(Err(e)) {
                                    Ok(_) => { continue 'read_loop; },
                                    Err(_) => { break 'read_loop; }
                                }
                             },
                        }
                    }
                    // TODO: break statements for exiting after here are suspect
                    // The naive solution would be using continue's, but then
                    // we end up in a spinloop on EOF while waiting for data or
                    // the exit signal. TODO is coming up with a better idea
                    if read_offset == 0 {
                        // We hit EOF and ran out of buffer contents
                        buf.clear();
                        // Store a false value so that we wait until another
                        // read is requested.
                        // Delay between store and wait doesn't matter.
                        // If it gets set back to 1 in the meantime then
                        // we want to continue anyways
                        unpause_flag.store(0, Ordering::Release);
                        match tx.send(Ok(Box::default())) {
                            Ok(_) => { continue 'read_loop; },
                            Err(_) => { break 'read_loop; }
                        }
                    }
                    // Shrink Vec back to how much was actually read
                    buf.truncate(read_offset);

                    if chunk_size > buf.len() {
                        // Yield the remaining data at EOF
                        let boxed_data: Box<[u8]> = buf.drain(..).collect();
                        match tx.send(Ok(boxed_data)) {
                            Ok(_) => { continue 'read_loop; },
                            Err(_) => { break 'read_loop; }
                        }
                        // We continue so that next iteration hits EOF again
                        // Then we trigger the wait logic
                    } else {
                        if tx.send(Ok(buf.drain(..chunk_size).collect())).is_err() {
                            break;
                        }
                    }
                }
                reader
            })
        };
        Self { reader_thread_handle: Some(reader_thread_handle), chunk_size, buf_count, channel_receiver: Some(rx), stop_flag, unpause_flag }
    }
}
impl<R: Read+Seek+Send+'static> ThreadedChunkedReaderIter<R> {
    /// Constructs a new [`ThreadedChunkedReaderIter`] that rewinds the reader to ensure that all data is yielded by the iterator.
    /// See [`ThreadedChunkedReaderIter::new`] for descriptions of the other parameters.
    pub fn new_with_rewind(mut reader: R, chunk_size: usize, buf_count: usize) -> Self {
        reader.seek(SeekFrom::Start(0)).unwrap();
        Self::new(reader, chunk_size, buf_count)
    }
}
impl<R: Send> Iterator for ThreadedChunkedReaderIter<R> {
    type Item = IOResult<Box<[u8]>>;

    /// Yields `self.chunk_size` bytes at a time until reaching EOF, after which it yields the remaining bytes before returning `None`.
    /// All bytes successfully read are eventually returned: if reads into the buffer result in an error, the error is passed up, but the successfully read data will be returned the next time the buffer is successfully filled.
    /// 
    /// Note: If reading from a readable object that is being concurrently modified (e.g. a file that is being appended to by another process),
    /// EOF may be hit more than once, resulting in more chunks after a chunk smaller than `self.chunk_size` or more chunks after yielding `None`.
    /// (This is also a concern with the base [`Read`] trait, which may return more data even after returning `Ok(0)`).
    fn next(&mut self) -> Option<Self::Item> {
        // Swap in a true value for whatever was there before
        // If a false value was there previously, then the thread is waiting
        // So we need to wake the thread
        if self.unpause_flag.swap(1, Ordering::AcqRel) == 0 {
            wake_all(self.unpause_flag.deref());
        }
        if let Some(ref rx) = self.channel_receiver {
                match rx.recv() {
                Ok(Ok(data)) if data.len()==0 => None,
                Ok(Ok(data)) => Some(Ok(data)),
                Ok(Err(e)) => Some(Err(e)),
                Err(_) => None,
            }
        } else {
            // self.channel_receiver should always be live unless in destructor
            unreachable!("self.channel_receiver deinitialized while in use")
        }
    }
}

impl<R: Send> Drop for ThreadedChunkedReaderIter<R> {
    fn drop(&mut self) {
        // This is similar to what happens in self.into_inner(), except that
        // self.into_inner() may have already stopped the thread before we
        // get here. Most of the cleanup is idempotent but do checks on the
        // steps that aren't
        self.stop_flag.store(true, Ordering::Release);
        // Unpause and wake the thread if necessary
        if self.unpause_flag.swap(1, Ordering::AcqRel) == 0 {
            wake_all(self.unpause_flag.deref());
        }
        // Take and drop the receiver, if live, so that thread sends error
        drop(self.channel_receiver.take());
        // Wait for thread to stop if it hasn't already
        if let Some(handle) = self.reader_thread_handle.take() {
            handle.join().unwrap();
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
        let mut funny_read_iter = ThreadedChunkedReaderIter::new(funny_read, 4, 2);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[0,1,2,3]);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[4,5,6,7]);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[8,9,10,11]);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[12,13,14,15]);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[16,17,18,19]);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[20,21,22,23]);
    }
    #[test]
    fn chunked_read_iter_icecuberead() {
        let funny_read = IceCubeRead::default();
        let mut funny_read_iter = ThreadedChunkedReaderIter::new(funny_read, 2, 5);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[9,99]);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[0x99,9]);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[99,0x99]);
        assert_eq!(funny_read_iter.next().unwrap().unwrap_err().kind(), ErrorKind::Other);
        assert!(funny_read_iter.next().is_none());
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), &[9,99]);
    }
    #[test]
    fn chunked_read_iter_truncatedread() {
        let funny_read = TruncatedRead::default();
        let mut funny_read_iter = ThreadedChunkedReaderIter::new(funny_read, 3, 1);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), b"rei");
        assert_eq!(funny_read_iter.next().unwrap().unwrap_err().kind(), ErrorKind::Other);
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), b"mu");
        assert_eq!(funny_read_iter.next().unwrap().unwrap().as_ref(), b"rei");
    }

    #[test]
    fn chunked_read_iter_cursor_large() {
        let data_buf = [1,2,3,4,5,6,7,8,9];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ThreadedChunkedReaderIter::new(data_cursor, 4, 2);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[1,2,3,4]);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[5,6,7,8]);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[9]);
        assert!(data_chunk_iter.next().is_none());
    }
    #[test]
    fn chunked_read_iter_cursor_while() {
        let data_buf = [1,2,3,4,5,6,7,8,9];
        let data_cursor = Cursor::new(data_buf);

        let data_chunks: Vec<_> = ThreadedChunkedReaderIter::new(data_cursor, 4, 2).collect();
        let data_chunks_as_slice: Vec<&[u8]> = data_chunks.iter()
            .map(|r| r.as_ref().unwrap().as_ref())
            .collect();
        let expected_data_chunks: &[&[u8]] = &[&[1,2,3,4], &[5,6,7,8], &[9]];
        assert_eq!(data_chunks_as_slice.as_slice(), expected_data_chunks);
    }
    #[test]
    fn chunked_read_iter_cursor_large_buf_eq_chunk() {
        let data_buf = [1,2,3,4,5,6,7,8,9];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ThreadedChunkedReaderIter::new(data_cursor, 4, 1);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[1,2,3,4]);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[5,6,7,8]);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[9]);
        assert!(data_chunk_iter.next().is_none());
    }
    #[test]
    fn chunked_read_iter_cursor_large_buf_zero_chunk() {
        let data_buf = [1,2,3,4,5,6,7,8,9];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ThreadedChunkedReaderIter::new(data_cursor, 4, 0);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[1,2,3,4]);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[5,6,7,8]);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[9]);
        assert!(data_chunk_iter.next().is_none());
    }
    #[test]
    fn chunked_read_iter_cursor_smol() {
        let data_buf = [1,2,3];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ThreadedChunkedReaderIter::new(data_cursor, 4, 1);
        assert_eq!(data_chunk_iter.next().unwrap().unwrap().as_ref(), &[1,2,3]);
        assert!(data_chunk_iter.next().is_none());
    }
}