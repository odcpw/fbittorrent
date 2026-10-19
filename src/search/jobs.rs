//! Search job registration, streaming results, cancellation and drain.
use super::*;

pub(super) struct JobState {
    pub(super) results: Vec<SearchResult>,
    pub(super) raw: Vec<u8>,
    pub(super) hashes: BTreeMap<String,String>,
    pub(super) error: String,
    pub(super) warnings:Vec<crate::api::ProviderWarning>,
}
pub(super) struct SearchJob {
    pub(super) pid: AtomicI32,
    pub(super) done: AtomicBool,
    pub(super) cancelled: AtomicBool,
    pub(super) native:Mutex<BTreeSet<i32>>,
    pub(super) state: Mutex<JobState>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
#[derive(Default)]
pub(super) struct SearchRegistry {
    pub(super) closing: bool,
    pub(super) processes: BTreeSet<i32>,
    pub(super) jobs: BTreeMap<i64, Arc<SearchJob>>,
}
static JOBS: OnceLock<Mutex<SearchRegistry>> = OnceLock::new();
static NEXT_ID: AtomicI64 = AtomicI64::new(1);

pub(super) fn jobs() -> &'static Mutex<SearchRegistry> {
    JOBS.get_or_init(|| Mutex::new(SearchRegistry::default()))
}

pub fn begin_session() -> Result<()> {
    jobs()
        .lock()
        .map_err(|_| anyhow!("search lifecycle lock failed"))?
        .closing = false;
    Ok(())
}

pub fn start(query: &str) -> Result<i64> {
    start_selected(&ProviderStore::default(), query,None)
}

pub(super) fn start_in(store: &ProviderStore, query: &str) -> Result<i64> {
    start_with(store,query,None,Vec::new())
}

pub(super) fn start_selected(store: &ProviderStore, query: &str, selected: Option<&[String]>) -> Result<i64> {
    let preferences=store.load()?;
    let native=torznab::indexers()?.into_iter().filter(|i|preferences.enabled.get(&i.name).copied().unwrap_or(true) && selected.is_none_or(|names|names.contains(&i.name))).collect();
    start_with(store,query,selected,native)
}
fn start_with(store:&ProviderStore,query:&str,selected:Option<&[String]>,native:Vec<torznab::Indexer>)->Result<i64> {
    if query.trim().is_empty() {
        bail!("no search query given");
    }
    if jobs()
        .lock()
        .map_err(|_| anyhow!("search lifecycle lock failed"))?
        .closing
    {
        bail!("the app is closing; search providers cannot start");
    }
    let mut providers: Vec<_> = plugins_in(store)?
        .into_iter()
        .filter(|plugin| plugin.enabled && selected.is_none_or(|names| names.contains(&plugin.name)))
        .collect();
    if providers.is_empty() && native.is_empty() {
        bail!("no enabled search providers; install providers with fbittorrent search --install");
    }
    // Python providers open their own, unbound sockets: under a VPN binding
    // they are skipped (Torznab indexers still run bound) or refused.
    let mut warnings = Vec::new();
    if !providers.is_empty() {
        if let Err(error) = crate::outbound::unbound_helpers_allowed("Nova search providers") {
            if native.is_empty() { return Err(error); }
            warnings.push(crate::api::ProviderWarning { provider: "nova".into(), message: format!("{error:#}").chars().take(200).collect() });
            providers.clear();
        }
    }
    let process=if providers.is_empty(){None}else {
        let names=providers.iter().map(|p|p.name.as_str()).collect::<Vec<_>>().join(",");
        let mut command=python(&store.runtime());command.arg("nova2.py").arg(names).arg("all").arg(query);
        Some(ProcessGroup::spawn(command)?)
    };
    let job = Arc::new(SearchJob {
        pid: AtomicI32::new(process.as_ref().map_or(0,|p|p.pid)),
        native:Mutex::new(BTreeSet::new()),
        done: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        state: Mutex::new(JobState {
            results: vec![],
            raw: vec![],
            hashes: BTreeMap::new(),
            error: String::new(),
            warnings,
        }),
        worker: Mutex::new(None),
    });
    let worker_job = job.clone();
    let query=query.to_owned();
    let worker = thread::Builder::new()
        .name("torrent-search".into())
        .spawn(move || {
            let natives:Vec<_>=native.into_iter().map(|indexer| {
                let job=worker_job.clone();let query=query.clone();
                thread::spawn(move ||torznab::query(indexer,&query,&job))
            }).collect();
            if let Some(process)=process {run_nova(process,providers,&worker_job);}
            for task in natives {let _=task.join();}
            worker_job.done.store(true, Ordering::Release);
        })?;
    *job.worker
        .lock()
        .map_err(|_| anyhow!("search worker lock failed"))? = Some(worker);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut registry = jobs()
        .lock()
        .map_err(|_| anyhow!("search job lock failed"))?;
    if registry.closing {
        drop(registry);
        stop_job(&job);
        bail!("the app closed while search providers were starting");
    }
    registry.jobs.insert(id, job);
    Ok(id)
}

