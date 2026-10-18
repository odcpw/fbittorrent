//! App-owned Nova search providers. Python runs only for metadata inspection or
//! an active search or a selected result download. Each runner's entire process
//! group is stopped on exit.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::qbt::{SearchPlugin, SearchResponse, SearchResult};

const MAX_RESULTS: usize = 5000;
const MAX_CAPTURE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TORRENT_BYTES: u64 = 16 * 1024 * 1024;
const DOWNLOAD_RUNNER: &str = r#"
import os, runpy, signal, sys
def stop_group(signum, frame):
    os.killpg(os.getpgrp(), signal.SIGKILL)
signal.signal(signal.SIGTERM, stop_group)
sys.argv = ['nova2dl.py', *sys.argv[1:]]
runpy.run_path('nova2dl.py', run_name='__main__')
"#;
const RUNTIME: &[(&str, &str)] = &[
    ("nova2.py", include_str!("../vendor/nova3/nova2.py")),
    ("nova2dl.py", include_str!("../vendor/nova3/nova2dl.py")),
    ("helpers.py", include_str!("../vendor/nova3/helpers.py")),
    (
        "novaprinter.py",
        include_str!("../vendor/nova3/novaprinter.py"),
    ),
    ("socks.py", include_str!("../vendor/nova3/socks.py")),
    ("README.md", include_str!("../vendor/nova3/README.md")),
];

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    enabled: BTreeMap<String, bool>,
    imported_legacy: bool,
}

struct PrefLock(File);
impl Drop for PrefLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Clone)]
struct ProviderStore {
    data: PathBuf,
    legacy: Option<PathBuf>,
}

impl Default for ProviderStore {
    fn default() -> Self {
        Self {
            data: crate::config::data_dir(),
            legacy: dirs::data_local_dir().map(|dir| dir.join("qBittorrent/nova3")),
        }
    }
}

impl ProviderStore {
    fn runtime(&self) -> PathBuf {
        self.data.join("nova3")
    }
    fn preferences(&self) -> PathBuf {
        self.data.join("search.json")
    }

    fn lock(&self) -> Result<PrefLock> {
        std::fs::create_dir_all(&self.data)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.data.join("search.lock"))?;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(PrefLock(file));
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error.into());
            }
        }
    }

    fn load(&self) -> Result<Preferences> {
        match std::fs::read(self.preferences()) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).context("parsing search provider preferences")
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(Preferences::default())
            }
            Err(error) => Err(error).context("reading search provider preferences"),
        }
    }

    fn save(&self, preferences: &Preferences) -> Result<()> {
        write_atomic(&self.preferences(), &serde_json::to_vec(preferences)?)
    }

    fn ensure_runtime(&self) -> Result<()> {
        let _lock = self.lock()?;
        let runtime = self.runtime();
        let engines = runtime.join("engines");
        std::fs::create_dir_all(&engines)?;
        // Refresh bundled helpers after upgrades while leaving provider scripts
        // as the user installed them. Their original licence notices are kept.
        for (name, source) in RUNTIME {
            let path = runtime.join(name);
            if std::fs::read(&path).ok().as_deref() != Some(source.as_bytes()) {
                write_atomic(&path, source.as_bytes())?;
            }
        }
        for path in [runtime.join("__init__.py"), engines.join("__init__.py")] {
            if !path.exists() {
                write_atomic(&path, b"")?;
            }
        }
        let mut preferences = self.load()?;
        if !preferences.imported_legacy {
            if let Some(legacy) = &self.legacy {
                copy_python_tree(&legacy.join("engines"), &engines)?;
            }
            preferences.imported_legacy = true;
            self.save(&preferences)?;
        }
        Ok(())
    }

    fn set_enabled(&self, name: &str, enabled: bool) -> Result<()> {
        self.ensure_runtime()?;
        validate_name(name)?;
        if !self
            .runtime()
            .join("engines")
            .join(format!("{name}.py"))
            .exists()
        {
            bail!("search provider not found: {name}");
        }
        let _lock = self.lock()?;
        let mut preferences = self.load()?;
        preferences.enabled.insert(name.into(), enabled);
        self.save(&preferences)
    }
}

