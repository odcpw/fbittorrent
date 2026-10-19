//! Daemon-owned sessions; connection loss starts a reclaimable thirty-second lease.
use super::*;
use crate::api::{
    ops::{error, ApiResult},
    *,
};

pub const GRACE: Duration = Duration::from_secs(30);
struct Session {
    owner: u64,
    expires: Option<Instant>,
    job: Option<i64>,
    limit: usize,
    done_sent: bool,
    cached: Vec<SearchItem>,
    observed: usize,
}
#[derive(Default)]
struct State {
    next: u64,
    sessions: BTreeMap<u64, Session>,
    closed: bool,
}
#[derive(Default)]
pub struct Service {
    state: Mutex<State>,
}

pub(crate) fn missing(id: u64) -> Error {
    let mut e = error(ErrorCode::NotFound, "unknown search session", None);
    e.details = Some(serde_json::Map::from_iter([(
        "search_id".into(),
        id.into(),
    )]));
    e
}
pub(crate) fn upstream(e: anyhow::Error) -> Error {
    let mut failure = error(ErrorCode::Upstream, &format!("{e:#}"), None);
    failure.details = Some(serde_json::Map::from_iter([(
        "source".into(),
        "search".into(),
    )]));
    failure
}
impl Service {
    pub fn start(&self, owner: u64, args: &SearchStartArgs) -> ApiResult<u64> {
        let id = {
            let mut state = self.state.lock().unwrap();
            if state.closed {
                return Err(error(ErrorCode::EngineUnavailable, "daemon draining", None));
            }
            if state.sessions.len() == 4 {
                let mut e = error(ErrorCode::Busy, "search session limit", None);
                e.details = Some(serde_json::Map::from_iter([(
                    "limit".into(),
                    "sessions".into(),
                )]));
                return Err(e);
            }
            state.next += 1;
            let id = state.next;
            state.sessions.insert(
                id,
                Session {
                    owner,
                    expires: None,
                    job: None,
                    limit: args.limit.unwrap_or(5000) as usize,
                    done_sent: false,
                    cached: vec![],
                    observed: 0,
                },
            );
            id
        };
        // Reserve before provider inspection, so concurrent admissions also obey four.
        let result = jobs::start_selected(
            &ProviderStore::default(),
            &args.query,
            args.providers.as_deref(),
        );
        let mut state = self.state.lock().unwrap();
        match result {
            Ok(job) => {
                if let Some(session) = state.sessions.get_mut(&id) {
                    session.job = Some(job);
                    Ok(id)
                } else {
                    drop(state);
                    let _ = delete(job);
                    Err(missing(id))
                }
            }
            Err(e) => {
                state.sessions.remove(&id);
                Err(upstream(e))
            }
        }
    }
    pub fn disconnected(&self, owner: u64) {
        let mut state = self.state.lock().unwrap();
        for session in state.sessions.values_mut().filter(|s| s.owner == owner) {
            session.expires.get_or_insert(Instant::now() + GRACE);
        }
    }
    pub fn reclaim(&self, owner: u64, id: u64) -> ApiResult<()> {
        let mut state = self.state.lock().unwrap();
        let session = state.sessions.get_mut(&id).ok_or_else(|| missing(id))?;
        if session
            .expires
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            return Err(missing(id));
        }
        if session.expires.is_some() {
            session.owner = owner;
            session.expires = None;
        }
        Ok(())
    }
    pub fn stop(&self, id: u64) -> ApiResult<()> {
        let session = self
            .state
            .lock()
            .unwrap()
            .sessions
            .remove(&id)
            .ok_or_else(|| missing(id))?;
        if let Some(job) = session.job {
            delete(job).map_err(upstream)?;
        }
        Ok(())
    }
    pub fn count(&self) -> usize {
        self.state.lock().unwrap().sessions.len()
    }
    pub fn sweep(&self) {
        let expired: Vec<_> = {
            let mut state = self.state.lock().unwrap();
            let ids: Vec<_> = state
                .sessions
                .iter()
                .filter(|(_, s)| s.expires.is_some_and(|d| d <= Instant::now()))
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| state.sessions.remove(&id).and_then(|s| s.job))
                .collect()
        };
        for job in expired {
            let _ = delete(job);
        }
    }
    pub fn items(&self, id: u64) -> ApiResult<Vec<SearchItem>> {
        let mut state = self.state.lock().unwrap();
        let session = state.sessions.get_mut(&id).ok_or_else(|| missing(id))?;
        if let Some(id) = session.job {
            let job = jobs::job(id).map_err(upstream)?;
            let rows = job.state.lock().unwrap();
            if rows.results.len() != session.observed {
                session.cached =
                    normalize::aggregate_with_hashes(&rows.results, session.limit, &rows.hashes);
                session.observed = rows.results.len();
            }
        }
        Ok(session.cached.clone())
    }
    pub fn completed(&self) -> Vec<serde_json::Value> {
        let mut state = self.state.lock().unwrap();
        let mut events = Vec::new();
        for (id, session) in &mut state.sessions {
            let Some(job) = session.job.and_then(|id| jobs::job(id).ok()) else {
                continue;
            };
            if !session.done_sent && job.done.load(Ordering::Acquire) {
                session.done_sent = true;
                let rows = job.state.lock().unwrap();
                let warnings: Vec<_> = (!rows.error.is_empty())
                    .then(|| crate::api::ProviderWarning {
                        provider: "nova".into(),
                        message: rows.error.chars().take(200).collect(),
                    })
                    .into_iter()
                    .collect();
                events.push(serde_json::json!({"ev":"search.done","search_id":id,
                    "total":normalize::aggregate_with_hashes(&rows.results,session.limit,&rows.hashes).len(),"warnings":warnings}));
            }
        }
        events
    }
    pub fn shutdown(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.sessions.clear();
        drop(state);
        shutdown_all();
    }
}
