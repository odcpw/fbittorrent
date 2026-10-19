//! Resolution only for a result the user chose to download.
use super::*;

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
    if let Some(download)=torznab::resolve(result)? {return Ok(download);}
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

pub(super) fn resolve_download_in(
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
