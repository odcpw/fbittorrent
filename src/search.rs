//! App-owned Nova search providers. Python runs only for metadata inspection or
//! an active search or a selected result download. Each runner's entire process
//! group is stopped on exit.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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

use crate::qbt::{SearchPlugin, SearchResult};

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
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
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
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        if let Some(parent) = path.parent() { File::open(parent)?.sync_all()?; }
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

type ProviderStamp = Vec<(String, u64, u128)>;
struct PluginCache {
    runtime: PathBuf,
    stamp: ProviderStamp,
    plugins: Vec<SearchPlugin>,
}
static PLUGINS: OnceLock<Mutex<Option<PluginCache>>> = OnceLock::new();

pub fn plugins() -> Result<Vec<crate::api::Provider>> {
    let store=ProviderStore::default();
    let mut rows=plugins_in(&store)?;
    let preferences=store.load()?;
    rows.extend(torznab::indexers()?.into_iter().map(|i|SearchPlugin {
        enabled:preferences.enabled.get(&i.name).copied().unwrap_or(true),
        full_name:i.name.clone(),name:i.name,url:i.url,version:String::new(),kind:"torznab".into(),
    }));
    rows.sort_by(|a,b|a.name.cmp(&b.name));
    Ok(rows.into_iter().map(|p| crate::api::Provider {
        name: p.name,
        kind: if p.kind == "torznab" {crate::api::ProviderKind::Torznab} else {crate::api::ProviderKind::Nova},
        enabled: p.enabled,
        full_name: (!p.full_name.is_empty()).then_some(p.full_name),
        url: (!p.url.is_empty()).then_some(p.url),
        version: (!p.version.is_empty()).then_some(p.version),
    }).collect())
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
        let mut available=if files.is_empty(){Vec::new()}else {
            let mut command = python(&store.runtime());
            command.arg("nova2.py").arg("--capabilities");
            parse_capabilities(&capture(command, Duration::from_secs(10))?)?
        };
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
    let store=ProviderStore::default();
    if torznab::indexers()?.iter().any(|i|i.name==name) {
        let _lock=store.lock()?;let mut prefs=store.load()?;prefs.enabled.insert(name.into(),enabled);return store.save(&prefs);
    }
    store.set_enabled(name, enabled)
}

pub fn install(sources: &[String], timeout_secs: u64) -> Result<()> {
    let store = ProviderStore::default();
    store.ensure_runtime()?;
    // Python source runs later: https only, including every redirect, and
    // VPN-bound when network_interface is set (see `crate::outbound`).
    let http = crate::outbound::Http::new(ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs.max(1))))
        .https_only(true)
        .build(), "provider install")?;
    let agent = http.agent();
    let mut failures = Vec::new();
    for source in sources {
        let result = (|| -> Result<()> {
            if torznab::indexers()?.iter().any(|i|i.url==*source) || url::Url::parse(source).is_ok_and(|u|!u.path().ends_with(".py")) {
                torznab::install(source,None)?;return Ok(());
            }
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
            File::open(path.parent().context("provider path has no parent")?)?.sync_all()?;
            Ok(())
        })();
        if let Err(error) = result {
            failures.push(format!("{}: {error:#}",torznab::public_source(source)));
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


mod download;
pub(crate) mod normalize;
pub(crate) mod service;
pub(crate) mod torznab;
mod jobs;
mod process;
#[cfg(test)]
mod tests;

pub(crate) use download::{resolve_download, ResolvedDownload};
#[cfg(test)]
use download::resolve_download_in;
pub use jobs::{begin_session, delete, shutdown_all, start};
pub(crate) use jobs::normalized_snapshot;
use jobs::jobs;
#[cfg(test)]
use jobs::{parse_result, results, start_in, stop};
#[cfg(test)]
use jobs::job;
use process::*;