/// A whole Nova search (every provider) must finish within this time; a
/// stalled provider would otherwise hold its process group and session open.
#[cfg(not(test))]
fn nova_deadline() -> Duration { Duration::from_secs(120) }
#[cfg(test)]
pub(super) static TEST_NOVA_DEADLINE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(120_000);
#[cfg(test)]
fn nova_deadline() -> Duration { Duration::from_millis(TEST_NOVA_DEADLINE_MS.load(Ordering::Acquire)) }

fn run_nova(mut process:ProcessGroup,providers:Vec<SearchPlugin>,job:&SearchJob) {
    let stdout=process.child.stdout.take().unwrap();let stderr=process.child.stderr.take().unwrap();
    // The reader below blocks on provider output; a watchdog enforces the
    // deadline by stopping the whole group, which closes that output.
    let (finished, watched) = std::sync::mpsc::channel::<()>();
    let group = process.pid;
    let deadline = nova_deadline();
    let watchdog = thread::spawn(move || {
        let expired = matches!(watched.recv_timeout(deadline), Err(std::sync::mpsc::RecvTimeoutError::Timeout));
        if expired { unsafe { libc::kill(-group, libc::SIGKILL); } }
        expired
    });
            let errors = thread::spawn(move || {
                drain_bounded(&mut BufReader::new(stderr), 16 * 1024)
                    .map(|(bytes,_)| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default()
            });
            let mut reader = BufReader::new(stdout);
            let mut retained = 0usize;
            let mut inspected = BTreeMap::<String, Option<String>>::new();
            let mut line = Vec::new();
            loop {
                line.clear();
                let budget = MAX_CAPTURE_BYTES.saturating_sub(retained);
                let count = reader.by_ref().take(budget as u64 + 1).read_until(b'\n', &mut line).unwrap_or(0);
                if count == 0 { break; }
                if count > budget {
                    if let Ok(mut state) = job.state.lock() { state.error = "search provider output exceeded 2 MiB".into(); }
                    process.kill(); break;
                }
                retained += count;
                let result = std::str::from_utf8(&line).ok().and_then(|text| parse_result(text, &providers));
                let hash = result.as_ref().and_then(|r| inspected.entry(r.file_url.clone()).or_insert_with(|| normalize::torrent_hash(&r.file_url)).clone());
                if let Ok(mut state) = job.state.lock() {
                    state.raw.extend_from_slice(&line);
                    if let Some(result) = result {
                        if let Some(hash) = hash { state.hashes.insert(result.file_url.clone(), hash); }
                        if state.results.len() < MAX_RESULTS { state.results.push(result); }
                    }
                }
            }
            // Stop the watchdog while the group leader is still unreaped.
            drop(finished);
            let expired = watchdog.join().unwrap_or(false);
            let status = process.child.wait();
            process.kill();
            job.pid.store(0, Ordering::Release);
            let errors = errors.join().unwrap_or_default();
            if expired && !job.cancelled.load(Ordering::Acquire) {
                if let Ok(mut state) = job.state.lock() {
                    if state.error.is_empty() { state.error = format!("search providers timed out after {} seconds", deadline.as_secs()); }
                }
            }
            if !job.cancelled.load(Ordering::Acquire)
                && !status.as_ref().is_ok_and(|status| status.success())
            {
                if let Ok(mut state) = job.state.lock() {
                    if state.error.is_empty() { state.error = if errors.trim().is_empty() {
                        format!("search providers failed to finish: {}",match &status {
                            Ok(status)=>status.to_string(),Err(error)=>error.to_string(),
                        })
                    } else {
                        errors.trim().into()
                    }; }
                }
            }

}

pub(super) fn warnings(state:&JobState)->Vec<crate::api::ProviderWarning> {
    let mut warnings=state.warnings.clone();
    if !state.error.is_empty(){warnings.push(crate::api::ProviderWarning{provider:"nova".into(),message:state.error.chars().take(200).collect()});}
    warnings
}

pub(super) fn parse_result(line: &str, providers: &[SearchPlugin]) -> Option<SearchResult> {
    let fields: Vec<_> = line.trim_end_matches(['\r', '\n']).split('|').collect();
    if fields.len() < 6 || fields[0].is_empty() || fields[1].is_empty() {
        return None;
    }
    let engine = providers
        .iter()
        .find(|plugin| plugin.url.trim_end_matches('/') == fields[5].trim_end_matches('/'))
        .map(|plugin| plugin.name.clone())
        .unwrap_or_else(|| fields[5].to_string());
    Some(SearchResult {
        file_url: fields[0].into(),
        file_name: fields[1].into(),
        file_size: fields[2].parse().unwrap_or(-1),
        seeders: fields[3].parse().unwrap_or(-1),
        leechers: fields[4].parse().unwrap_or(-1),
        site_url: fields[5].into(),
        engine,
        descr_link: fields.get(6).copied().unwrap_or("").into(),
        pub_date: fields
            .get(7)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
            .max(0),
        imdb: String::new(),
    })
}

pub(super) fn job(id: i64) -> Result<Arc<SearchJob>> {
    jobs()
        .lock()
        .map_err(|_| anyhow!("search job lock failed"))?
        .jobs
        .get(&id)
        .cloned()
        .ok_or_else(|| anyhow!("search job not found: {id}"))
}

/// Local catalogue clients consume the same normalized items and resolved-hash
/// dedupe as the daemon service; legacy sentinels are introduced by the CLI only.
pub(crate) fn normalized_snapshot(id:i64) -> Result<(Vec<crate::api::SearchItem>,bool,Vec<crate::api::ProviderWarning>)> {
    let job=job(id)?;
    let state=job.state.lock().map_err(|_|anyhow!("search results lock failed"))?;
    let complete=job.done.load(Ordering::Acquire);
    let warnings=warnings(&state);
    Ok((normalize::aggregate_with_hashes(&state.results,MAX_RESULTS,&state.hashes),complete,warnings))
}

pub fn results(id: i64, limit: i64, offset: i64) -> Result<SearchResponse> {
    let job = job(id)?;
    let state = job
        .state
        .lock()
        .map_err(|_| anyhow!("search results lock failed"))?;
    let done = job.done.load(Ordering::Acquire);
    if done && state.results.is_empty() && (!state.error.is_empty() || !state.warnings.is_empty()) {
        bail!("search failed: {}", state.error);
    }
    let total = state.results.len() as i64;
    let skip = offset.max(0) as usize;
    let take = if limit < 0 {
        MAX_RESULTS
    } else {
        limit as usize
    };
    Ok(SearchResponse {
        results: state
            .results
            .iter()
            .skip(skip)
            .take(take)
            .cloned()
            .collect(),
        status: if done { "Stopped" } else { "Running" }.into(),
        total,
        error: (done && !state.error.is_empty()).then(|| state.error.clone()),
    })
}

fn stop_job(job: &SearchJob) {
    job.cancelled.store(true, Ordering::Release);
    for pid in job.native.lock().unwrap().iter(){unsafe{libc::kill(-*pid,libc::SIGKILL);}}
    let pid = job.pid.load(Ordering::Acquire);
    if pid > 0 {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    if let Ok(mut worker) = job.worker.lock() {
        if let Some(worker) = worker.take() {
            let _ = worker.join();
        }
    }
    job.done.store(true, Ordering::Release);
}

pub fn stop(id: i64) -> Result<()> {
    let selected = job(id)?;
    stop_job(&selected);
    Ok(())
}
pub fn delete(id: i64) -> Result<()> {
    let job = jobs()
        .lock()
        .map_err(|_| anyhow!("search job lock failed"))?
        .jobs
        .remove(&id);
    if let Some(job) = job {
        stop_job(&job);
    }
    Ok(())
}

pub fn shutdown_all() {
    let active = jobs()
        .lock()
        .map(|mut registry| {
            registry.closing = true;
            for pid in &registry.processes {
                unsafe {
                    libc::kill(-*pid, libc::SIGKILL);
                }
            }
            std::mem::take(&mut registry.jobs)
        })
        .unwrap_or_default();
    for (_, job) in active {
        stop_job(&job);
    }
}
