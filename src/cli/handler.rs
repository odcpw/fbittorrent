//! The desktop handler's confirmation is a deliberate UI action, never a hidden add.
use crate::{
    agent::{AppError, Code},
    api::client::Client,
    config::Config,
};
use anyhow::{Context, Result};
use serde_json::{Value, json};

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

pub(super) fn payload(
    source: Value,
    seed: bool,
    save: Option<String>,
    category: Option<String>,
    stopped: bool,
) -> Value {
    let mut add = json!({"source":source,"origin":"handler","seed":seed});
    if let Some(save) = save {
        add["save_path"] = json!(save);
    }
    if let Some(category) = category {
        add["category"] = json!(category);
    }
    if stopped {
        add["stopped"] = json!(true);
    }
    json!({"view":"torrents","add":add})
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
    let staging = client.hello.caps.iter().any(|cap| cap == "staging");
    let mut result = if staging {
        super::native::add(
            client,
            cfg,
            input,
            stopped,
            seed,
            save.clone(),
            category.clone(),
            "handler",
            true,
        )?
    } else {
        json!({})
    };
    let route = if staging {
        json!({"view":"torrents","hash":result["hash"].as_str().context("staged add omitted hash")?})
    } else {
        payload(
            source(input)?,
            seed || cfg.seed_by_default,
            save.or(cfg.save_path.clone()),
            category.or(cfg.category.clone()),
            stopped,
        )
    };
    if !crate::backend::summon_dashboard(&route)? {
        return Err(AppError::new(
            Code::Upstream,
            "handler confirmation requires the dashboard; no download was started",
        )
        .hint("fbittorrent setup --plugin=copy")
        .into());
    }
    result["ok"] = json!(true);
    result["action"] = json!("add");
    result["confirmation_required"] = json!(true);
    result["staged"] = json!(staging);
    result["count"] = json!(if staging { 1 } else { 0 });
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
    fn confirmation_payload_preserves_source_and_settings_as_data() {
        let source = serde_json::json!({"magnet":"magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&dn=$(never-execute)"});
        let value = super::payload(
            source.clone(),
            true,
            Some("/tmp/a folder".into()),
            Some("films".into()),
            true,
        );
        assert_eq!(value["add"]["source"], source);
        assert_eq!(value["add"]["origin"], "handler");
        assert_eq!(value["add"]["seed"], true);
        assert_eq!(value["add"]["save_path"], "/tmp/a folder");
        assert!(value["add"].get("stage").is_none());
        assert_eq!(value["add"]["category"], "films");
        assert_eq!(value["add"]["stopped"], true);
    }
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
