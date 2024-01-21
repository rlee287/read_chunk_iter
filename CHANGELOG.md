# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2024-01-20

### Added

- Threaded `ThreadedChunkedReadIter` for asynchronous reading
- Ability to select vectored reading
- Examples and benchmarks for comparing performance

### Fixed

- Significantly improve performance by reducing unneeded memory copying

### Changed

- Read errors are now yielded with byte-level precision
- `usize` parameters that were required to be nonzero are now `NonZeroUsize`s

### Removed

- Nothing

## [0.1.0] - 2023-07-30

Initial release