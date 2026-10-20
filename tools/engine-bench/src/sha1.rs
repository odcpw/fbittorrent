//! Portable SHA-1 (FIPS 180-4). Only used to describe benchmark payloads; the
//! engine under test verifies with its own implementation, so a mismatch fails
//! every piece loudly instead of passing silently.

pub struct Sha1 {
    h: [u32; 5],
    buf: [u8; 64],
    buflen: usize,
    len: u64,
}

impl Default for Sha1 {
    fn default() -> Self {
        Self {
            h: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0],
            buf: [0; 64],
            buflen: 0,
            len: 0,
        }
    }
}

fn compress(h: &mut [u32; 5], block: &[u8]) {
    let mut w = [0u32; 80];
    for (i, word) in w.iter_mut().take(16).enumerate() {
        *word = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }
    let [mut a, mut b, mut c, mut d, mut e] = *h;
    for (i, word) in w.iter().enumerate() {
        let (f, k) = match i {
            0..=19 => ((b & c) | (!b & d), 0x5A827999),
            20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
            40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
            _ => (b ^ c ^ d, 0xCA62C1D6),
        };
        let t = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(k)
            .wrapping_add(*word);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = t;
    }
    for (state, value) in h.iter_mut().zip([a, b, c, d, e]) {
        *state = state.wrapping_add(value);
    }
}

impl Sha1 {
    pub fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        if self.buflen > 0 {
            let take = (64 - self.buflen).min(data.len());
            self.buf[self.buflen..self.buflen + take].copy_from_slice(&data[..take]);
            self.buflen += take;
            data = &data[take..];
            if self.buflen < 64 {
                return;
            }
            let block = self.buf;
            compress(&mut self.h, &block);
            self.buflen = 0;
        }
        let mut blocks = data.chunks_exact(64);
        for block in &mut blocks {
            compress(&mut self.h, block);
        }
        let rest = blocks.remainder();
        self.buf[..rest.len()].copy_from_slice(rest);
        self.buflen = rest.len();
    }

    pub fn finish(mut self) -> [u8; 20] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buflen != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 20];
        for (chunk, word) in out.chunks_exact_mut(4).zip(self.h) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

pub fn digest(data: &[u8]) -> [u8; 20] {
    let mut hash = Sha1::default();
    hash.update(data);
    hash.finish()
}

#[cfg(test)]
mod tests {
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn known_vectors() {
        assert_eq!(hex(&super::digest(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(&super::digest(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        let million = vec![b'a'; 1_000_000];
        assert_eq!(hex(&super::digest(&million)), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
        let mut split = super::Sha1::default();
        for part in million.chunks(997) {
            split.update(part);
        }
        assert_eq!(hex(&split.finish()), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
    }
}
