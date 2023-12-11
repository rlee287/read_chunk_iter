#![forbid(unsafe_code)]

#![doc(html_root_url = "https://docs.rs/read_chunk_iter/0.1.0")]

mod simple;
mod threaded;

pub(crate) mod dev_helpers;

pub use simple::ChunkedReaderIter;
pub use threaded::ThreadedChunkedReaderIter;