fn copy_python_tree(source: &Path, target: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(source) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("reading existing search providers"),
    };
    std::fs::create_dir_all(target)?;
    for entry in entries {
        let entry = entry?;
        let ty = entry.file_type()?;
        let path = target.join(entry.file_name());
        if ty.is_dir() && entry.file_name() != "__pycache__" {
            copy_python_tree(&entry.path(), &path)?;
        } else if ty.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "py")
            && !path.exists()
        {
            write_atomic(&path, &std::fs::read(entry.path())?)?;
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), suffix));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.starts_with('_')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        bail!("invalid search provider name: {name}");
    }
    Ok(())
}

/// Enabled states from qBittorrent can be imported alongside its torrent/RSS
/// migration. Existing app preferences take priority on subsequent imports.
pub fn import_plugins(plugins: &[SearchPlugin]) -> Result<()> {
    let store = ProviderStore::default();
    store.ensure_runtime()?;
    let _lock = store.lock()?;
    let mut preferences = store.load()?;
    for plugin in plugins {
        preferences
            .enabled
            .entry(plugin.name.clone())
            .or_insert(plugin.enabled);
    }
    store.save(&preferences)
}

type ProviderStamp = Vec<(String, u64, u128)>;
struct PluginCache {
    runtime: PathBuf,
    stamp: ProviderStamp,
    plugins: Vec<SearchPlugin>,
}
static PLUGINS: OnceLock<Mutex<Option<PluginCache>>> = OnceLock::new();

pub fn plugins() -> Result<Vec<SearchPlugin>> {
    plugins_in(&ProviderStore::default())
}

fn plugins_in(store: &ProviderStore) -> Result<Vec<SearchPlugin>> {
    store.ensure_runtime()?;
    let mut files = Vec::new();
    for entry in std::fs::read_dir(store.runtime().join("engines"))? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.extension().is_none_or(|extension| extension != "py")
            || validate_name(name).is_err()
        {
            continue;
        }
        let metadata = entry.metadata()?;
        let modified = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        files.push((name.to_string(), metadata.len(), modified));
    }
    files.sort();
    let mut cache = PLUGINS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| anyhow!("search metadata lock failed"))?;
    let changed = cache
        .as_ref()
        .is_none_or(|cache| cache.runtime != store.runtime() || cache.stamp != files);
    if changed {
        let mut command = python(&store.runtime());
        command.arg("nova2.py").arg("--capabilities");
        let output = capture(command, Duration::from_secs(10))?;
        let mut available = parse_capabilities(&output)?;
        // Nova omits scripts that fail import. Keep them discoverable, with an
        // explicit unavailable label, instead of silently losing providers.
        for (name, _, _) in &files {
            let source = std::fs::read_to_string(
                store.runtime().join("engines").join(format!("{name}.py")),
            )?;
            let version = source
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("# VERSION:")
                        .map(|v| v.trim().into())
                })
                .unwrap_or_default();
            if let Some(plugin) = available.iter_mut().find(|plugin| plugin.name == *name) {
                plugin.version = version;
            } else {
                available.push(SearchPlugin {
                    name: name.clone(),
                    full_name: format!("{name} (cannot load)"),
                    version,
                    ..SearchPlugin::default()
                });
            }
        }
        available.sort_by(|a, b| a.name.cmp(&b.name));
        *cache = Some(PluginCache {
            runtime: store.runtime(),
            stamp: files,
            plugins: available,
        });
    }
    let preferences = store.load()?;
    let mut available = cache.as_ref().unwrap().plugins.clone();
    for plugin in &mut available {
        plugin.enabled = !plugin.full_name.ends_with("(cannot load)")
            && preferences
                .enabled
                .get(&plugin.name)
                .copied()
                .unwrap_or(true);
    }
    Ok(available)
}

