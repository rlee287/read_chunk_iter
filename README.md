# read_chunk_iter

[![Crates.io](https://img.shields.io/crates/v/read_chunk_iter)](https://crates.io/crates/read_chunk_iter)
[![Crates.io](https://img.shields.io/crates/d/read_chunk_iter)](https://crates.io/crates/read_chunk_iter)
[![License](https://img.shields.io/badge/license-MIT-blue)](https://github.com/rlee287/range_union_find/blob/main/LICENSE-MIT)

Iterator adapters over a reader that yields fixed-size chunks at a time.

## Why not use generic iterator composition over Read objects?

A simple solution would be to use iterator adapters over the bytes of a file, e.g. `&BufReader::new(file_path).bytes().chunks(CHUNK_SIZE)`, using the `itertools` crate to provide the `chunks` adapter. However, using generic iterator adaptors is significantly slower than a dedicated iterator, with the timing comparison in the `examples` folder demonstrating a slowdown by a factor of 2.5-40.5. (This is including the use of `BufReader` to reduce the number of underlying `read` calls. Without such buffering, `bytes()` would call `read` once for each byte, resulting in a much larger slowdown.)

This crate offers two alternatives:

- `ChunkedReaderIter`, which synchronously reads from the underlying `Read` object and yields chunks of data when requested.
- `ThreadedChunkedReaderIter`, which performs the reads in a separate thread and transmits chunks of data to the originating thread.

Whether to use `ChunkedReaderIter` or `ThreadedChunkedReaderIter` depends on whether the saved time of asynchronous reads while doing other computations outweighs the overhead of threading. *Benchmark your particular use case before assuming that one is necessarily better than the other.*

# Features
- autodetect_vectored: Enable automatic detection of whether vectored reads offer speedups, and take advantage of them when they offer speedups. This feature requires nightly, but manual selection of vectored reads is still possible without it.

## Planned features

- fadvise: Use `posix_fadvise` to signal `POSIX_FADV_SEQUENTIAL` for the whole file and to provide the option to free filesystem cache with `POSIX_FADV_DONTNEED` on yielded data. This feature will be enabled by default but will be a no-op on non-Unix systems.