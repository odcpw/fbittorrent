//! Peer wire data and bounded one-second counter baselines, owned by the daemon.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub addr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    pub conn: PeerConnection,
    pub incoming: bool,
    pub state: String,
    pub downloaded: u64,
    pub uploaded: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dl_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ul_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<bool>,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerConnection {
    Tcp,
    TcpMse,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeersResult {
    pub peers: Vec<Peer>,
}
struct Baseline {
    at: Instant,
    rows: BTreeMap<String, (u64, u64)>,
    rates: BTreeMap<String, (Option<u64>, Option<u64>)>,
}
#[derive(Default)]
pub(crate) struct Sampler(Mutex<BTreeMap<String, Baseline>>);
impl Sampler {
    pub fn retain(&self, topics: &[Topic]) {
        self.retain_if(|hash| {
            topics
                .iter()
                .any(|t| matches!(t,Topic::Peers(h) if h.as_str()==hash))
        });
    }
    pub fn retain_if(&self, keep: impl Fn(&str) -> bool) {
        self.0.lock().unwrap().retain(|hash, _| keep(hash));
    }
    pub fn read(&self, daemon: &crate::daemon::Daemon, hash: &Hash) -> ops::ApiResult<Vec<Peer>> {
        let crate::engine::ops::EngineResult::Peers(raw) = daemon
            .execute(crate::engine::ops::EngineOp::Peers {
                hash: hash.to_string(),
            })
            .map_err(|e| {
                let mut failure = ops::engine_error(e);
                if failure.code == ErrorCode::NotFound {
                    failure.details = Some(serde_json::Map::from_iter([(
                        "hash".into(),
                        serde_json::json!(hash),
                    )]));
                }
                failure
            })?
        else {
            unreachable!("typed peer result")
        };
        Ok(self.sample(hash.as_str(), raw, Instant::now()))
    }
    fn sample(&self, hash: &str, raw: Vec<crate::engine::peers::Peer>, now: Instant) -> Vec<Peer> {
        let mut cache = self.0.lock().unwrap();
        cache.retain(|_, b| now.duration_since(b.at) < Duration::from_secs(10));
        if cache.values().map(|b| b.rows.len()).sum::<usize>() + raw.len() > 8192 {
            cache.clear();
        }
        if !cache.contains_key(hash) && cache.len() >= 512 {
            if let Some(key) = cache
                .iter()
                .min_by_key(|(_, b)| b.at)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&key);
            }
        }
        let cacheable = raw.len() <= 8192;
        let b = cache.entry(hash.into()).or_insert_with(|| Baseline {
            at: now,
            rows: BTreeMap::new(),
            rates: BTreeMap::new(),
        });
        let elapsed = now.duration_since(b.at).as_secs_f64();
        if elapsed >= 1.0 {
            b.rates = raw
                .iter()
                .map(|p| {
                    let rates = b
                        .rows
                        .get(&p.addr)
                        .map(|(d, u)| {
                            (
                                p.downloaded
                                    .checked_sub(*d)
                                    .map(|v| (v as f64 / elapsed) as u64),
                                p.uploaded
                                    .checked_sub(*u)
                                    .map(|v| (v as f64 / elapsed) as u64),
                            )
                        })
                        .unwrap_or((None, None));
                    (p.addr.clone(), rates)
                })
                .collect();
            b.at = now;
            b.rows = raw
                .iter()
                .map(|p| (p.addr.clone(), (p.downloaded, p.uploaded)))
                .collect();
        } else if b.rows.is_empty() {
            b.rows = raw
                .iter()
                .map(|p| (p.addr.clone(), (p.downloaded, p.uploaded)))
                .collect();
        }
        b.rows.retain(|addr, _| raw.iter().any(|p| &p.addr == addr));
        let rows = &b.rows;
        b.rates.retain(|addr, _| rows.contains_key(addr));
        let result = raw
            .into_iter()
            .map(|p| {
                let (dl_bps, ul_bps) = b.rates.get(&p.addr).copied().unwrap_or((None, None));
                Peer {
                    addr: p.addr,
                    client: p.client,
                    conn: if p.encrypted {
                        PeerConnection::TcpMse
                    } else {
                        PeerConnection::Tcp
                    },
                    incoming: p.incoming,
                    state: "live".into(),
                    downloaded: p.downloaded,
                    uploaded: p.uploaded,
                    dl_bps,
                    ul_bps,
                    seed: Some(p.seed),
                }
            })
            .collect();
        if !cacheable {
            cache.remove(hash);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn peer(bytes: u64) -> crate::engine::peers::Peer {
        crate::engine::peers::Peer {
            addr: "127.0.0.1:1234".into(),
            client: None,
            encrypted: false,
            incoming: false,
            downloaded: bytes,
            uploaded: bytes / 2,
            seed: true,
        }
    }
    #[test]
    fn peer_rates_wait_one_second_and_reset_or_disconnect_removes_baseline() {
        let s = Sampler::default();
        let now = Instant::now();
        let hash = "12".repeat(20);
        assert_eq!(s.sample(&hash, vec![peer(100)], now)[0].dl_bps, None);
        assert_eq!(
            s.sample(&hash, vec![peer(200)], now + Duration::from_millis(500))[0].dl_bps,
            None
        );
        let row = s
            .sample(&hash, vec![peer(1100)], now + Duration::from_secs(1))
            .remove(0);
        assert_eq!((row.dl_bps, row.ul_bps), (Some(1000), Some(500)));
        let row = s
            .sample(&hash, vec![peer(0)], now + Duration::from_secs(2))
            .remove(0);
        assert_eq!((row.dl_bps, row.ul_bps), (None, None));
        s.retain(&[Topic::Peers(Hash::new(&hash).unwrap())]);
        assert_eq!(s.0.lock().unwrap().len(), 1);
        s.retain(&[]);
        assert!(s.0.lock().unwrap().is_empty());
        assert_eq!(
            s.sample(&hash, vec![peer(10000)], now + Duration::from_secs(3))[0].dl_bps,
            None
        );
    }
}
