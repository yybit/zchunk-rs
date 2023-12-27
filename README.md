[![crates.io](https://img.shields.io/crates/v/zchunk.svg)](https://crates.io/crates/zchunk)
[![docs.rs](https://docs.rs/zchunk/badge.svg)](https://docs.rs/zchunk)

## zchunk-rs

A pure rust library for parsing and generating [zchunk](https://github.com/zchunk/zchunk) file

### Example

* Compress
```rust
use std::fs::File;
use tempfile::Builder;
use zchunk::Encoder;

let input = File::open("test.txt").unwrap();
let output = File::create("test.txt.zck").unwrap();

let temp = Builder::new()
    .prefix("zchunk-temp-")
    .tempfile_in("tmp/")
    .unwrap();

let mut encoder = Encoder::new(input, temp).unwrap();
encoder.prepare_chunks().unwrap();
encoder.compress_to(output).unwrap();
```

* Decompress
```rust
use std::{fs::File, io::BufReader};
use zchunk::Decoder;

let input = File::open("test.txt.zck").unwrap();
let mut output = File::create("test.txt").unwrap();

let mut decoder = Decoder::new(BufReader::new(input)).unwrap();
decoder.decompress_to(&mut output).unwrap();
```

* Sync
```rust
use zchunk::Decoder;

let mut source_decoder = Decoder::new(&mut source_reader).unwrap();
let mut cache_decoder = Decoder::new(&mut cache_reader).unwrap();
source_decoder.sync_to(cache_decoder, &mut writer).unwrap();
```