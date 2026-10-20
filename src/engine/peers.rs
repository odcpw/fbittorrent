//! Expensive peer details are read only on explicit API reads or peer subscriptions.
use super::*;
#[derive(Debug, Clone, Serialize)]
pub struct Peer {
    pub addr: String,
    pub client: Option<String>,
    pub encrypted: bool,
    pub incoming: bool,
    pub downloaded: u64,
    pub uploaded: u64,
    pub seed: bool,
}
impl EngineDispatcher {
    pub(crate) fn peer_snapshot(&self, hash: &str) -> Result<Vec<Peer>> {
        self.runtime.block_on(async {
            let ledger =
                self.observation_ledger().await?;
            if !ledger.records.iter().any(|r| r.hash == hash) {
                bail!("torrent record not found");
            }
            let Some(live) = self.inner.handle(hash).ok().and_then(|h| h.live()) else {
                return Ok(Vec::new());
            };
            let snapshot = live.per_peer_stats_snapshot(Default::default());
            let mut peers: Vec<_> = snapshot
                .peers
                .into_iter()
                .map(|(addr, p)| Peer {
                    addr,
                    client: p.client_name,
                    encrypted: p.encrypted,
                    incoming: p.incoming,
                    downloaded: p.counters.fetched_bytes,
                    uploaded: p.counters.uploaded_bytes,
                    seed: p.seed,
                })
                .collect();
            peers.sort_unstable_by(|a, b| a.addr.cmp(&b.addr));
            Ok(peers)
        })
    }
}
