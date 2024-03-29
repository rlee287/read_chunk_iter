use std::io::Result as IOResult;
use std::io::{ErrorKind, Read, Seek};

use std::num::NonZeroUsize;

use crate::vectored_read::{read_vectored_into_buf, resolve_read_vectored, VectoredReadSelect};

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, TrySendError};
use std::sync::Arc;
use std::thread::spawn as thread_spawn;
use std::thread::JoinHandle;

use atomic_wait::{wait, wake_all};
use std::ops::Deref;

/// An iterator adapter for readers that yields chunks of bytes in a `Box<[u8]>` and performs reads asynchronously via a thread.
///
#[derive(Debug)]
pub struct ThreadedChunkedReaderIter<R: Send> {
    // Thread yields back the reader so we can still do into_inner
    // This is an Option<> so that we can take() from it in destructor
    reader_thread_handle: Option<JoinHandle<R>>,
    reader_vectored: VectoredReadSelect,
    chunk_size: NonZeroUsize,
    buf_count: usize,
    // This is an Option<> so that we can take() from it in destructor
    channel_receiver: Option<Receiver<IOResult<Box<[u8]>>>>,
    stop_flag: Arc<AtomicBool>,
    // Semantically this is a bool but atomic_wait only supports U32
    // Flag is cleared only in the read thread and set only in the main thread
    // so there is no ABA problem with waiting on a value
    unpause_flag: Arc<AtomicU32>,
}
impl<R: Send> ThreadedChunkedReaderIter<R> {
    /// Returns the wrapped reader and any unyielded data. This includes both
    /// any IOErrors that occured on previous read attempts and unyielded data
    /// that was read before then. Read bytes have been coalesced, i.e. it is
    /// guaranteed that all instances of data are either followed by an error
    /// or occur at the end of the returned `Vec`.
    ///
    /// In almost all cases, the returned Vec will contain a single boxed slice
    /// followed by at most one I/O error, as the reading thread pauses upon
    /// encountering an I/O error and only resumes when more data is requested.
    /// However, a spurious thread wakeup may occur which would cause another
    /// read to be attempted and could lead to multiple I/O errors being
    /// enqueued.
    pub fn into_inner(mut self) -> (Vec<IOResult<Box<[u8]>>>, R) {
        self.stop_flag.store(true, Ordering::Release);
        // Unpause and wake the thread if necessary
        if self.unpause_flag.swap(1, Ordering::AcqRel) == 0 {
            wake_all(self.unpause_flag.deref());
        }
        // Take the receiver, get remaining data, and drop it
        // so that thread sends error
        // We already signalled stop above so we won't get stuck here
        let remaining_data = match self.channel_receiver.take() {
            Some(recv) => {
                let mut vec_return = Vec::new();
                let mut vec_byte_flatten = Vec::new();
                for item in recv {
                    match item {
                        Ok(boxed) => vec_byte_flatten.extend(boxed.as_ref()),
                        Err(e) => {
                            if !vec_byte_flatten.is_empty() {
                                vec_return.push(Ok(vec_byte_flatten.drain(..).collect()));
                            }
                            vec_return.push(Err(e));
                        }
                    }
                }
                if !vec_byte_flatten.is_empty() {
                    vec_return.push(Ok(vec_byte_flatten.into_boxed_slice()));
                }
                vec_return
            }
            None => Vec::new(),
        };
        let reader = self.reader_thread_handle.take().unwrap().join().unwrap();
        (remaining_data, reader)
    }
    /// Returns the chunk size which is yielded by the iterator.
    #[inline]
    pub fn chunk_size(&self) -> NonZeroUsize {
        self.chunk_size
    }
    /// Returns whether the reads will be vectored or not.
    #[inline]
    pub fn vectored_read_select(&self) -> VectoredReadSelect {
        self.reader_vectored
    }
    /// Returns the size of the buffer used to read from the underlying reader.
    #[inline]
    pub fn buf_size(&self) -> NonZeroUsize {
        match NonZeroUsize::new(self.buf_count) {
            None => self.chunk_size,
            Some(len) => self.chunk_size.checked_mul(len).unwrap(),
        }
    }
    // Cannot provide buf view due to multithreading
}
impl<R: Read + Send + 'static> ThreadedChunkedReaderIter<R> {
    /// Instantiates a new [`ThreadedChunkedReaderIter`] that asynchronously tries to read up to `chunk_size*buf_count` bytes at a time and that yields `chunk_size` bytes as an iterator until reaching EOF.
    /// The use of a thread allows I/O reads to occur in the background while the program performs processing on the data.
    /// (If `buf_count` is zero, then `chunk_size` bytes are read at a time, and the thread waits for the chunk to be yielded before reading again.)
    ///
    /// For readers that implement `Seek`, [`Self::new_with_rewind`] rewinds the given reader.
    pub fn new(
        mut reader: R,
        chunk_size: NonZeroUsize,
        buf_count: usize,
        reader_vectored: VectoredReadSelect,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let unpause_flag = Arc::new(AtomicU32::new(1));
        let (tx, rx) = sync_channel(buf_count);

        let reader_thread_handle = {
            let stop_flag = stop_flag.clone();
            let unpause_flag = unpause_flag.clone();

            thread_spawn(move || {
                let buf_size = match NonZeroUsize::new(buf_count) {
                    None => chunk_size,
                    Some(len) => chunk_size.checked_mul(len).unwrap(),
                };
                let mut buf = Vec::with_capacity(buf_size.into());
                // Code modified from ChunkedReaderIter::next()
                'read_loop: while !stop_flag.load(Ordering::Acquire) {
                    // unpause_flag only gets set if we hit EOF earlier
                    // If we did, we don't want to retry until newly requested
                    // Otherwise we get stuck in a spinloop of reading nothing
                    // `wait` might return spuriously but the worst that happens
                    // is that we go around the loop and wait here again
                    // TODO: is it worth it to do a loop like with condvars?
                    wait(&unpause_flag, 0);
                    // Don't try to read again if we're supposed to stop
                    if stop_flag.load(Ordering::Acquire) {
                        break 'read_loop;
                    }

                    let mut read_offset = buf.len();
                    // Temporarily resize Vec to try to fill it
                    // Due to initialization with `with_capacity` we do not need to reallocate
                    buf.resize(buf_size.into(), 0x00);
                    // Try to fill entire buf, but we're good if we have a whole chunk
                    while read_offset < chunk_size.into() {
                        let reader_result = match resolve_read_vectored(&reader, reader_vectored) {
                            true => read_vectored_into_buf(
                                &mut reader,
                                &mut buf[read_offset..],
                                chunk_size,
                            ),
                            false => reader.read(&mut buf[read_offset..]),
                        };
                        match reader_result {
                            Ok(0) => {
                                break;
                            }
                            Ok(n) => {
                                read_offset += n;
                            }
                            Err(e) if e.kind() == ErrorKind::Interrupted => { /* continue */ }
                            Err(e) => {
                                // Shrink Vec back to how much was actually read
                                buf.truncate(read_offset);
                                // Yield currently read data before yielding Err
                                // We are in loop so read_offset < chunk_size
                                if read_offset > 0 {
                                    let boxed_data: Box<[u8]> = buf.drain(..).collect();
                                    if tx.send(Ok(boxed_data)).is_err() {
                                        break 'read_loop;
                                    }
                                }
                                // IOError -> no more data -> pause until more is requested
                                unpause_flag.store(0, Ordering::Release);
                                // On send error we no longer have anyone listening
                                match tx.send(Err(e)) {
                                    Ok(_) => {
                                        continue 'read_loop;
                                    }
                                    Err(_) => {
                                        break 'read_loop;
                                    }
                                }
                            }
                        }
                    }
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
                            Ok(_) => {
                                continue 'read_loop;
                            }
                            Err(_) => {
                                break 'read_loop;
                            }
                        }
                    }
                    // Shrink Vec back to how much was actually read
                    buf.truncate(read_offset);

                    if usize::from(chunk_size) > read_offset {
                        // Yield the remaining data at EOF
                        let boxed_data: Box<[u8]> = buf.drain(..).collect();
                        // Since we ran out of data, do a pause until more is requested
                        unpause_flag.store(0, Ordering::Release);
                        match tx.send(Ok(boxed_data)) {
                            Ok(_) => {
                                continue 'read_loop;
                            }
                            Err(_) => {
                                break 'read_loop;
                            }
                        }
                    } else {
                        let mut bytes_yielded = 0;

                        // Yield full chunks while we have them
                        while buf[bytes_yielded..].len() >= chunk_size.into() {
                            match tx.try_send(Ok(buf
                                [bytes_yielded..bytes_yielded + usize::from(chunk_size)]
                                .iter()
                                .copied()
                                .collect()))
                            {
                                Ok(()) => {
                                    // OK to remove chunk from the vec
                                    // Update bookkeeping but delay removal
                                    bytes_yielded += usize::from(chunk_size);
                                }
                                Err(TrySendError::Full(b)) => {
                                    // May as well drain the vec while waiting
                                    buf.drain(..bytes_yielded);
                                    bytes_yielded = 0;

                                    // If the buffer isn't full, then go around
                                    // and try to fill it up in the meantime
                                    // If it is full, then block until it isn't
                                    if buf.len() >= buf_size.into() {
                                        if tx.send(b).is_err() {
                                            break 'read_loop;
                                        }
                                        // Successful send -> can remove from vec
                                        bytes_yielded += usize::from(chunk_size);
                                    }
                                }
                                Err(TrySendError::Disconnected(_)) => {
                                    break 'read_loop;
                                }
                            }
                        }
                        // OK to remove chunks from the vec
                        buf.drain(..bytes_yielded);
                    }
                }
                reader
            })
        };
        Self {
            reader_thread_handle: Some(reader_thread_handle),
            reader_vectored,
            chunk_size,
            buf_count,
            channel_receiver: Some(rx),
            stop_flag,
            unpause_flag,
        }
    }
}
impl<R: Read + Seek + Send + 'static> ThreadedChunkedReaderIter<R> {
    /// Constructs a new [`ThreadedChunkedReaderIter`] that rewinds the reader to ensure that all data is yielded by the iterator.
    /// See [`ThreadedChunkedReaderIter::new`] for descriptions of the other parameters.
    pub fn new_with_rewind(
        mut reader: R,
        chunk_size: NonZeroUsize,
        buf_count: usize,
        reader_vectored: VectoredReadSelect,
    ) -> Self {
        reader.rewind().unwrap();
        Self::new(reader, chunk_size, buf_count, reader_vectored)
    }
}
impl<R: Send> Iterator for ThreadedChunkedReaderIter<R> {
    type Item = IOResult<Box<[u8]>>;

