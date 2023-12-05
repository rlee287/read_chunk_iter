#![forbid(unsafe_code)]

#![doc(html_root_url = "https://docs.rs/read_chunk_iter/0.1.0")]

mod simple;

pub(crate) mod dev_helpers;

pub use simple::ChunkedReaderIter;