fn parse_capabilities(bytes: &[u8]) -> Result<Vec<SearchPlugin>> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut stack = Vec::new();
    let mut current: Option<SearchPlugin> = None;
    let mut output = Vec::new();
    loop {
        match reader
            .read_event()
            .context("reading search provider metadata")?
        {
            Event::Start(event) => {
                let name = String::from_utf8_lossy(event.local_name().as_ref()).to_string();
                if stack.len() == 1 {
                    current = Some(SearchPlugin {
                        name: name.clone(),
                        ..SearchPlugin::default()
                    });
                }
                stack.push(name);
            }
            Event::Text(event) => {
                if let Some(plugin) = current.as_mut() {
                    match stack.last().map(String::as_str) {
                        Some("name") => plugin.full_name.push_str(&event.decode()?),
                        Some("url") => plugin.url.push_str(&event.decode()?),
                        _ => {}
                    }
                }
            }
            Event::GeneralRef(event) => {
                if let Some(plugin) = current.as_mut() {
                    let text = quick_xml::escape::unescape(&format!("&{};", event.decode()?))?
                        .into_owned();
                    match stack.last().map(String::as_str) {
                        Some("name") => plugin.full_name.push_str(&text),
                        Some("url") => plugin.url.push_str(&text),
                        _ => {}
                    }
                }
            }
            Event::End(_) => {
                stack.pop();
                if stack.len() == 1 {
                    if let Some(mut plugin) = current.take() {
                        if plugin.full_name.is_empty() {
                            plugin.full_name = plugin.name.clone();
                        }
                        output.push(plugin);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(output)
}

pub fn set_enabled(name: &str, enabled: bool) -> Result<()> {
    ProviderStore::default().set_enabled(name, enabled)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResolvedDownload {
    Url(String),
    Torrent { filename: String, bytes: Vec<u8> },
}

/// Resolves only a result that the user has chosen to download. Browsing,
/// search refreshes and cached results never invoke provider download hooks.
pub(crate) fn resolve_download(
    result: &SearchResult,
    timeout_secs: u64,
) -> Result<ResolvedDownload> {
    resolve_download_in(&ProviderStore::default(), result, timeout_secs)
}

fn direct_download(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("magnet:")
        || ((lower.starts_with("https://") || lower.starts_with("http://"))
            && lower
                .split(['?', '#'])
                .next()
                .is_some_and(|path| path.ends_with(".torrent")))
}

struct DownloadScratch(PathBuf);
impl Drop for DownloadScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn resolve_download_in(
    store: &ProviderStore,
    result: &SearchResult,
    timeout_secs: u64,
) -> Result<ResolvedDownload> {
    let parameter = result.file_url.trim();
    if parameter.is_empty() {
        bail!("the selected result has no download link");
    }
    if direct_download(parameter) {
        return Ok(ResolvedDownload::Url(parameter.into()));
    }

    store.ensure_runtime()?;
    let provider = result.engine.trim();
    let known = validate_name(provider).is_ok()
        && store
            .runtime()
            .join("engines")
            .join(format!("{provider}.py"))
            .is_file();
    if !known {
        // Some built-in browse sources use extensionless direct torrent URLs.
        // They have no installed Python provider and are handled by the engine.
        if parameter.starts_with("https://") || parameter.starts_with("http://") {
            return Ok(ResolvedDownload::Url(parameter.into()));
        }
        bail!("no installed search provider can resolve this result: {provider}");
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let scratch = DownloadScratch(
        store
            .runtime()
            .join(format!("download-{}-{stamp}", std::process::id())),
    );
    std::fs::DirBuilder::new().mode(0o700).create(&scratch.0)?;
    let mut command = python(&store.runtime());
    command
        .args(["-c", DOWNLOAD_RUNNER, provider, parameter])
        .env("TMPDIR", &scratch.0)
        .env("TMP", &scratch.0)
        .env("TEMP", &scratch.0);
    let output = capture(command, Duration::from_secs(timeout_secs.clamp(1, 60)))
        .with_context(|| format!("resolving selected result with {provider}"))?;
    let output = std::str::from_utf8(&output).context("provider download output is not UTF-8")?;
    for line in output
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(url) = line.split_whitespace().next().filter(|url| {
            url.starts_with("magnet:") || url.starts_with("http://") || url.starts_with("https://")
        }) {
            return Ok(ResolvedDownload::Url(url.into()));
        }
        // Nova's protocol is `temporary-file source-url`; split from the right
        // so temporary paths containing spaces remain usable.
        let candidate = line.rsplit_once(' ').map_or(line, |(file, _)| file).trim();
        let path = Path::new(candidate);
        if !path.is_absolute() || !path.is_file() {
            continue;
        }
        let file = File::open(path).context("opening provider torrent file")?;
        let mut bytes = Vec::new();
        file.take(MAX_TORRENT_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_TORRENT_BYTES {
            bail!("provider torrent file exceeds 16 MiB");
        }
        librqbit::torrent_from_bytes(&bytes)
            .context("provider returned an invalid .torrent file")?;
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "search.torrent".into());
        return Ok(ResolvedDownload::Torrent { filename, bytes });
    }
    bail!("{provider} returned no magnet link or .torrent file; open the selected result's details to download it manually")
}

pub fn install(sources: &[String], timeout_secs: u64) -> Result<()> {
    let store = ProviderStore::default();
    store.ensure_runtime()?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs.max(1))))
        .build()
        .into();
    let mut failures = Vec::new();
    for source in sources {
        let result = (|| -> Result<()> {
            if !source.starts_with("https://") {
                bail!("provider download must use https");
            }
            let filename = source
                .split(['?', '#'])
                .next()
                .unwrap_or(source)
                .rsplit('/')
                .next()
                .unwrap_or("");
            let name = filename
                .strip_suffix(".py")
                .context("provider URL must end in .py")?;
            validate_name(name)?;
            let mut response = agent.get(source).call().context("downloading provider")?;
            let bytes = response
                .body_mut()
                .with_config()
                .limit(2 * 1024 * 1024)
                .read_to_vec()?;
            let text =
                std::str::from_utf8(&bytes).context("provider is not UTF-8 Python source")?;
            if text.trim_start().starts_with('<') {
                bail!("provider URL returned HTML instead of Python");
            }
            let path = store.runtime().join("engines").join(filename);
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let staged = path.with_extension(format!("{suffix}.download"));
            write_atomic(&staged, &bytes)?;
            // Compile without importing or running the provider on installation.
            let mut compile = python(&store.runtime());
            compile.arg("-c").arg("import pathlib,sys; compile(pathlib.Path(sys.argv[1]).read_bytes(), sys.argv[1], 'exec')").arg(&staged);
            if let Err(error) = capture(compile, Duration::from_secs(5)) {
                let _ = std::fs::remove_file(&staged);
                return Err(error).context("invalid provider script");
            }
            std::fs::rename(&staged, &path)?;
            Ok(())
        })();
        if let Err(error) = result {
            failures.push(format!("{source}: {error:#}"));
        }
    }
    if !failures.is_empty() {
        bail!(
            "some search providers could not be installed: {}",
            failures.join("; ")
        );
    }
    Ok(())
}

fn python(directory: &Path) -> Command {
    let mut command = Command::new("python3");
    command
        .current_dir(directory)
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::null());
    let parent = unsafe { libc::getpid() };
    // Every child gets its own process group. kill(-pid) reaches Nova's pool,
    // its resource tracker, and any provider subprocesses together.
    unsafe {
        command.pre_exec(move || {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // Nova handles TERM by killing its group, including subprocesses
            // created by providers. Killing only Nova would orphan those children.
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                libc::raise(libc::SIGKILL);
            }
            Ok(())
        });
    }
    command
}

