//! The desktop handler's confirmation is a deliberate UI action, never a hidden add.
use crate::{
    agent::{AppError, Code},
    api::client::Client,
    config::Config,
};
use anyhow::{Context, Result};
use serde_json::{json, Value};

pub(super) fn source(source: &str) -> Result<Value> {
    let local = crate::cli_sources::local_torrent_source(source)?;
    Ok(if source.starts_with("magnet:") {
        json!({"magnet":source})
    } else if source.starts_with("http://") || source.starts_with("https://") {
        json!({"url":source})
    } else {
        json!({"path":std::fs::canonicalize(local).map_err(|_|AppError::new(Code::Usage,"torrent source must be an accessible local file, magnet or HTTP URL"))?.to_string_lossy()})
    })
}

fn dashboard_required() -> AppError {
    AppError::new(
        Code::Upstream,
        "handler confirmation requires the dashboard; no download was started",
    )
    .hint("fbittorrent setup --plugin=copy")
}

pub(super) fn confirm(
    client: &Client,
    cfg: &Config,
    input: &str,
    stopped: bool,
    seed: bool,
    save: Option<String>,
    category: Option<String>,
) -> Result<Value> {
    // Resolve installation and shell availability before the first mutation.
    // Opening the window is part of this explicit confirmation request.
    if !crate::backend::summon_dashboard(&json!({"view":"torrents"}))? {
        return Err(dashboard_required().into());
    }
    let mut result = super::native::add(
        client,
        cfg,
        input,
        stopped,
        seed,
        save.clone(),
        category.clone(),
        "handler",
        true,
    )?;
    let hash = result["hash"].as_str().context("staged add omitted hash")?;
    let route = json!({"view":"torrents","hash":hash});
    let summoned = crate::backend::summon_dashboard(&route);
    if !matches!(summoned, Ok(true)) {
        // If the shell disappeared between the two summons, remove only the
        // stage created by this invocation. A duplicate belongs to its owner.
        if result["duplicate"] != true {
            let discarded = client
                .call("torrents.discard", json!({"hash":hash}))
                .context("dashboard unavailable; could not discard the new staged torrent")?;
            if discarded["state"] == "pending" {
                super::native::await_mutation(client, discarded)?;
            }
        }
        return Err(summoned
            .err()
            .unwrap_or_else(|| dashboard_required().into()));
    }
    result["ok"] = json!(true);
    result["action"] = json!("add");
    result["confirmation_required"] = json!(true);
    result["staged"] = json!(true);
    result["count"] = json!(1);
    result["hashes"] = if let Some(hash) = result["hash"].as_str() {
        json!([hash])
    } else {
        json!([])
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_rejects_inaccessible_local_paths_and_keeps_magnets_literal() {
        assert!(super::source("/nonexistent/fbt-input.torrent").is_err());
        let magnet = "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&dn=literal";
        assert_eq!(
            super::source(magnet).unwrap(),
            serde_json::json!({"magnet":magnet})
        );
    }
}
