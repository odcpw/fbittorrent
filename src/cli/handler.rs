//! The desktop handler's confirmation is a deliberate UI action, never a hidden add.
use crate::{
    agent::{AppError, Code},
    config::Config,
};
use anyhow::Result;
use serde_json::{Value, json};

pub(super) fn source(source: &str) -> Result<Value> {
    let local = crate::cli_sources::local_torrent_source(source)?;
    Ok(if source.starts_with("magnet:") {
        json!({"magnet":source})
    } else if source.starts_with("http://") || source.starts_with("https://") {
        json!({"url":source})
    } else {
        json!({"path":std::fs::canonicalize(local).map_err(|_|AppError::new(Code::Usage,crate::user_messages::UNREADABLE_TORRENT))?.to_string_lossy()})
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
    cfg: &Config,
    input: &str,
    stopped: bool,
    save: Option<String>,
    category: Option<String>,
) -> Result<Value> {
    // The shell owns staging and confirmation. Do not wait for a daemon or
    // metadata before showing the source name and the Download action.
    let source = source(input)?;
    let route = json!({"view":"torrents","add":{"source":source,"origin":"handler",
        "seed":false,"stopped":stopped,"save_path":save.or_else(||cfg.save_path.clone()),
        "category":category.or_else(||cfg.category.clone())}});
    if !crate::backend::summon_dashboard(&route)? {
        return Err(dashboard_required().into());
    }
    Ok(json!({"ok":true,"action":"review","confirmation_required":true,"count":0}))
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
