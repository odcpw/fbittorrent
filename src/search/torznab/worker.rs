//! A session-owned native HTTP worker. Stop/quit can kill a stalled TLS request.
use super::*;
use base64::Engine;
use std::os::fd::AsRawFd;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub name: String,
    pub query: Option<String>,
    pub download: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub rows: Vec<SearchResult>,
    pub torrent: Option<String>,
    pub error: Option<String>,
}
impl Reply {
    fn failed(message: &str) -> Self {
        Self {
            rows: Vec::new(),
            torrent: None,
            error: Some(message.into()),
        }
    }
}
fn origin(u: &url::Url) -> (&str, Option<&str>, Option<u16>) {
    (u.scheme(), u.host_str(), u.port_or_known_default())
}
fn encoded(value: &str) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair("", value)
        .finish()[1..]
        .to_owned()
}
fn redact(value: &str, key: Option<&str>) -> String {
    match key {
        Some(key) => value
            .replace(&encoded(key), KEY_TOKEN)
            .replace(key, KEY_TOKEN),
        None => value.into(),
    }
}
fn fetch(
    indexer: &Indexer,
    query: Option<&str>,
    download: Option<&str>,
) -> Result<(Vec<u8>, Option<String>)> {
    let key = indexer.key()?;
    let base = config::endpoint(&indexer.url, key.is_some())?;
    let mut target = if let Some(source) = download {
        let source = source.replace(KEY_TOKEN, key.as_deref().unwrap_or(""));
        let url =
            url::Url::parse(&source).map_err(|_| anyhow!("Torznab download URL is invalid"))?;
        anyhow::ensure!(
            origin(&url) == origin(&base),
            "Torznab cross-origin download refused"
        );
        url
    } else {
        base.clone()
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    for _ in 0..6 {
        if let Some(query) = query {
            // Retain indexer-specific path/query parameters; own these three.
            let pairs: Vec<_> = target
                .query_pairs()
                .filter(|(k, _)| !matches!(k.as_ref(), "t" | "q" | "apikey"))
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            target.set_query(None);
            let mut qs = target.query_pairs_mut();
            for (k, v) in pairs {
                qs.append_pair(&k, &v);
            }
            qs.append_pair("t", "search").append_pair("q", query);
            if let Some(key) = &key {
                qs.append_pair("apikey", key);
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        anyhow::ensure!(!remaining.is_zero(), "Torznab request timed out");
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .proxy(None)
            .timeout_global(Some(remaining))
            .build()
            .into();
        let mut response = agent
            .get(target.as_str())
            .call()
            .map_err(|_| anyhow!("Torznab upstream request failed"))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow!("Torznab redirect omitted its location"))?;
            let next = target
                .join(location)
                .map_err(|_| anyhow!("Torznab redirect URL is invalid"))?;
            anyhow::ensure!(
                origin(&next) == origin(&base),
                "Torznab cross-origin redirect refused"
            );
            target = next;
            continue;
        }
        anyhow::ensure!(status == 200, "Torznab upstream HTTP failure");
        let cap = if download.is_some() {
            super::super::MAX_TORRENT_BYTES
        } else {
            RESPONSE_CAP
        };
        let bytes = response
            .body_mut()
            .with_config()
            .limit(cap)
            .read_to_vec()
            .map_err(|_| anyhow!("Torznab response cap or body read failed"))?;
        return Ok((bytes, key));
    }
    bail!("Torznab redirect limit exceeded")
}
pub(crate) fn run_if_requested() -> Option<i32> {
    if std::env::var_os("FBT_TORZNAB_WORKER").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return None;
    }
    let result = || -> Result<Reply> {
        let mut input = Vec::new();
        std::io::stdin().take(16385).read_to_end(&mut input)?;
        anyhow::ensure!(input.len() <= 16384, "Torznab worker input limit");
        let request: Request = serde_json::from_slice(&input)
            .map_err(|_| anyhow!("Torznab worker input is invalid"))?;
        anyhow::ensure!(
            request.query.is_some() != request.download.is_some(),
            "Torznab worker action is invalid"
        );
        if let Some(q) = &request.query {
            anyhow::ensure!(q.chars().count() <= 256, "Torznab query is too long");
        }
        let indexer = config::indexers()?
            .into_iter()
            .find(|i| i.name == request.name)
            .ok_or_else(|| anyhow!("Torznab indexer is not configured"))?;
        let (bytes, key) = fetch(
            &indexer,
            request.query.as_deref(),
            request.download.as_deref(),
        )?;
        if request.download.is_some() {
            librqbit::torrent_from_bytes(&bytes)
                .map_err(|_| anyhow!("Torznab download is not metainfo"))?;
            return Ok(Reply {
                rows: Vec::new(),
                torrent: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
                error: None,
            });
        }
        let mut rows = xml::parse(&bytes, &indexer)?;
        for row in &mut rows {
            row.file_url = redact(&row.file_url, key.as_deref());
            row.descr_link = redact(&row.descr_link, key.as_deref());
            row.file_name = redact(&row.file_name, key.as_deref());
        }
        Ok(Reply {
            rows,
            torrent: None,
            error: None,
        })
    };
    // All external errors are intentionally static; never format a ureq URL,
    // response body, TOML source or key-file content in diagnostics.
    let reply = result().unwrap_or_else(|e| {
        let message = if e.downcast_ref::<config::Invalid>().is_some() {
            "Torznab credential/configuration refused"
        } else {
            match e.to_string().as_str() {
                "Torznab cross-origin redirect refused" => "Torznab cross-origin redirect refused",
                _ => "Torznab upstream response failed",
            }
        };
        Reply::failed(message)
    });
    Some(
        if serde_json::to_writer(std::io::stdout(), &reply).is_ok() {
            0
        } else {
            1
        },
    )
}
pub(super) fn spawn(request: &Request) -> Result<super::super::process::ProcessGroup> {
    let mut command = Command::new(std::env::current_exe()?);
    command.env("FBT_TORZNAB_WORKER", "1").stdin(Stdio::piped());
    super::super::process::isolated(&mut command);
    let mut child = super::super::process::ProcessGroup::spawn(command)?;
    if let Some(mut input) = child.child.stdin.take() {
        serde_json::to_writer(&mut input, request)?;
        input.flush()?;
    }
    Ok(child)
}
