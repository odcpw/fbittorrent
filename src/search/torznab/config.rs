//! Credentials stay in private files. Wrong modes and symlinks are refused.
use super::*;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Indexer {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_file: Option<PathBuf>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    #[serde(default)]
    pub torznab: Vec<Indexer>,
}
#[derive(Debug)]
pub(crate) struct Invalid(pub &'static str);
impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid Torznab {}", self.0)
    }
}
impl std::error::Error for Invalid {}
fn invalid(key: &'static str) -> anyhow::Error {
    Invalid(key).into()
}
pub(super) fn private(path: &Path, cap: u64) -> Result<Vec<u8>> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| invalid("credential file"))?;
    let meta = f.metadata().map_err(|_| invalid("credential file"))?;
    if !meta.is_file() || meta.mode() & 0o777 != 0o600 || meta.uid() != unsafe { libc::geteuid() } {
        return Err(invalid("credential file permissions (require 0600)"));
    }
    let mut bytes = Vec::new();
    f.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("credential file"))?;
    anyhow::ensure!(bytes.len() <= cap as usize, Invalid("credential file size"));
    Ok(bytes)
}
pub(super) fn endpoint(value: &str, key: bool) -> Result<url::Url> {
    let parsed = url::Url::parse(value).map_err(|_| invalid("url"))?;
    let loopback = match parsed.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed
            .query_pairs()
            .any(|(k, _)| matches!(k.to_ascii_lowercase().as_str(), "apikey" | "api_key"))
        || (key && parsed.scheme() != "https" && !loopback)
    {
        return Err(invalid(
            "url (HTTPS required for keyed non-loopback endpoints)",
        ));
    }
    Ok(parsed)
}
impl Indexer {
    pub(super) fn validate(&self) -> Result<()> {
        super::super::validate_name(&self.name).map_err(|_| invalid("name"))?;
        anyhow::ensure!(self.name.len() <= 64, Invalid("name"));
        endpoint(&self.url, self.api_key_file.is_some())?;
        if let Some(path) = &self.api_key_file {
            let key = key(path)?;
            anyhow::ensure!(
                !self.url.contains(&key) && !self.name.contains(&key),
                Invalid("inline credential (use api_key_file only)")
            );
        }
        Ok(())
    }
    pub(super) fn key(&self) -> Result<Option<String>> {
        self.api_key_file.as_ref().map(|p| key(p)).transpose()
    }
}
fn key(path: &Path) -> Result<String> {
    let value = String::from_utf8(private(path, 4096)?).map_err(|_| invalid("api_key_file"))?;
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) || value.contains(super::KEY_TOKEN) {
        return Err(invalid("api_key_file"));
    }
    Ok(value.into())
}
pub(crate) fn path() -> PathBuf {
    crate::config::config_path().with_file_name("indexers.toml")
}
pub(super) fn read(path: &Path) -> Result<Config> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(_) => return Err(invalid("indexers.toml")),
    };
    let meta = file.metadata().map_err(|_| invalid("indexers.toml"))?;
    if !meta.is_file() || meta.mode() & 0o777 != 0o600 || meta.uid() != unsafe { libc::geteuid() } {
        return Err(invalid("indexers.toml permissions (require 0600)"));
    }
    let mut text = String::new();
    file.take(256 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|_| invalid("indexers.toml"))?;
    anyhow::ensure!(text.len() <= 256 * 1024, Invalid("indexers.toml size"));
    let cfg: Config = toml::from_str(&text).map_err(|_| invalid("indexers.toml syntax"))?;
    anyhow::ensure!(
        cfg.torznab.len() <= 16,
        Invalid("indexer count (maximum 16)")
    );
    let mut names = std::collections::BTreeSet::new();
    for indexer in &cfg.torznab {
        indexer.validate()?;
        anyhow::ensure!(
            names.insert(indexer.name.clone()),
            Invalid("duplicate name")
        );
    }
    Ok(cfg)
}
pub(crate) fn indexers() -> Result<Vec<Indexer>> {
    Ok(read(&path())?.torznab)
}
pub(crate) fn install(url: &str, key_file: Option<PathBuf>) -> Result<String> {
    let parsed = endpoint(url, key_file.is_some())?;
    let path = path();
    let parent = path.parent().context("configuration directory")?;
    std::fs::create_dir_all(parent)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path.with_extension("lock"))?;
    anyhow::ensure!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == 0,
        "indexer configuration lock failed"
    );
    let mut cfg = read(&path)?;
    let name = if let Some(existing) = cfg.torznab.iter_mut().find(|i| i.url == url) {
        if key_file.is_some() {
            existing.api_key_file = key_file;
        }
        existing.validate()?;
        existing.name.clone()
    } else {
        anyhow::ensure!(
            cfg.torznab.len() < 16,
            Invalid("indexer count (maximum 16)")
        );
        let stem = format!("torznab_{}{}", parsed.host_str().unwrap(), parsed.path())
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .take(48)
            .collect::<String>();
        let mut name = stem.clone();
        let mut suffix = 1;
        while cfg.torznab.iter().any(|i| i.name == name) {
            suffix += 1;
            name = format!("{stem}_{suffix}");
        }
        let entry = Indexer {
            name: name.clone(),
            url: url.into(),
            api_key_file: key_file,
        };
        entry.validate()?;
        cfg.torznab.push(entry);
        name
    };
    let bytes = toml::to_string(&cfg).map_err(|_| invalid("indexers.toml serialization"))?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    f.write_all(bytes.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(tmp, &path)?;
    File::open(parent)?.sync_all()?;
    Ok(name)
}
