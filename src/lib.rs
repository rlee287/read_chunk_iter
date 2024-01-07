//! Iterator adapters over a reader that yields fixed-size chunks at a time.
//!
//! All the iterators in this crate will yield the exactly specified number of bytes, except under the following circumstances:
//!  - EOF is reached, in which case a partial chunk is yielded. (This can occur multiple times if EOF is hit multiple times, e.g. with a network socket.)
//!  - An IO error other than `ErrorKind::Interrupted` occurs, in which case a partial chunk is yielded before the error. This preserves the exact byte location at which an error occured.
#![forbid(unsafe_code)]
#![doc(html_root_url = "https://docs.rs/read_chunk_iter/0.1.0")]

mod simple;
mod threaded;

mod vectored_read;

pub(crate) mod dev_helpers;

pub use simple::ChunkedReaderIter;
pub use threaded::ThreadedChunkedReaderIter;
