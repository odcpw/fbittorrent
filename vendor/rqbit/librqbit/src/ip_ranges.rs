use anyhow::{Context, Result};
use intervaltree::IntervalTree;
use std::iter::empty;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::ops::Range;
use std::str::FromStr;
use rqbit_runtime::{Cx, http::Client};
use std::io::Read;
use url::Url;

struct IntervalTreeWithSize<T> {
    t: IntervalTree<T, ()>,
    len: usize,
}

fn interval_tree<T: Clone + Ord>(it: impl Iterator<Item = Range<T>>) -> IntervalTreeWithSize<T> {
    let mut len = 0;
    let t = IntervalTree::from_iter(it.map(|r| {
        len += 1;
        (r, ())
    }));
    IntervalTreeWithSize { t, len }
}

pub struct IpRanges {
    // We could store only one interval tree, but splitting them takes less memory,
    // as IpAddr is 17 bytes, Ipv4Addr is only 4 bytes (the majority of ranges).
    v4: IntervalTreeWithSize<Ipv4Addr>,
    v6: IntervalTreeWithSize<Ipv6Addr>,
}

impl Default for IpRanges {
    fn default() -> Self {
        Self {
            v4: interval_tree(empty()),
            v6: interval_tree(empty()),
        }
    }
}

impl IpRanges {
    pub fn new(
        v4_ranges: impl IntoIterator<Item = Range<Ipv4Addr>>,
        v6_ranges: impl IntoIterator<Item = Range<Ipv6Addr>>,
    ) -> Self {
        Self {
            v4: interval_tree(v4_ranges.into_iter()),
            v6: interval_tree(v6_ranges.into_iter()),
        }
    }

    pub fn len(&self) -> usize {
        self.v4.len + self.v6.len
    }

    pub async fn load_from_url(cx: &Cx, url: &str) -> Result<Self> {
        Self::load_from_url_with_client(cx, url, &Client::new(crate::client_name_and_version())).await
    }

    pub async fn load_from_url_with_client(cx: &Cx, url: &str, client: &Client) -> Result<Self> {
        cx.checkpoint()?;
        let parsed_url = Url::parse(url).context("failed to parse URL")?;
        let bytes = if parsed_url.scheme() == "file" {
            let path = parsed_url.to_file_path().ok().context("invalid file URL")?;
            rqbit_runtime::fs::read(path).await?
        } else {
            client.get(cx, url).await?
        };
        Self::from_bytes(cx, &bytes)
    }

    fn from_bytes(cx: &Cx, bytes: &[u8]) -> Result<Self> {
        const MAX_DECODED_BYTES: u64 = 32 * 1024 * 1024;
        let decoded;
        let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut decoder = flate2::read::GzDecoder::new(bytes).take(MAX_DECODED_BYTES + 1);
            let mut data = Vec::new();
            decoder.read_to_end(&mut data)?;
            anyhow::ensure!(data.len() as u64 <= MAX_DECODED_BYTES, "IP list exceeds size limit");
            decoded = data;
            &decoded[..]
        } else { bytes };
        let text = std::str::from_utf8(bytes).context("IP list is not UTF-8")?;
        let mut v4 = Vec::new();
        let mut v6 = Vec::new();
        for line in text.lines() {
            cx.checkpoint()?;
            match parse_ip_range(line) {
                Some(IpRange::V4(r)) => v4.push(r),
                Some(IpRange::V6(r)) => v6.push(r),
                None => {},
            }
        }
        Ok(Self::new(v4,v6))
    }

    pub fn has(&self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(a) => self.v4.t.query_point(a).next().is_some(),
            IpAddr::V6(a) => self.v6.t.query_point(a).next().is_some(),
        }
    }
}

enum IpRange {
    V4(Range<Ipv4Addr>),
    V6(Range<Ipv6Addr>),
}

fn parse_ip_range(line: &str) -> Option<IpRange> {
    let line = line.trim();
    if line.starts_with('#') || line.is_empty() {
        return None;
    }

    let (_name, ips) = {
        let is_ipv6 = line.matches(":").count() > 2;
        if is_ipv6 {
            line.split_once(':')?
        } else {
            line.rsplit_once(':')?
        }
    };
    let (start, end) = ips.split_once('-')?;
    match (IpAddr::from_str(start).ok()?, IpAddr::from_str(end).ok()?) {
        (IpAddr::V4(start), IpAddr::V4(end)) => {
            let end = Ipv4Addr::from_bits(end.to_bits().saturating_add(1));
            Some(IpRange::V4(start..end))
        }
        (IpAddr::V6(start), IpAddr::V6(end)) => {
            let end = Ipv6Addr::from_bits(end.to_bits().saturating_add(1));
            Some(IpRange::V6(start..end))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inclusive_range_and_comments() {
        assert!(parse_ip_range("# ignored").is_none());
        let Some(IpRange::V4(range)) = parse_ip_range("local:192.168.1.1-192.168.1.255") else { panic!("range"); };
        let list = IpRanges::new([range], []);
        assert!(list.has("192.168.1.255".parse().unwrap()));
        assert!(!list.has("192.168.2.0".parse().unwrap()));
    }
}
