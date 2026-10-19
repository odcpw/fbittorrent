//! Daemon-local index ownership and bounded, atomic card resources.
use super::*;
use std::sync::{Arc, Mutex};

pub const JSON_CAP: u64 = 2 * 1024 * 1024;
pub const POSTER_CAP: u64 = 8 * 1024 * 1024;

pub fn valid_id(id: &str) -> bool {
    id.strip_prefix("tt").is_some_and(|digits| {
        !digits.is_empty() && digits.len() <= 10 && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

fn endpoint(url: &str) -> Result<String> {
    let Some(base) = std::env::var_os("FBT_IMDB_BASE_URL") else {
        return Ok(url.into());
    };
    let parsed = url::Url::parse(url)?;
    let mut target = url::Url::parse(&base.to_string_lossy())?;
    target.set_path(parsed.path());
    target.set_query(parsed.query());
    Ok(target.into())
}

pub(super) fn read_http(url: &str, timeout: u64, cap: u64) -> Result<Vec<u8>> {
    let mut response = agent(timeout)
        .get(endpoint(url)?)
        .call()
        .context("IMDb upstream request")?;
    anyhow::ensure!(
        response.status().as_u16() == 200,
        "IMDb upstream HTTP {}",
        response.status()
    );
    response
        .body_mut()
        .with_config()
        .limit(cap)
        .read_to_vec()
        .context("IMDb upstream byte cap/read")
}

/// One instance belongs to the daemon owner, independent of its engine slot.
#[derive(Default)]
pub struct Service {
    index: Mutex<Option<Arc<RatingsIndex>>>,
}

impl Service {
    fn index(&self, refresh: bool) -> Result<Arc<RatingsIndex>> {
        let mut slot = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if refresh || slot.is_none() {
            if refresh || !ratings_path().exists() {
                // The lock also serializes forced downloads and index replacement.
                let _download = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                download_ratings_to(&cache_dir(), RATINGS_URL)?;
            }
            let index = Arc::new(RatingsIndex::load()?);
            eprintln!("INFO imdb index loaded refresh={refresh}");
            *slot = Some(index);
        }
        Ok(slot.as_ref().unwrap().clone())
    }

    pub fn lookup(
        &self,
        name: Option<&str>,
        id: Option<&str>,
        refresh: bool,
    ) -> Result<Option<Title>> {
        let index = self.index(refresh)?;
        let resolved = if let Some(id) = id {
            Some(id.to_owned())
        } else {
            let name = name.context("missing IMDb name")?;
            if let Some(cache) = cache_read(NAME_CACHE_KIND, name) {
                if cache["missing"].as_bool() == Some(true) {
                    return Ok(None);
                }
                cache["id"].as_str().map(str::to_owned)
            } else {
                let release = crate::release::parse(name);
                let found = candidates(&release)?.into_iter().next();
                let value = found.as_ref().map_or_else(
                    || serde_json::json!({"missing":true}),
                    |title| serde_json::json!({"id":title.id}),
                );
                cache_write(NAME_CACHE_KIND, name, &value);
                if let Some(title) = found {
                    return self.finish(title, &index, true).map(Some);
                }
                return Ok(None);
            }
        };
        let Some(id) = resolved else { return Ok(None) };
        if let Some(title) = cached_title(&id) {
            return self.finish(title, &index, false).map(Some);
        }
        let Some(title) = search(&id, None)?.into_iter().find(|t| t.id == id) else {
            return Ok(None);
        };
        self.finish(title, &index, true).map(Some)
    }

    fn finish(&self, mut title: Title, index: &RatingsIndex, fresh: bool) -> Result<Title> {
        let rating = index.get(&title.id);
        title.rating = rating.map(|v| v.0);
        title.votes = rating.map(|v| v.1);
        if fresh {
            (title.blurb, title.wiki_url) = wiki_blurb(&title)?;
        }
        store_title(&title);
        Ok(title)
    }
}

pub fn poster(title: &Title) -> Result<PathBuf> {
    anyhow::ensure!(valid_id(&title.id), "invalid IMDb id");
    let root = cache_dir();
    let path = root.join("posters").join(format!("{}.jpg", title.id));
    if path.exists() {
        private_cached_file(&root, &path)?;
        anyhow::ensure!(
            path.metadata()?.len() <= POSTER_CAP,
            "cached poster exceeds byte cap"
        );
    } else {
        let bytes = read_http(&title.poster_url, 10, POSTER_CAP)?;
        write_cache_file(&root, &path, &bytes, None)?;
    }
    Ok(path)
}
