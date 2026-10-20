//! Deterministic, random-access benchmark payload and its v1 metainfo.
//!
//! Byte `p` of payload `key` is byte `p % 8` (little endian) of
//! `splitmix64(key + (p / 8 + 1) * GOLDEN)`. Any block can be produced without
//! storing the payload, so seeds need no disk and downloads can be verified
//! byte-for-byte without trusting the engine's own hash check.

use crate::bencode::{put_bytes, put_int};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

#[inline]
fn word(key: u64, index: u64) -> u64 {
    let mut z = key.wrapping_add(index.wrapping_add(1).wrapping_mul(GOLDEN));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Fill `out` with payload bytes starting at absolute `offset`.
pub fn fill(key: u64, offset: u64, out: &mut [u8]) {
    let mut pos = offset;
    let mut i = 0;
    while pos % 8 != 0 && i < out.len() {
        out[i] = (word(key, pos / 8) >> (8 * (pos % 8))) as u8;
        pos += 1;
        i += 1;
    }
    let mut index = pos / 8;
    let mut chunks = out[i..].chunks_exact_mut(8);
    for chunk in &mut chunks {
        chunk.copy_from_slice(&word(key, index).to_le_bytes());
        index += 1;
    }
    for (j, byte) in chunks.into_remainder().iter_mut().enumerate() {
        *byte = (word(key, index) >> (8 * j)) as u8;
    }
}

/// Overlay a small real media fixture at offset zero for streaming validation.
pub fn fill_with_prefix(key: u64, offset: u64, out: &mut [u8], prefix: &[u8]) {
    fill(key, offset, out);
    if offset < prefix.len() as u64 {
        let start = offset as usize;
        let len = out.len().min(prefix.len() - start);
        out[..len].copy_from_slice(&prefix[start..start + len]);
    }
}

#[derive(Clone)]
pub struct Spec {
    pub name: String,
    pub size: u64,
    pub piece_len: u32,
    pub key: u64,
    pub private: bool,
    pub prefix: Vec<u8>,
}

impl Spec {
    pub fn piece_count(&self) -> u64 {
        self.size.div_ceil(u64::from(self.piece_len))
    }
}

fn hash_range(spec: &Spec, first: u64, out: &mut [u8]) {
    let mut buf = vec![0u8; spec.piece_len as usize];
    for (n, slot) in out.chunks_exact_mut(20).enumerate() {
        let index = first + n as u64;
        let start = index * u64::from(spec.piece_len);
        let len = (spec.size - start).min(u64::from(spec.piece_len)) as usize;
        fill_with_prefix(spec.key, start, &mut buf[..len], &spec.prefix);
        slot.copy_from_slice(&crate::sha1::digest(&buf[..len]));
    }
}

/// SHA-1 of every piece, computed on all cores and cached by payload identity.
pub fn piece_hashes(spec: &Spec, cache: Option<&Path>) -> std::io::Result<Vec<u8>> {
    let pieces = spec.piece_count() as usize;
    let cache_file: Option<PathBuf> = cache.map(|dir| {
        dir.join(format!("{:016x}-{}-{}-{}.pieces", spec.key, spec.size, spec.piece_len, hex(&crate::sha1::digest(&spec.prefix))))
    });
    if let Some(path) = &cache_file {
        if let Ok(mut file) = std::fs::File::open(path) {
            let mut cached = Vec::new();
            file.read_to_end(&mut cached)?;
            if cached.len() == pieces * 20 {
                return Ok(cached);
            }
        }
    }
    let mut out = vec![0u8; pieces * 20];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(4);
    let per = pieces.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (t, slots) in out.chunks_mut(per * 20).enumerate() {
            scope.spawn(move || hash_range(spec, (t * per) as u64, slots));
        }
    });
    if let Some(path) = cache_file {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let partial = path.with_extension("partial");
        std::fs::write(&partial, &out)?;
        std::fs::rename(partial, path)?;
    }
    Ok(out)
}

pub fn info_bytes(spec: &Spec, pieces: &[u8]) -> Vec<u8> {
    let mut info = Vec::with_capacity(pieces.len() + 128);
    info.push(b'd');
    put_bytes(&mut info, b"length");
    put_int(&mut info, spec.size as i64);
    put_bytes(&mut info, b"name");
    put_bytes(&mut info, spec.name.as_bytes());
    put_bytes(&mut info, b"piece length");
    put_int(&mut info, i64::from(spec.piece_len));
    put_bytes(&mut info, b"pieces");
    put_bytes(&mut info, pieces);
    if spec.private {
        put_bytes(&mut info, b"private");
        put_int(&mut info, 1);
    }
    info.push(b'e');
    info
}

pub fn torrent_bytes(spec: &Spec, info: &[u8], announce: &str) -> Vec<u8> {
    let mut torrent = Vec::with_capacity(info.len() + 256);
    torrent.push(b'd');
    put_bytes(&mut torrent, b"announce");
    put_bytes(&mut torrent, announce.as_bytes());
    put_bytes(&mut torrent, b"info");
    torrent.extend_from_slice(info);
    put_bytes(&mut torrent, b"x-fbt-key");
    put_bytes(&mut torrent, format!("{:016x}", spec.key).as_bytes());
    if !spec.prefix.is_empty() {
        put_bytes(&mut torrent, b"x-fbt-prefix");
        put_bytes(&mut torrent, &spec.prefix);
    }
    torrent.push(b'e');
    torrent
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write the payload as a regular file (for recheck/initial-check benchmarks).
pub fn write_file(path: &Path, size: u64, key: u64, prefix: &[u8]) -> std::io::Result<()> {
    let mut file = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(path)?);
    let mut buf = vec![0u8; 1 << 20];
    let mut offset = 0;
    while offset < size {
        let len = (size - offset).min(buf.len() as u64) as usize;
        fill_with_prefix(key, offset, &mut buf[..len], prefix);
        file.write_all(&buf[..len])?;
        offset += len as u64;
    }
    file.flush()?;
    file.into_inner().map_err(|e| e.into_error())?.sync_all()
}

/// Compare a downloaded file with the generator. Returns the first differing
/// offset, or None when every byte and the length match.
pub fn verify_file(path: &Path, size: u64, key: u64, prefix: &[u8]) -> std::io::Result<Option<u64>> {
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() != size {
        return Ok(Some(file.metadata()?.len().min(size)));
    }
    let mut expected = vec![0u8; 1 << 20];
    let mut actual = vec![0u8; 1 << 20];
    let mut offset = 0;
    while offset < size {
        let len = (size - offset).min(expected.len() as u64) as usize;
        fill_with_prefix(key, offset, &mut expected[..len], prefix);
        file.read_exact(&mut actual[..len])?;
        if expected[..len] != actual[..len] {
            let at = expected.iter().zip(&actual).position(|(a, b)| a != b).unwrap_or(0);
            return Ok(Some(offset + at as u64));
        }
        offset += len as u64;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    #[test]
    fn random_access_matches_sequential() {
        let mut whole = vec![0u8; 4099];
        super::fill(7, 0, &mut whole);
        for start in [0usize, 1, 7, 8, 9, 1000, 4090] {
            for len in [0usize, 1, 5, 8, 9, 3] {
                if start + len > whole.len() {
                    continue;
                }
                let mut part = vec![0u8; len];
                super::fill(7, start as u64, &mut part);
                assert_eq!(part, whole[start..start + len]);
            }
        }
        let mut other = vec![0u8; 64];
        super::fill(8, 0, &mut other);
        assert_ne!(other, whole[..64]);
    }
}
