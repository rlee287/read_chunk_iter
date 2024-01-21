//! Iterator adapters over a reader that yields fixed-size chunks at a time.
//! These iterators are substantially more performant than using iterator 
//! adapters over a `Bytes` iterator of a `Read` object.
//!
//! All the iterators in this crate will yield the exactly specified number of bytes, except under the following circumstances:
//!  - EOF is reached, in which case a partial chunk is yielded. (This can occur multiple times if EOF is hit multiple times, e.g. with a network socket.)
//!  - An IO error other than `ErrorKind::Interrupted` occurs, in which case a partial chunk is yielded before the error. This preserves the exact byte location at which an error occured.
//!
//! ## Features
//! - autodetect_vectored: Enable automatic detection of whether vectored reads offer speedups, and take advantage of them when they offer speedups. This feature requires nightly, but manual selection of vectored reads is still possible without it.
#![forbid(unsafe_code)]
#![cfg_attr(feature = "autodetect_vectored", feature(can_vector))]

mod simple;
mod threaded;

mod vectored_read;

pub(crate) mod dev_helpers;

pub use vectored_read::VectoredReadSelect;

pub use simple::ChunkedReaderIter;
pub use threaded::ThreadedChunkedReaderIter;
