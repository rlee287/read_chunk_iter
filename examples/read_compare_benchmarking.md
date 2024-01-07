These are benchmark results from the author's computer using `hyperfine` to time the example `read_compare.rs` binary against a 64MiB file generated using `cat /dev/urandom | head -c 67108864 > examples/random_bytes.bin`.

TL;DR:

|                      | `BufReader` + iter | `ChunkedReaderIter` | `ThreadedChunkedReaderIter` |
|----------------------|--------------------|---------------------|----------------------------|
| Warm file cache      | 2.265 s | 61.7 ms  | 55.8 ms  |
| Cold file cache, HDD | 2.482 s | 975.0 ms | 987.2 ms |
| Cold file cache, SSD | 2.291 s | 120.6 ms | 125.0 ms |

The tradeoff between synchronous reading and threaded asynchronous reading is application-dependent, but `BufReader` is clearly much slower.

With a warm file cache:

```
# In one shell, lock the file's contents into memory cache for consistency
$ vmtouch -l examples/random_bytes.bin
# Run the benchmark in another shell
$ hyperfine --warmup 1 -L method bufread,simple,threaded -- './target/release/examples/read_compare {method} examples/random_bytes.bin'
Benchmark 1: ./target/release/examples/read_compare bufread examples/random_bytes.bin
  Time (mean ± σ):      2.265 s ±  0.005 s    [User: 2.245 s, System: 0.020 s]
  Range (min … max):    2.257 s …  2.275 s    10 runs
 
Benchmark 2: ./target/release/examples/read_compare simple examples/random_bytes.bin
  Time (mean ± σ):      61.7 ms ±   1.6 ms    [User: 43.0 ms, System: 18.6 ms]
  Range (min … max):    58.6 ms …  65.9 ms    45 runs
 
Benchmark 3: ./target/release/examples/read_compare threaded examples/random_bytes.bin
  Time (mean ± σ):      55.8 ms ±   1.4 ms    [User: 68.2 ms, System: 33.8 ms]
  Range (min … max):    52.2 ms …  58.8 ms    50 runs
 
Summary
  ./target/release/examples/read_compare threaded examples/random_bytes.bin ran
    1.10 ± 0.04 times faster than ./target/release/examples/read_compare simple examples/random_bytes.bin
   40.55 ± 1.03 times faster than ./target/release/examples/read_compare bufread examples/random_bytes.bin
```

With a cold file cache on an HDD:

```
$ hyperfine --warmup 1 --prepare 'vmtouch -e examples/random_bytes.bin' -L method bufread,simple,threaded -- './target/release/examples/read_compare {method} examples/random_bytes.bin'
Benchmark 1: ./target/release/examples/read_compare bufread examples/random_bytes.bin
  Time (mean ± σ):      2.482 s ±  0.057 s    [User: 2.243 s, System: 0.057 s]
  Range (min … max):    2.380 s …  2.593 s    10 runs
 
Benchmark 2: ./target/release/examples/read_compare simple examples/random_bytes.bin
  Time (mean ± σ):     975.0 ms ±  50.6 ms    [User: 45.2 ms, System: 66.9 ms]
  Range (min … max):   913.9 ms … 1071.1 ms    10 runs
 
Benchmark 3: ./target/release/examples/read_compare threaded examples/random_bytes.bin
  Time (mean ± σ):     987.2 ms ±  61.1 ms    [User: 81.8 ms, System: 98.8 ms]
  Range (min … max):   873.9 ms … 1059.0 ms    10 runs
 
Summary
  ./target/release/examples/read_compare simple examples/random_bytes.bin ran
    1.01 ± 0.08 times faster than ./target/release/examples/read_compare threaded examples/random_bytes.bin
    2.55 ± 0.14 times faster than ./target/release/examples/read_compare bufread examples/random_bytes.bin
```

With a cold file cache on an SSD:

```
$ hyperfine --warmup 1 --prepare 'vmtouch -e /tmp/random_bytes.bin' -L method bufread,simple,threaded -- './target/release/examples/read_compare {method} /tmp/random_bytes.bin'
Benchmark 1: ./target/release/examples/read_compare bufread /tmp/random_bytes.bin
  Time (mean ± σ):      2.291 s ±  0.009 s    [User: 2.234 s, System: 0.057 s]
  Range (min … max):    2.277 s …  2.306 s    10 runs
 
Benchmark 2: ./target/release/examples/read_compare simple /tmp/random_bytes.bin
  Time (mean ± σ):     120.6 ms ±   1.7 ms    [User: 41.1 ms, System: 50.3 ms]
  Range (min … max):   117.8 ms … 125.3 ms    21 runs
 
Benchmark 3: ./target/release/examples/read_compare threaded /tmp/random_bytes.bin
  Time (mean ± σ):     125.0 ms ±   3.1 ms    [User: 75.2 ms, System: 67.1 ms]
  Range (min … max):   122.1 ms … 136.1 ms    21 runs
 
Summary
  ./target/release/examples/read_compare simple /tmp/random_bytes.bin ran
    1.04 ± 0.03 times faster than ./target/release/examples/read_compare threaded /tmp/random_bytes.bin
   18.99 ± 0.28 times faster than ./target/release/examples/read_compare bufread /tmp/random_bytes.bin
```