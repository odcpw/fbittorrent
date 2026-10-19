//! Native Torznab requests; private keys never enter argv or public diagnostics.
mod bound;
mod config;
#[cfg(test)]
mod tests;
mod worker;
mod xml;
use super::*;
pub(crate) use config::{indexers, install, Indexer, Invalid};
pub(crate) const KEY_TOKEN: &str = "__FBT_TORZNAB_KEY__";
const RESPONSE_CAP: u64 = 4 * 1024 * 1024;
pub(crate) use worker::run_if_requested as run_worker;

/// Diagnostics may identify the failed endpoint but never echo inline credentials.
pub(crate) fn public_source(source: &str) -> String {
    worker::redact_url(source, None)
}

fn exchange(
    request: &worker::Request,
    job: Option<&jobs::SearchJob>,
    cap: usize,
) -> Result<worker::Reply> {
    let mut child = worker::spawn(request)?;
    let pid = child.pid;
    if let Some(job) = job {
        let mut groups = job.native.lock().unwrap();
        if job.cancelled.load(Ordering::Acquire) {
            child.kill();
            bail!("search was cancelled");
        }
        groups.insert(pid);
    }
    struct Remove<'a>(Option<&'a jobs::SearchJob>, i32);
    impl Drop for Remove<'_> {
        fn drop(&mut self) {
            if let Some(job) = self.0 {
                job.native.lock().unwrap().remove(&self.1);
            }
        }
    }
    let _remove = Remove(job, pid);
    let stdout = child.child.stdout.take().unwrap();
    let stderr = child.child.stderr.take().unwrap();
    let output = thread::spawn(move || drain_bounded(&mut BufReader::new(stdout), cap));
    let errors = thread::spawn(move || {
        let _ = drain_bounded(&mut BufReader::new(stderr), 1024);
    });
    let began = Instant::now();
    let status = loop {
        if let Some(status) = child.child.try_wait()? {
            break Some(status);
        }
        if began.elapsed() > Duration::from_secs(32)
            || job.is_some_and(|j| j.cancelled.load(Ordering::Acquire))
        {
            child.kill();
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    child.kill();
    let _ = child.child.wait();
    let _ = errors.join();
    let (bytes, exceeded) = output
        .join()
        .map_err(|_| anyhow!("Torznab result reader failed"))??;
    anyhow::ensure!(!exceeded, "Torznab result output cap exceeded");
    anyhow::ensure!(
        status.is_some_and(|s| s.success()),
        "Torznab worker did not finish"
    );
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("Torznab result is invalid"))
}
pub(super) fn query(indexer: Indexer, query: &str, job: &jobs::SearchJob) {
    let request = worker::Request {
        name: indexer.name.clone(),
        query: Some(query.into()),
        download: None,
    };
    let result = exchange(&request, Some(job), MAX_CAPTURE_BYTES);
    if job.cancelled.load(Ordering::Acquire) {
        return;
    }
    let mut state = job.state.lock().unwrap();
    match result {
        Ok(reply) if reply.error.is_none() => {
            let space = MAX_RESULTS.saturating_sub(state.results.len());
            state.results.extend(reply.rows.into_iter().take(space));
        }
        result => {
            let message = match result {
                Ok(reply) => reply
                    .error
                    .unwrap_or_else(|| "Torznab request failed".into()),
                Err(_) => "Torznab request failed".into(),
            };
            state.warnings.push(crate::api::ProviderWarning {
                provider: indexer.name,
                message,
            });
        }
    }
}
pub(crate) fn resolve(result: &SearchResult) -> Result<Option<ResolvedDownload>> {
    if result.file_url.starts_with("magnet:") {
        return Ok(None);
    }
    let configured = indexers()?;
    let endpoint = url::Url::parse(&result.file_url).ok();
    let selected = configured
        .iter()
        .find(|i| i.name == result.engine)
        .or_else(|| {
            endpoint.as_ref().and_then(|target| {
                configured
                    .iter()
                    .find(|i| url::Url::parse(&i.url).is_ok_and(|u| u.origin() == target.origin()))
            })
        });
    let Some(indexer) = selected else {
        return Ok(None);
    };
    let request = worker::Request {
        name: indexer.name.clone(),
        query: None,
        download: Some(result.file_url.clone()),
    };
    let reply = exchange(&request, None, 24 * 1024 * 1024)?;
    if let Some(error) = reply.error {
        bail!("{error}");
    }
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(reply.torrent.context("Torznab download omitted metainfo")?)
        .map_err(|_| anyhow!("Torznab download encoding failed"))?;
    Ok(Some(ResolvedDownload::Torrent {
        filename: "torznab.torrent".into(),
        bytes,
    }))
}