struct ProcessGroup {
    child: Child,
    pid: i32,
}
impl ProcessGroup {
    fn spawn(mut command: Command) -> Result<Self> {
        let mut registry = jobs()
            .lock()
            .map_err(|_| anyhow!("search lifecycle lock failed"))?;
        if registry.closing {
            bail!("the app is closing; search providers cannot start");
        }
        let child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("search requires a working Python 3 installation (python3)")?;
        let pid = child.id() as i32;
        // Track metadata/compile runners too, before shutdown can acquire the
        // mutex. Search-job registration may happen after the worker is created.
        registry.processes.insert(pid);
        Ok(Self { child, pid })
    }
    fn kill(&self) {
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
        }
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.kill();
        let _ = self.child.wait();
        if let Ok(mut registry) = jobs().lock() {
            registry.processes.remove(&self.pid);
        }
    }
}

fn capture(command: Command, budget: Duration) -> Result<Vec<u8>> {
    let mut process = ProcessGroup::spawn(command)?;
    let mut stdout = process.child.stdout.take().unwrap();
    let mut stderr = process.child.stderr.take().unwrap();
    let output = thread::spawn(move || drain_bounded(&mut stdout, MAX_CAPTURE_BYTES));
    let errors = thread::spawn(move || drain_bounded(&mut stderr, 16 * 1024));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = process.child.try_wait()? {
            break Some(status);
        }
        if started.elapsed() > budget {
            process.kill();
            let _ = process.child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    process.kill(); // clear any children that outlived the runner
    let (output, exceeded) = output
        .join()
        .map_err(|_| anyhow!("search output reader failed"))??;
    let (errors, _) = errors
        .join()
        .map_err(|_| anyhow!("search error reader failed"))??;
    if exceeded {
        bail!("search provider output exceeded 2 MiB");
    }
    match status {
        Some(status) if status.success() => Ok(output),
        Some(_) => bail!(
            "search provider runtime failed: {}",
            String::from_utf8_lossy(&errors).trim()
        ),
        None => bail!("search provider runtime timed out"),
    }
}

fn drain_bounded(reader: &mut impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0; 8192];
    loop {
        let size = match reader.read(&mut buffer) {
            Ok(0) => return Ok((bytes, exceeded)),
            Ok(size) => size,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let retained = size.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..retained]);
        exceeded |= retained < size;
    }
}

