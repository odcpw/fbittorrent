//! Per-torrent cap syntax. Zero is not a valid finite cap; none explicitly clears.
use crate::agent::{AppError, Code};
pub(super) fn parse(text: &str) -> anyhow::Result<Option<u64>> {
    if text.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let lower = text.to_ascii_lowercase();
    let (digits, scale) = if let Some(v) = lower.strip_suffix('k') {
        (v, 1024)
    } else if let Some(v) = lower.strip_suffix('m') {
        (v, 1024 * 1024)
    } else {
        (lower.as_str(), 1)
    };
    let bps = digits
        .parse::<u64>()
        .ok()
        .and_then(|v| v.checked_mul(scale))
        .filter(|bps| (16384..=u32::MAX as u64).contains(bps))
        .ok_or_else(|| {
            AppError::new(
                Code::Usage,
                "--down must be 16384..4294967295 bytes/s, K/M suffix, or none",
            )
        })?;
    Ok(Some(bps))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn download_limit_syntax() {
        assert_eq!(parse("64K").unwrap(), Some(65536));
        assert_eq!(parse("2m").unwrap(), Some(2097152));
        assert_eq!(parse("none").unwrap(), None);
        for value in ["0", "1", "-1", "4294967296", "NaN", "5000M"] {
            assert!(parse(value).is_err(), "{value}");
        }
    }
}
