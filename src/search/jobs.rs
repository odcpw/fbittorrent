//! Search job registration, streaming results, cancellation and drain.
use super::*;

pub(super) struct JobState {
    pub(super) results: Vec<SearchResult>,
    pub(super) raw: Vec<u8>,
    pub(super) hashes: BTreeMap<String,String>,
    pub(super) error: String,
}
pub(super) struct SearchJob {
    pub(super) pid: AtomicI32,
    pub(super) done: AtomicBool,
    cancelled: AtomicBool,
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
    start_in(&ProviderStore::default(), query)
}

pub(super) fn start_in(store: &ProviderStore, query: &str) -> Result<i64> {
    start_selected(store, query, None)
}

pub(super) fn start_selected(store: &ProviderStore, query: &str, selected: Option<&[String]>) -> Result<i64> {
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
    let providers: Vec<_> = plugins_in(store)?
        .into_iter()
        .filter(|plugin| plugin.enabled && selected.is_none_or(|names| names.contains(&plugin.name)))
        .collect();
    if providers.is_empty() {
        bail!("no enabled search providers; install providers with fbittorrent search --install");
    }
    let names = providers
        .iter()
        .map(|plugin| plugin.name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let mut command = python(&store.runtime());
    command.arg("nova2.py").arg(names).arg("all").arg(query);
    let mut process = ProcessGroup::spawn(command)?;
    let stdout = process.child.stdout.take().unwrap();
    let stderr = process.child.stderr.take().unwrap();
    let job = Arc::new(SearchJob {
        pid: AtomicI32::new(process.pid),
        done: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        state: Mutex::new(JobState {
            results: vec![],
            raw: vec![],
            hashes: BTreeMap::new(),
            error: String::new(),
        }),
        worker: Mutex::new(None),
    });
    let worker_job = job.clone();
    let worker = thread::Builder::new()
        .name("torrent-search".into())
        .spawn(move || {
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
                    if let Ok(mut state) = worker_job.state.lock() { state.error = "search provider output exceeded 2 MiB".into(); }
                    process.kill(); break;
                }
                retained += count;
                let result = std::str::from_utf8(&line).ok().and_then(|text| parse_result(text, &providers));
                let hash = result.as_ref().and_then(|r| inspected.entry(r.file_url.clone()).or_insert_with(|| normalize::torrent_hash(&r.file_url)).clone());
                if let Ok(mut state) = worker_job.state.lock() {
                    state.raw.extend_from_slice(&line);
                    if let Some(result) = result {
                        if let Some(hash) = hash { state.hashes.insert(result.file_url.clone(), hash); }
                        if state.results.len() < MAX_RESULTS { state.results.push(result); }
                    }
                }
            }
            let status = process.child.wait();
            process.kill();
            worker_job.pid.store(0, Ordering::Release);
            let errors = errors.join().unwrap_or_default();
            if !worker_job.cancelled.load(Ordering::Acquire)
                && !status.as_ref().is_ok_and(|status| status.success())
            {
                if let Ok(mut state) = worker_job.state.lock() {
                    if state.error.is_empty() { state.error = if errors.trim().is_empty() {
                        "search providers failed to finish".into()
                    } else {
                        errors.trim().into()
                    }; }
                }
            }
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

pub fn results(id: i64, limit: i64, offset: i64) -> Result<SearchResponse> {
    let job = job(id)?;
    let state = job
        .state
        .lock()
        .map_err(|_| anyhow!("search results lock failed"))?;
    let done = job.done.load(Ordering::Acquire);
    if done && state.results.is_empty() && !state.error.is_empty() {
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
