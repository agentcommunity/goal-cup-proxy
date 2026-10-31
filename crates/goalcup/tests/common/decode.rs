//! Body encoders and the synthetic SSE text the decode tests share (`proxy_decode.rs`,
//! `lessons_decode.rs`).

use flate2::Compression;
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use std::io::Write;

/// 2,000 synthetic SSE events, the plaintext of `proxy_fixtures/anthropic-sse.br`.
pub fn sse() -> Vec<u8> {
    (0..2000)
        .map(|i| {
            format!(
                "event: content_block_delta\ndata: {{\"index\":{i},\"delta\":{{\"text\":\"token {i}\"}}}}\n\n"
            )
        })
        .collect::<String>()
        .into_bytes()
}

pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

pub fn raw_deflate(data: &[u8]) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}
