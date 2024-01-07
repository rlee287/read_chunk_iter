use std::env::args;

use std::fs::File;
use std::io::{BufReader, Read};

use itertools::Itertools;

use poly1305::universal_hash::{KeyInit, UniversalHash};
use poly1305::Poly1305;
use read_chunk_iter::{VectoredReadSelect, ChunkedReaderIter, ThreadedChunkedReaderIter};

// This is the default buffer size for a BufReader
const CHUNK_SIZE: usize = 8192;

fn main() -> Result<(), String> {
    let args: Vec<_> = args().collect();
    if args.len() != 3 {
        eprintln!("Got wrong number of arguments: args {:?}", args);
        return Err(format!(
            "Usage: {} (bufread|simple|threaded) filename",
            args[0]
        ));
    }
    let mut hash_obj = Poly1305::new_from_slice(&[0x13; 32]).unwrap();
    let file = File::open(&args[2]).map_err(|e| format!("Error opening file: {}", e))?;
    match args[1].as_str() {
        "bufread" => {
            let bufread = BufReader::with_capacity(8192, file);
            for chunk in &bufread.bytes().chunks(CHUNK_SIZE) {
                hash_obj.update_padded(&chunk.map(|x| x.unwrap()).collect::<Vec<_>>());
            }
        }
        "simple" => {
            let read_iter = ChunkedReaderIter::new(file, CHUNK_SIZE, CHUNK_SIZE, VectoredReadSelect::No);
            for chunk in read_iter {
                hash_obj.update_padded(&chunk.unwrap());
            }
        }
        "threaded" => {
            let read_iter = ThreadedChunkedReaderIter::new(file, CHUNK_SIZE, 1, VectoredReadSelect::No);
            for chunk in read_iter {
                hash_obj.update_padded(&chunk.unwrap());
            }
        }
        _ => {
            eprintln!("Got method {}", args[1]);
            return Err(format!(
                "Usage: {} (bufread|simple|threaded) filename",
                args[0]
            ));
        }
    }
    for byte in hash_obj.finalize() {
        print!("{:02x}", byte);
    }
    println!();
    Ok(())
}