struct JobState {
    results: Vec<SearchResult>,
    error: String,
}
struct SearchJob {
    pid: AtomicI32,
    done: AtomicBool,
    cancelled: AtomicBool,
    state: Mutex<JobState>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
#[derive(Default)]
struct SearchRegistry {
    closing: bool,
    processes: BTreeSet<i32>,
    jobs: BTreeMap<i64, Arc<SearchJob>>,
}
static JOBS: OnceLock<Mutex<SearchRegistry>> = OnceLock::new();
static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn jobs() -> &'static Mutex<SearchRegistry> {
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

fn start_in(store: &ProviderStore, query: &str) -> Result<i64> {
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
        .filter(|plugin| plugin.enabled)
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
            error: String::new(),
        }),
        worker: Mutex::new(None),
    });
    let worker_job = job.clone();
    let worker = thread::Builder::new()
        .name("torrent-search".into())
        .spawn(move || {
            let errors = thread::spawn(move || {
                // Drain stderr completely so a noisy failing provider cannot block
                // the search, while retaining a bounded diagnostic for the user.
                let mut reader = BufReader::new(stderr);
                let mut retained = String::new();
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    if retained.len() < 16 * 1024 {
                        retained.push_str(&line);
                    }
                    line.clear();
                }
                retained
            });
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                if let Some(result) = parse_result(&line, &providers) {
                    if let Ok(mut state) = worker_job.state.lock() {
                        if state.results.len() < MAX_RESULTS {
                            state.results.push(result);
                        }
                    }
                }
                line.clear();
            }
            let status = process.child.wait();
            process.kill();
            worker_job.pid.store(0, Ordering::Release);
            let errors = errors.join().unwrap_or_default();
            if !worker_job.cancelled.load(Ordering::Acquire)
                && !status.as_ref().is_ok_and(|status| status.success())
            {
                if let Ok(mut state) = worker_job.state.lock() {
                    state.error = if errors.trim().is_empty() {
                        "search providers failed to finish".into()
                    } else {
                        errors.trim().into()
                    };
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

fn parse_result(line: &str, providers: &[SearchPlugin]) -> Option<SearchResult> {
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

fn job(id: i64) -> Result<Arc<SearchJob>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    static SEARCH_TESTS: Mutex<()> = Mutex::new(());

    fn session() -> std::sync::MutexGuard<'static, ()> {
        let guard = SEARCH_TESTS.lock().unwrap();
        begin_session().unwrap();
        guard
    }

    fn live_processes(group: i32) -> Vec<i32> {
        std::fs::read_dir("/proc")
            .unwrap()
            .flatten()
            .filter_map(|entry| {
                let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
                let (pid, tail) = stat.split_once(" (")?;
                let (_, tail) = tail.rsplit_once(") ")?;
                let fields: Vec<_> = tail.split_whitespace().collect();
                (fields.first() != Some(&"Z")
                    && fields.get(2).and_then(|value| value.parse::<i32>().ok()) == Some(group))
                .then(|| pid.parse().ok())
                .flatten()
            })
            .collect()
    }

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn scratch() -> (Scratch, ProviderStore) {
        let dir = std::env::temp_dir().join(format!(
            "fbittorrent-search-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        (
            Scratch(dir.clone()),
            ProviderStore {
                data: dir,
                legacy: None,
            },
        )
    }

    #[test]
    fn nova_results_preserve_urls_sizes_providers_and_dates() {
        let providers = vec![SearchPlugin {
            name: "example".into(),
            url: "https://example.org/".into(),
            ..SearchPlugin::default()
        }];
        let result = parse_result("magnet:?xt=urn:btih:abc|Example S01E01|1234|100|2|https://example.org|https://example.org/1|1791547200\n", &providers).unwrap();
        assert_eq!(result.engine, "example");
        assert_eq!(result.file_size, 1234);
        assert_eq!(result.seeders, 100);
        assert_eq!(result.pub_date, 1791547200);
        assert!(parse_result("a provider printed a diagnostic", &providers).is_none());
    }

    #[test]
    fn providers_are_imported_without_changing_original_files() {
        let _session = session();
        let (_scratch, mut store) = scratch();
        let legacy = store.data.join("legacy");
        std::fs::create_dir_all(legacy.join("engines")).unwrap();
        let source = b"# VERSION: 1\nclass example:\n    name = 'Example & Friends'\n    url = 'https://example.org'\n    def search(self, query): pass\n";
        std::fs::write(legacy.join("engines/example.py"), source).unwrap();
        store.legacy = Some(legacy.clone());
        let providers = plugins_in(&store).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].full_name, "Example & Friends");
        assert_eq!(
            std::fs::read(legacy.join("engines/example.py")).unwrap(),
            source
        );
        store.set_enabled("example", false).unwrap();
        assert!(!plugins_in(&store).unwrap()[0].enabled);
        assert!(start_in(&store, "test").is_err());
    }

    #[test]
    fn selected_detail_result_runs_its_provider_download_hook() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(
            store.runtime().join("engines/example.py"),
            r#"class example:
    def download_torrent(self, parameter):
        import pathlib
        pathlib.Path('download-hook-called').write_text(parameter)
        print('magnet:?xt=urn:btih:0123456789012345678901234567890123456789 ' + parameter)
"#,
        )
        .unwrap();
        let result = SearchResult {
            file_url: "https://example.org/details/123".into(),
            engine: "example".into(),
            ..SearchResult::default()
        };
        assert_eq!(
            resolve_download_in(&store, &result, 5).unwrap(),
            ResolvedDownload::Url(
                "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into()
            )
        );
        assert_eq!(
            std::fs::read_to_string(store.runtime().join("download-hook-called")).unwrap(),
            result.file_url
        );
        assert!(jobs().lock().unwrap().processes.is_empty());
    }

    #[test]
    fn selected_provider_torrent_file_is_read_and_its_scratch_is_removed() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/example.py"), r#"class example:
    def download_torrent(self, parameter):
        import os, pathlib
        target = pathlib.Path(os.environ['TMPDIR']) / 'payload file.torrent'
        target.write_bytes(b'd4:infod6:lengthi1e4:name1:x12:piece lengthi4e6:pieces20:aaaaaaaaaaaaaaaaaaaaee')
        pathlib.Path('download-file-path').write_text(str(target))
        print(str(target) + ' https://example.org/download')
