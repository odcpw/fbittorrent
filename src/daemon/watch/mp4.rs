//! Follow ISO BMFF box sizes, never search media bytes for a `moov` string.
//! At most 512 headers (8 KiB) and one 48-byte mvhd are read per inspection.
use std::{
    fs::{File, OpenOptions},
    ops::Range,
    os::unix::fs::{FileExt, OpenOptionsExt},
    path::Path,
};

const HEADERS: usize = 512;

#[derive(Clone, Default)]
pub(super) struct Movie {
    range: Option<Range<u64>>,
    validated: bool,
    duration: Option<f64>,
}
struct Atom {
    range: Range<u64>,
    body: u64,
    kind: [u8; 4],
}
struct Reader<'a, F> {
    file: &'a File,
    have: F,
    left: usize,
}
impl<F: Fn(Range<u64>) -> bool> Reader<'_, F> {
    fn atom(&mut self, at: u64, end: u64) -> Option<Atom> {
        self.left = self.left.checked_sub(1)?;
        let mut header = [0; 16];
        let mut body = at.checked_add(8)?;
        if body > end || !(self.have)(at..body) {
            return None;
        }
        self.file.read_exact_at(&mut header[..8], at).ok()?;
        let mut size = u64::from(u32::from_be_bytes(header[..4].try_into().ok()?));
        if size == 1 {
            body = at.checked_add(16)?;
            if body > end || !(self.have)(at..body) {
                return None;
            }
            self.file.read_exact_at(&mut header[8..], at + 8).ok()?;
            size = u64::from_be_bytes(header[8..].try_into().ok()?);
        } else if size == 0 {
            size = end.checked_sub(at)?;
        }
        let stop = at.checked_add(size)?;
        if stop < body || stop > end {
            return None;
        }
        Some(Atom {
            range: at..stop,
            body,
            kind: header[4..8].try_into().ok()?,
        })
    }
    fn movie(&mut self, size: u64) -> Option<Range<u64>> {
        let mut at = 0;
        while at < size {
            let atom = self.atom(at, size)?;
            if &atom.kind == b"moov" {
                return Some(atom.range);
            }
            at = atom.range.end;
        }
        None
    }
    fn validate(&mut self, range: Range<u64>) -> Option<Option<f64>> {
        let movie = self.atom(range.start, range.end)?;
        if &movie.kind != b"moov" || movie.range != range {
            return None;
        }
        let mut at = movie.body;
        let mut found = false;
        let mut duration = None;
        while at < range.end {
            let atom = self.atom(at, range.end)?;
            if &atom.kind == b"mvhd" {
                if found {
                    return None;
                }
                // mvhd is required in a moov. Read only its fixed duration
                // fields; tracks/sample tables may be many megabytes long.
                let mut bytes = [0; 48];
                let n = usize::try_from((atom.range.end - at).min(48)).ok()?;
                self.file.read_exact_at(&mut bytes[..n], at).ok()?;
                let header = usize::try_from(atom.body - at).ok()?;
                let body = &bytes[header..n];
                let required = match body.first()? {
                    0 => 20,
                    1 => 32,
                    _ => return None,
                };
                if body.len() < required {
                    return None;
                }
                duration = super::duration::parse(&bytes[..n]);
                found = true;
            }
            at = atom.range.end;
        }
        found.then_some(duration)
    }
}
impl Movie {
    pub(super) fn duration(&self) -> Option<f64> {
        self.duration
    }

    pub(super) fn inspect(
        &mut self,
        path: &Path,
        size: u64,
        have: impl Fn(Range<u64>) -> bool,
    ) -> bool {
        if let Some(range) = &self.range {
            if !have(range.clone()) {
                return false;
            }
            if self.validated {
                return true;
            }
        } else if size < 8 || !have(0..8) {
            return false;
        }
        let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        else {
            return false;
        };
        if !file
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() >= size)
        {
            return false;
        }
        let mut reader = Reader {
            file: &file,
            have,
            left: HEADERS,
        };
        let range = match self.range.clone().or_else(|| reader.movie(size)) {
            Some(range) => range,
            None => return false,
        };
        self.range = Some(range.clone());
        if !(reader.have)(range.clone()) {
            return false;
        }
        let Some(duration) = reader.validate(range) else {
            return false;
        };
        self.duration = duration;
        self.validated = true;
        true
    }
}

#[cfg(test)]
#[path = "mp4_tests.rs"]
mod tests;
