//! The hand close of section 12, step 2, with the locked zstd crate: a frame written by the part
//! file encoder (level 3, window 2^27, long matching, no checksum, no content size), flushed twice,
//! closed with `01 00 00` and followed by a second frame, decodes as one stream with the default
//! decoder limits.

use goalcup_format::ZSTD_END_BLOCK;
use goalcup_format::part::new_encoder;
use std::io::Write;

/// Bytes that compress.
fn data(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let sym = b"abcdefgh {}\":,\n0123";
            sym[(x % sym.len() as u64) as usize]
        })
        .collect()
}

#[test]
fn hand_closed_frame_then_a_new_frame_decode_as_one_stream() {
    let a = data(2 << 20, 1);
    let b = data(6 << 20, 2);
    let mut e = new_encoder(Vec::new(), 27).unwrap();
    e.write_all(&a).unwrap();
    e.flush().unwrap();
    e.write_all(&b).unwrap();
    e.write_all(&a).unwrap();
    e.flush().unwrap();
    let mut z = e.get_ref().clone();
    drop(e);

    // Frame header: magic, descriptor 0x00 (no content size, no checksum, not single segment),
    // window descriptor 0x88 (2^27).
    assert_eq!(z[..6], [0x28, 0xb5, 0x2f, 0xfd, 0x00, 0x88]);

    z.extend_from_slice(&ZSTD_END_BLOCK);
    let c = data(1 << 20, 3);
    let mut e2 = new_encoder(Vec::new(), 27).unwrap();
    e2.write_all(&c).unwrap();
    z.extend(e2.finish().unwrap());

    let raw = zstd::stream::decode_all(&z[..]).unwrap();
    let want = [&a[..], &b[..], &a[..], &c[..]].concat();
    assert!(raw == want, "decoded {} bytes, want {}", raw.len(), want.len());
}
