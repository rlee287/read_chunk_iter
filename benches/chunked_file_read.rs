use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use itertools::Itertools;

use read_chunk_iter::{ChunkedReaderIter, ThreadedChunkedReaderIter};

use std::fs::File;
use std::io::{BufReader, Read, Seek, Write};

use poly1305::Poly1305;

use poly1305::universal_hash::{KeyInit, UniversalHash};
use tempfile::NamedTempFile;

fn do_std_iter_bufread(file: File, chunk_size: usize) {
    let bufread = BufReader::with_capacity(chunk_size, file);
    for chunk in &bufread.bytes().chunks(chunk_size) {
        assert_eq!(
            chunk.map(|x| x.unwrap()).collect::<Vec<_>>(),
            vec![0xf0; chunk_size]
        );
    }
}
fn do_chunked_read(file: File, chunk_size: usize, multiplier: usize) {
    let read_iter = ChunkedReaderIter::new(file, chunk_size, chunk_size * multiplier);
    for chunk in read_iter {
        assert_eq!(chunk.unwrap().as_ref(), vec![0xf0; chunk_size].as_slice());
    }
}
fn do_threaded_chunked_read(file: File, chunk_size: usize, multiplier: usize) {
    let read_iter = ThreadedChunkedReaderIter::new(file, chunk_size, multiplier);
    for chunk in read_iter {
        assert_eq!(chunk.unwrap().as_ref(), vec![0xf0; chunk_size].as_slice());
    }
}
fn do_chunked_read_hash(file: File, chunk_size: usize, multiplier: usize) {
    let read_iter = ChunkedReaderIter::new(file, chunk_size, chunk_size * multiplier);
    let mut hash_obj = Poly1305::new_from_slice(&[0x13; 32]).unwrap();
    for chunk in read_iter {
        hash_obj.update_padded(&chunk.unwrap());
    }
    assert_ne!(hash_obj.finalize().as_slice(), &[0xff; 16]);
}
fn do_threaded_chunked_read_hash(file: File, chunk_size: usize, multiplier: usize) {
    let read_iter = ThreadedChunkedReaderIter::new(file, chunk_size, multiplier);
    let mut hash_obj = Poly1305::new_from_slice(&[0x13; 32]).unwrap();
    for chunk in read_iter {
        hash_obj.update_padded(&chunk.unwrap());
    }
    assert_ne!(hash_obj.finalize().as_slice(), &[0xff; 16]);
}

fn criterion_benchmark_multiplier(c: &mut Criterion) {
    let mut temp_file = NamedTempFile::new().unwrap();
    temp_file.write_all(&[0xf0; 1024 * 1024 * 2]).unwrap();
    temp_file.flush().unwrap();
    temp_file.rewind().unwrap();

    let temp_file_path = temp_file.path();

    let mut group: criterion::BenchmarkGroup<'_, criterion::measurement::WallTime> =
        c.benchmark_group("(verify, filesize:2MiB, chunk:8192, multiplier:?)");

    for multiplier in [1, 2, 4, 6, 8] {
        group.bench_with_input(
            BenchmarkId::new("simple", multiplier),
            &multiplier,
            |b, &mult| {
                b.iter_batched(
                    || File::open(temp_file_path).unwrap(),
                    |file| do_chunked_read(file, black_box(8192), mult),
                    criterion::BatchSize::PerIteration,
                );
            },
        );
        group.bench_with_input(
            BenchmarkId::new("threaded", multiplier),
            &multiplier,
            |b, &mult| {
                b.iter_batched(
                    || File::open(temp_file_path).unwrap(),
                    |file| do_threaded_chunked_read(file, black_box(8192), mult),
                    criterion::BatchSize::PerIteration,
                );
            },
        );
    }
    group.finish();
}

