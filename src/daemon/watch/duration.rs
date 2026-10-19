//! Bounded container header metadata; malformed or absent duration stays unknown.
use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path};
pub(super) fn read(path: &Path, limit: u64) -> Option<f64> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    f.take(limit.min(super::MIB)).read_to_end(&mut bytes).ok()?;
    parse(&bytes)
}
pub(super) fn parse(bytes: &[u8]) -> Option<f64> {
    let value = if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        mkv(bytes)
    } else {
        mp4(bytes, 0)
    }?;
    (value.is_finite() && value > 0.0 && value <= 365.0 * 86400.0).then_some(value)
}
fn vint(bytes: &[u8], id: bool) -> Option<(u64, usize)> {
    let first = *bytes.first()?;
    let length = first.leading_zeros() as usize + 1;
    if length > 8 || bytes.len() < length || (id && length > 4) {
        return None;
    }
    let mut v = if id {
        u64::from(first)
    } else {
        u64::from(first & (0xffu8.checked_shr(length as u32).unwrap_or(0)))
    };
    for b in &bytes[1..length] {
        v = v.checked_mul(256)?.checked_add(u64::from(*b))?;
    }
    Some((v, length))
}
fn mkv(bytes: &[u8]) -> Option<f64> {
    fn walk(
        mut bytes: &[u8],
        depth: u8,
        scale: &mut f64,
        duration: &mut Option<f64>,
    ) -> Option<()> {
        if depth > 8 {
            return None;
        }
        while !bytes.is_empty() {
            let (id, n) = vint(bytes, true)?;
            let (size, m) = vint(bytes.get(n..)?, false)?;
            let payload = bytes.get(n + m..)?;
            let size = usize::try_from(size).ok()?;
            let data = &payload[..size.min(payload.len())];
            match id {
                0x18538067 | 0x1549a966 => {
                    walk(data, depth + 1, scale, duration)?;
                    if duration.is_some() {
                        return Some(());
                    }
                }
                0x2ad7b1 if size <= 8 && size > 0 => {
                    *scale = data.iter().fold(0u64, |n, b| n * 256 + u64::from(*b)) as f64;
                }
                0x4489 if size == 4 => {
                    *duration = Some(f32::from_be_bytes(data.try_into().ok()?) as f64);
                }
                0x4489 if size == 8 => {
                    *duration = Some(f64::from_be_bytes(data.try_into().ok()?));
                }
                _ => {}
            }
            if size >= payload.len() {
                break;
            }
            bytes = &payload[size..];
        }
        Some(())
    }
    let mut scale = 1_000_000.0;
    let mut duration = None;
    walk(bytes, 0, &mut scale, &mut duration)?;
    Some(duration? * scale / 1_000_000_000.0)
}
fn mp4(mut bytes: &[u8], depth: u8) -> Option<f64> {
    if depth > 4 {
        return None;
    }
    while bytes.len() >= 8 {
        let mut size = u64::from(u32::from_be_bytes(bytes[..4].try_into().ok()?));
        let kind = &bytes[4..8];
        let mut header = 8;
        if size == 1 {
            size = u64::from_be_bytes(bytes.get(8..16)?.try_into().ok()?);
            header = 16;
        }
        if size == 0 {
            size = bytes.len() as u64;
        }
        let size = usize::try_from(size).ok()?;
        if size < header {
            return None;
        }
        let body = &bytes[header..bytes.len().min(size)];
        if kind == b"moov" {
            if let Some(d) = mp4(body, depth + 1) {
                return Some(d);
            }
        }
        if kind == b"mvhd" {
            let version = *body.first()?;
            let (scale, duration) = match version {
                0 => (
                    u32::from_be_bytes(body.get(12..16)?.try_into().ok()?),
                    u64::from(u32::from_be_bytes(body.get(16..20)?.try_into().ok()?)),
                ),
                1 => (
                    u32::from_be_bytes(body.get(20..24)?.try_into().ok()?),
                    u64::from_be_bytes(body.get(24..32)?.try_into().ok()?),
                ),
                _ => return None,
            };
            return (scale > 0).then_some(duration as f64 / scale as f64);
        }
        if size >= bytes.len() {
            break;
        }
        bytes = &bytes[size..];
    }
    None
}