    /// Yields `self.chunk_size` bytes at a time until reaching EOF, after which it yields the remaining bytes before returning `None`.
    /// All bytes successfully read are eventually returned: if reads into the buffer result in an error, the previously read data is yielded first, and then the error is passed up.
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
                Ok(Ok(data)) if data.len() == 0 => None,
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

    use std::thread::sleep;
    use std::time::Duration;

    use std::convert::TryFrom;

    use crate::dev_helpers::{FunnyRead, IceCubeRead, TruncatedRead};

    #[test]
    fn chunked_read_iter_funnyread() {
        let funny_read = FunnyRead::default();
        let mut funny_read_iter = ThreadedChunkedReaderIter::new(
            funny_read,
            NonZeroUsize::new(4).unwrap(),
            2,
            VectoredReadSelect::Yes,
        );
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
        // Sleep to make reader buf fill and exercise TrySendError::Full logic
        sleep(Duration::from_millis(1));
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
        let mut funny_read_iter = ThreadedChunkedReaderIter::new(
            funny_read,
            NonZeroUsize::new(2).unwrap(),
            5,
            VectoredReadSelect::No,
        );
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
        let mut funny_read_iter = ThreadedChunkedReaderIter::new(
            funny_read,
            NonZeroUsize::new(3).unwrap(),
            1,
            VectoredReadSelect::No,
        );
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
        let mut funny_read_iter = ThreadedChunkedReaderIter::new(
            funny_read,
            NonZeroUsize::new(11).unwrap(),
            2,
            VectoredReadSelect::No,
        );
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
        let mut data_chunk_iter = ThreadedChunkedReaderIter::new(
            data_cursor,
            NonZeroUsize::new(4).unwrap(),
            2,
            VectoredReadSelect::No,
        );
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

        let (unyielded_data, _) = data_chunk_iter.into_inner();
        assert!(unyielded_data.is_empty());
    }
    #[test]
    fn chunked_read_iter_cursor_large_into_inner() {
        let data_buf = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ThreadedChunkedReaderIter::new(
            data_cursor,
            NonZeroUsize::new(4).unwrap(),
            2,
            VectoredReadSelect::No,
        );
        assert_eq!(
            data_chunk_iter.next().unwrap().unwrap().as_ref(),
            &[1, 2, 3, 4]
        );
        let (unyielded_data, mut cursor) = data_chunk_iter.into_inner();
        // We may not have read to the end so check how much was actually read
        let cursor_pos = usize::try_from(cursor.stream_position().unwrap() - 4).unwrap();
        let unyielded_data_bytes: Vec<_> = unyielded_data
            .into_iter()
            .map(|x| x.unwrap())
            .flat_map(Vec::from)
            .collect();
        assert_eq!(unyielded_data_bytes, &[5, 6, 7, 8, 9][..cursor_pos]);
    }
    #[test]
    fn chunked_read_iter_cursor_while() {
        let data_buf = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        let data_cursor = Cursor::new(data_buf);

        let data_chunks: Vec<_> = ThreadedChunkedReaderIter::new(
            data_cursor,
            NonZeroUsize::new(4).unwrap(),
            2,
            VectoredReadSelect::No,
        )
        .collect();
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
        let mut data_chunk_iter = ThreadedChunkedReaderIter::new(
            data_cursor,
            NonZeroUsize::new(4).unwrap(),
            1,
            VectoredReadSelect::No,
        );
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
    fn chunked_read_iter_cursor_large_buf_zero_chunk() {
        let data_buf = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        let data_cursor = Cursor::new(data_buf);
        let mut data_chunk_iter = ThreadedChunkedReaderIter::new(
            data_cursor,
            NonZeroUsize::new(4).unwrap(),
            0,
            VectoredReadSelect::No,
        );
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
}