"#).unwrap();
        let result = SearchResult {
            file_url: "opaque-provider-id".into(),
            engine: "example".into(),
            ..Default::default()
        };
        let resolved = resolve_download_in(&store, &result, 5).unwrap();
        let ResolvedDownload::Torrent { filename, bytes } = resolved else {
            panic!("expected a torrent file");
        };
        assert_eq!(filename, "payload file.torrent");
        assert!(librqbit::torrent_from_bytes(&bytes).is_ok());
        let temporary =
            std::fs::read_to_string(store.runtime().join("download-file-path")).unwrap();
        assert!(!Path::new(&temporary).exists());
        assert!(!Path::new(&temporary).parent().unwrap().exists());
    }

    #[test]
    fn direct_search_download_bypasses_python_even_after_shutdown() {
        let _session = session();
        let (_scratch, store) = scratch();
        shutdown_all();
        for url in [
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
            "https://example.org/payload.torrent?token=fixture",
        ] {
            let result = SearchResult {
                file_url: url.into(),
                engine: "../../untrusted-name".into(),
                ..Default::default()
            };
            assert_eq!(
                resolve_download_in(&store, &result, 5).unwrap(),
                ResolvedDownload::Url(url.into())
            );
        }
        assert!(!store.data.exists());
        assert!(jobs().lock().unwrap().processes.is_empty());
    }

    #[test]
    fn selected_download_hook_obeys_closed_spawn_gate() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(
            store.runtime().join("engines/example.py"),
            "raise RuntimeError('must never import after shutdown')",
        )
        .unwrap();
        shutdown_all();
        let result = SearchResult {
            file_url: "https://example.org/details/123".into(),
            engine: "example".into(),
            ..Default::default()
        };
        assert!(
            format!("{:#}", resolve_download_in(&store, &result, 5).unwrap_err())
                .contains("closing")
        );
        assert!(jobs().lock().unwrap().processes.is_empty());
    }

    #[test]
    fn selected_download_timeout_stops_provider_children() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(
            store.runtime().join("engines/example.py"),
            r#"class example:
    def download_torrent(self, parameter):
        import os, pathlib, subprocess, sys, time
        subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
        pathlib.Path('download-group').write_text(str(os.getpgrp()))
        time.sleep(60)
"#,
        )
        .unwrap();
        let result = SearchResult {
            file_url: "https://example.org/details/123".into(),
            engine: "example".into(),
            ..Default::default()
        };
        let failure = resolve_download_in(&store, &result, 1).unwrap_err();
        assert!(format!("{failure:#}").contains("timed out"));
        let group = std::fs::read_to_string(store.runtime().join("download-group"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(live_processes(group).is_empty());
        assert!(jobs().lock().unwrap().processes.is_empty());
    }

    #[test]
    fn active_search_streams_results_and_stop_kills_python_process_group() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/example.py"), r#"class example:
    name = 'Example'
    url = 'https://example.org'
    def search(self, query):
        import os, subprocess, sys, time
        from novaprinter import prettyPrinter
        child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
        open('provider-child.pid', 'w').write(str(child.pid))
        prettyPrinter({'link':'magnet:?xt=urn:btih:abc','name':'Fixture','size':123,'seeds':4,'leech':1,'engine_url':self.url})
        time.sleep(60)
"#).unwrap();
        let id = start_in(&store, "fixture").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if results(id, 100, 0).unwrap().total > 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "search never streamed its fixture result"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let process_group = job(id).unwrap().pid.load(Ordering::Acquire);
        stop(id).unwrap();
        assert_eq!(results(id, 100, 0).unwrap().status, "Stopped");
        // Zombies can briefly remain until the host reaps orphaned processes;
        // verify every member is terminated rather than testing /proc absence.
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            if let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) {
                if let Some((_, tail)) = stat.rsplit_once(") ") {
                    let fields: Vec<_> = tail.split_whitespace().collect();
                    if fields.get(2).and_then(|value| value.parse::<i32>().ok())
                        == Some(process_group)
                    {
                        assert_eq!(
                            fields[0], "Z",
                            "live process remained in search process group"
                        );
                    }
                }
            }
        }
        delete(id).unwrap();
    }

    #[test]
    fn shutdown_while_provider_metadata_loads_closes_the_spawn_gate() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/example.py"), "import pathlib,time\npathlib.Path('metadata-started').write_text('yes')\ntime.sleep(60)\nclass example:\n name='Example'\n url='https://example.org'\n def search(self,query): pass\n").unwrap();
        let worker_store = store.clone();
        let starting = thread::spawn(move || start_in(&worker_store, "fixture"));
        let deadline = Instant::now() + Duration::from_secs(8);
        while !store.runtime().join("metadata-started").exists() {
            assert!(Instant::now() < deadline, "metadata runner never started");
            thread::sleep(Duration::from_millis(20));
        }
        let groups = jobs().lock().unwrap().processes.clone();
        assert!(!groups.is_empty());
        shutdown_all();
        assert!(starting.join().unwrap().is_err());
        assert!(start_in(&store, "late query")
            .unwrap_err()
            .to_string()
            .contains("closing"));
        let registry = jobs().lock().unwrap();
        assert!(registry.jobs.is_empty());
        assert!(registry.processes.is_empty());
        drop(registry);
        for group in groups {
            assert!(live_processes(group).is_empty());
        }
        begin_session().unwrap();
        let mut command = python(&store.runtime());
        command.args(["-c", "print('reopened')"]);
        assert_eq!(
            capture(command, Duration::from_secs(2)).unwrap(),
            b"reopened\n"
        );
    }

    #[test]
    fn provider_failure_keeps_partial_results_and_reports_the_failure() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/example.py"), "class example:\n name='Example'\n url='https://example.org'\n def search(self,query):\n  from novaprinter import prettyPrinter\n  prettyPrinter({'link':'magnet:?xt=urn:btih:abc','name':'Result before failure','size':'1 GiB','seeds':4,'leech':1,'engine_url':self.url})\n  raise RuntimeError('fixture provider failed')\n").unwrap();
        let id = start_in(&store, "fixture").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let response = loop {
            let response = results(id, 100, 0).unwrap();
            if response.status != "Running" {
                break response;
            }
            assert!(Instant::now() < deadline, "failing provider did not finish");
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].file_size, 1 << 30);
        assert!(response.error.unwrap().contains("fixture provider failed"));
        delete(id).unwrap();
    }

    #[test]
    fn nova_parent_death_kills_provider_created_subprocesses() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/example.py"), "class example:\n name='Example'\n url='https://example.org'\n def search(self,query):\n  import pathlib,subprocess,sys,time\n  child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'])\n  pathlib.Path('provider-child.pid').write_text(str(child.pid))\n  time.sleep(60)\n").unwrap();
        let mut command = python(&store.runtime());
        command.args(["-c", r#"import ctypes,os,pathlib,signal,subprocess,sys,time
def prepare():
    os.setpgid(0,0)
    ctypes.CDLL(None).prctl(1,signal.SIGTERM,0,0,0)
child=subprocess.Popen([sys.executable,'nova2.py','example','all','fixture'],preexec_fn=prepare,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
pathlib.Path('nova.pid').write_text(str(child.pid))
deadline=time.monotonic()+5
while not pathlib.Path('provider-child.pid').exists():
    if time.monotonic()>deadline: raise RuntimeError('provider child never started')
    time.sleep(.02)
os._exit(0)
"#]);
        capture(command, Duration::from_secs(8)).unwrap();
        let group: i32 = std::fs::read_to_string(store.runtime().join("nova.pid"))
            .unwrap()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !live_processes(group).is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        let live = live_processes(group);
        // Always clean up a regressed fixture before reporting the failure.
        if !live.is_empty() {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        assert!(
            live.is_empty(),
            "provider subprocesses survived Nova parent death: {live:?}"
        );
    }
}
