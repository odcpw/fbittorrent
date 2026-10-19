//! Last-good settings for the owner's hot paths. The file is parsed again only
//! after its identity or mtime changes; a broken edit keeps the previous
//! settings, is logged once and is reported until a valid file replaces it.
use crate::config::Config;
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Device, inode, length and nanosecond mtime. Atomic saves rename a new inode.
pub(crate) type Stamp = (u64, u64, u64, i64, i64);

pub(crate) fn stamp(path: &Path) -> Option<Stamp> {
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino(), m.len(), m.mtime(), m.mtime_nsec()))
}

struct State {
    /// None until the first observation; Some(None) records an absent file.
    observed: Option<Option<Stamp>>,
    good: Arc<Config>,
    error: Option<String>,
}

pub(crate) struct ConfigCache {
    path: PathBuf,
    load: fn(&Path) -> anyhow::Result<Config>,
    state: Mutex<State>,
}

impl ConfigCache {
    /// `startup` is the configuration the owner already validated at launch.
    pub fn new(startup: Config) -> Self {
        Self::with_loader(crate::config::config_path(), startup, |_| Config::load())
    }

    fn with_loader(path: PathBuf, startup: Config, load: fn(&Path) -> anyhow::Result<Config>) -> Self {
        Self { path, load, state: Mutex::new(State { observed: None, good: Arc::new(startup), error: None }) }
    }

    /// One metadata call; a parse only after the file changed.
    pub fn current(&self) -> (Arc<Config>, Option<String>) {
        let next = stamp(&self.path);
        let mut state = self.state.lock().unwrap();
        if state.observed != Some(next) {
            state.observed = Some(next);
            match (self.load)(&self.path) {
                Ok(cfg) => {
                    if state.error.take().is_some() {
                        eprintln!("INFO daemon settings reloaded path={:?}", self.path.display().to_string());
                    }
                    state.good = Arc::new(cfg);
                }
                Err(error) => {
                    let message = format!("{error:#}").chars().take(512).collect::<String>();
                    if state.error.as_deref() != Some(message.as_str()) {
                        eprintln!("WARN daemon settings unreadable; keeping the last valid settings message={message:?}");
                    }
                    state.error = Some(message);
                }
            }
        }
        (state.good.clone(), state.error.clone())
    }

    /// The last valid settings, after observing any change.
    pub fn config(&self) -> Config {
        (*self.current().0).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(path: &Path) -> anyhow::Result<Config> {
        if !path.exists() { return Ok(Config::default()); }
        let cfg: Config = toml::from_str(&std::fs::read_to_string(path)?)?;
        cfg.validate()?;
        Ok(cfg)
    }
    fn directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("fbt-config-cache-{name}-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn replace(path: &Path, text: &str) {
        let staged = path.with_extension("tmp");
        std::fs::write(&staged, text).unwrap();
        std::fs::rename(staged, path).unwrap();
    }

    #[test]
    fn typo_keeps_last_good_settings_until_a_valid_file_returns() {
        let root = directory("typo");
        let path = root.join("config.toml");
        replace(&path, "idle_exit_secs = 120\nmax_active_downloads = 2\n");
        let cache = ConfigCache::with_loader(path.clone(), Config::default(), parse);
        let (cfg, error) = cache.current();
        assert_eq!((cfg.idle_exit_secs, cfg.max_active_downloads, error), (120, 2, None));
        replace(&path, "idle_exit_secs = 120\nmax_active_downloads = [\n");
        let (cfg, error) = cache.current();
        assert_eq!((cfg.idle_exit_secs, cfg.max_active_downloads), (120, 2), "last good survives a typo");
        let message = error.expect("the parse error is reported");
        assert!(message.contains("max_active_downloads") || message.contains("expected"), "{message}");
        // Unchanged broken file: no reparse, same report.
        assert_eq!(cache.current().1.as_deref(), Some(message.as_str()));
        replace(&path, "idle_exit_secs = 0\n");
        let (cfg, error) = cache.current();
        assert_eq!((cfg.idle_exit_secs, error), (0, None), "a valid edit clears the report");
        replace(&path, "idle_exit_secs = 5\n");
        assert_eq!(cache.current().0.idle_exit_secs, 0, "an invalid value is a broken file too");
        assert!(cache.current().1.unwrap().contains("idle_exit_secs"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unchanged_file_is_parsed_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static LOADS: AtomicUsize = AtomicUsize::new(0);
        fn counted(path: &Path) -> anyhow::Result<Config> { LOADS.fetch_add(1, Ordering::SeqCst); parse(path) }
        let root = directory("once");
        let path = root.join("config.toml");
        replace(&path, "max_active_downloads = 4\n");
        let cache = ConfigCache::with_loader(path.clone(), Config::default(), counted);
        for _ in 0..100 { assert_eq!(cache.current().0.max_active_downloads, 4); }
        assert_eq!(LOADS.load(Ordering::SeqCst), 1);
        replace(&path, "max_active_downloads = 5\n");
        assert_eq!(cache.current().0.max_active_downloads, 5);
        assert_eq!(LOADS.load(Ordering::SeqCst), 2);
        std::fs::remove_file(&path).unwrap();
        // An absent file is the default configuration, exactly as Config::load.
        let (cfg, error) = cache.current();
        assert_eq!((cfg.max_active_downloads, error), (Config::default().max_active_downloads, None));
        assert_eq!(LOADS.load(Ordering::SeqCst), 3);
        std::fs::remove_dir_all(root).unwrap();
    }
}
