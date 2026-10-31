//! Body encoders for tests: gzip, zlib and raw DEFLATE of a slice, at the default level.

use flate2::Compression;
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use std::io::Write;

macro_rules! encoder {
    ($name:ident, $enc:ident) => {
        pub fn $name(data: &[u8]) -> Vec<u8> {
            let mut e = $enc::new(Vec::new(), Compression::default());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        }
    };
}
encoder!(gzip, GzEncoder);
encoder!(zlib, ZlibEncoder);
encoder!(raw_deflate, DeflateEncoder);