fn criterion_benchmark_multiplier_hash(c: &mut Criterion) {
    let mut temp_file = NamedTempFile::new().unwrap();
    temp_file.write_all(&[0xf0; 1024 * 1024 * 2]).unwrap();
    temp_file.flush().unwrap();
    temp_file.rewind().unwrap();

    let temp_file_path = temp_file.path();

    let mut group = c.benchmark_group("(hash, filesize:2MiB, chunk:8192, multiplier:?)");

    for multiplier in [1, 2, 4, 6, 8] {
        group.bench_with_input(
            BenchmarkId::new("simple", multiplier),
            &multiplier,
            |b, &mult| {
                b.iter_batched(
                    || File::open(temp_file_path).unwrap(),
                    |file| do_chunked_read_hash(file, black_box(8192), mult),
                    criterion::BatchSize::PerIteration,
                );
            },
        );
        group.bench_with_input(
            BenchmarkId::new("threaded", multiplier),
            &multiplier,
            |b, &mult| {
                b.iter_batched(
                    || File::open(temp_file_path).unwrap(),
                    |file| do_threaded_chunked_read_hash(file, black_box(8192), mult),
                    criterion::BatchSize::PerIteration,
                );
            },
        );
    }
    group.finish();
}

fn criterion_benchmark_filesize(c: &mut Criterion) {
    let mut group = c.benchmark_group("(verify, filesize:?, chunk:16384, multiplier:4)");

    for file_size in [128, 256, 512, 1024, 1024 * 2] {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&vec![0xf0; 1024 * file_size]).unwrap();
        temp_file.flush().unwrap();
        temp_file.rewind().unwrap();

        let temp_file_path = temp_file.path();

        group.bench_with_input(
            BenchmarkId::new("simple", file_size),
            &file_size,
            |b, &_size| {
                b.iter_batched(
                    || File::open(temp_file_path).unwrap(),
                    |file| do_chunked_read(file, black_box(16384), black_box(4)),
                    criterion::BatchSize::PerIteration,
                );
            },
        );
        group.bench_with_input(
            BenchmarkId::new("threaded", file_size),
            &file_size,
            |b, &_size| {
                b.iter_batched(
                    || File::open(temp_file_path).unwrap(),
                    |file| do_threaded_chunked_read(file, black_box(16384), black_box(4)),
                    criterion::BatchSize::PerIteration,
                );
            },
        );
    }
    group.finish();
}

fn criterion_benchmark_chunksize(c: &mut Criterion) {
    let mut group = c.benchmark_group("(verify, filesize:2MiB, chunk:?, multiplier:4)");

    for chunk_size in [1024 * 2, 1024 * 4, 1024 * 8, 1024 * 16, 1024 * 32] {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&[0xf0; 1024 * 1024 * 2]).unwrap();
        temp_file.flush().unwrap();
        temp_file.rewind().unwrap();

        let temp_file_path = temp_file.path();

        group.bench_with_input(
            BenchmarkId::new("iter_bufread", chunk_size),
            &chunk_size,
            |b, &size| {
                b.iter_batched(
                    || File::open(temp_file_path).unwrap(),
                    |file| do_std_iter_bufread(file, size * 4),
                    criterion::BatchSize::PerIteration,
                );
            },
        );
        group.bench_with_input(
            BenchmarkId::new("simple", chunk_size),
            &chunk_size,
            |b, &size| {
                b.iter_batched(
                    || File::open(temp_file_path).unwrap(),
                    |file| do_chunked_read(file, size, black_box(4)),
                    criterion::BatchSize::PerIteration,
                );
            },
        );
        group.bench_with_input(
            BenchmarkId::new("threaded", chunk_size),
            &chunk_size,
            |b, &size| {
                b.iter_batched(
                    || File::open(temp_file_path).unwrap(),
                    |file| do_threaded_chunked_read(file, size, black_box(4)),
                    criterion::BatchSize::PerIteration,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    criterion_benchmark_multiplier,
    criterion_benchmark_multiplier_hash,
    criterion_benchmark_filesize,
    criterion_benchmark_chunksize
);
criterion_main!(benches);
