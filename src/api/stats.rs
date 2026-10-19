//! Session bytes are network bytes, independent of verified on-disk progress.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllTimeTotals {
    pub downloaded: u64,
    pub uploaded: u64,
    pub completed: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionTotals {
    pub downloaded: u64,
    pub uploaded: u64,
    pub started_at: u64,
}
