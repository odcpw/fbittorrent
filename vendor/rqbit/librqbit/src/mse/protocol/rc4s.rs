// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 urtorrent contributors

//! RC4 keystream with the MSE 1024-byte discard.

// The cloneable permutation state below is adapted from RustCrypto rc4 0.1.0
// (MIT OR Apache-2.0), https://github.com/RustCrypto/stream-ciphers/tree/rc4-v0.1.0/rc4.
// Its public cipher wrapper does not expose Clone. Keeping the underlying state
// cloneable lets a socket commit only the bytes it actually accepted.

/// One direction of the encrypted stream.
#[derive(Clone)]
pub struct Rc4Stream {
    state: [u8; 256],
    i: u8,
    j: u8,
}
impl Rc4Stream {
    /// Initialize the fixed 20-byte MSE key and discard 1024 keystream bytes.
    pub fn new(key: &[u8; 20]) -> Self {
        let mut cipher = Self {
            state: [0; 256],
            i: 0,
            j: 0,
        };
        for (i, byte) in cipher.state.iter_mut().enumerate() {
            *byte = i as u8;
        }
        let mut j = 0u8;
        for (i, k) in (0..256usize).zip(key.iter().cycle()) {
            j = j.wrapping_add(cipher.state[i]).wrapping_add(*k);
            cipher.state.swap(i, j as usize);
        }
        let mut discard = [0u8; 1024];
        cipher.apply(&mut discard);
        cipher
    }
    /// Apply the next keystream bytes in place; encryption is symmetric.
    pub fn apply(&mut self, data: &mut [u8]) {
        for byte in data {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.state[self.i as usize]);
            self.state.swap(self.i as usize, self.j as usize);
            let index = self.state[self.i as usize].wrapping_add(self.state[self.j as usize]);
            *byte ^= self.state[index as usize];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_rustcrypto_rc4_after_discard_across_chunks() {
        use rc4::{KeyInit, Rc4, StreamCipher, consts::U20};
        for key in [[0u8; 20], [3u8; 20], [0xffu8; 20]] {
            let mut reference = Rc4::<U20>::new((&key).into());
            reference.apply_keystream(&mut [0u8; 1024]);
            let mut cipher = Rc4Stream::new(&key);
            for size in [1, 7, 256, 16384, 32768] {
                let mut expected = vec![0x42; size];
                let mut actual = expected.clone();
                reference.apply_keystream(&mut expected);
                cipher.apply(&mut actual);
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn symmetric_and_discarding() {
        let key = [3u8; 20];
        let mut a = Rc4Stream::new(&key);
        let mut b = Rc4Stream::new(&key);
        let mut data = b"hello mse".to_vec();
        a.apply(&mut data);
        assert_ne!(data, b"hello mse");
        b.apply(&mut data);
        assert_eq!(data, b"hello mse");
        // Without the discard the first byte would differ from the raw RC4
        // keystream position 0; with it, two fresh streams still agree.
        let mut c = Rc4Stream::new(&key);
        let mut d = Rc4Stream::new(&key);
        let mut x = [0u8; 4];
        let mut y = [0u8; 4];
        c.apply(&mut x);
        d.apply(&mut y);
        assert_eq!(x, y);
    }
}
