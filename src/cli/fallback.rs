//! In-process catalogue sources. These never launch the torrent daemon.
use crate::{api, config::Config, search};
use anyhow::Result;
use std::time::{Duration, Instant};

pub(crate) struct Search {
    pub items: Vec<api::SearchItem>,
    pub complete: bool,
    pub warnings: Vec<api::ProviderWarning>,
}
struct Job(i64);
impl Drop for Job {
    fn drop(&mut self) {
        let _ = search::delete(self.0);
    }
}

pub(crate) fn search(query: &str, wait: u64) -> Result<Search> {
    let job = Job(search::start(query)?);
    let deadline = Instant::now() + Duration::from_secs(wait);
    loop {
        let (items, complete, warnings) = search::normalized_snapshot(job.0)?;
        if complete || Instant::now() >= deadline {
            return Ok(Search {
                items,
                complete,
                warnings,
            });
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(25)),
        );
    }
}

pub(crate) fn install(cfg: &Config, sources: &[String]) -> api::InstalledResult {
    let mut result = api::InstalledResult {
        installed: vec![],
        failed: vec![],
    };
    for source in sources {
        match search::install(
            std::slice::from_ref(source),
            cfg.timeout_secs.min(10).max(1),
        ) {
            Ok(()) => result.installed.push(source.clone()),
            Err(error) => result.failed.push(api::InstallFailure {
                source: search::torznab::public_source(source),
                message: format!("{error:#}").chars().take(200).collect(),
            }),
        }
    }
    result
}

pub(crate) fn imdb(query: &str, refresh: bool) -> Result<Option<api::ImdbCard>> {
    let args = super::catalog_imdb::arguments(query, refresh);
    let api::Reply::ImdbLookup(result) =
        api::ops::imdb::lookup(&crate::imdb::service::Service::default(), &args)
            .map_err(api::errors::app_error)?
    else {
        unreachable!("IMDb reply")
    };
    Ok(result.card)
}
