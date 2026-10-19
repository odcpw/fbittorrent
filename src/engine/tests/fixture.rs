//! Test fixture notation translated to typed engine operations; no socket transport.
use super::*;
use serde::{Serialize, Deserialize};
#[derive(Debug, Serialize, Deserialize)]
pub struct Upload { pub name: String, pub bytes: Vec<u8> }
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub method: String, pub path: String, pub args: Vec<(String,String)>, pub file: Option<Upload>,
}
impl Request {
    pub fn arg(&self, name: &str) -> Option<&str> {
        self.args.iter().find(|(key,_)| key==name).map(|(_,value)|value.as_str())
    }
}
use crate::engine::ops::*;
use crate::engine::ops::required;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Version,
    Shutdown,
    Restart,
    Network,
    Settings,
    SetPreferences,
    Queue,
    SetQueue,
    TransferInfo,
    Info,
    Files,
    Add,
    Start,
    Stop,
    Delete,
    FilePriority,
    SetUploadLimit,
    SetShareLimits,
    AddTags,
    RemoveTags,
    ToggleSequentialDownload,
    ToggleFirstLastPiecePrio,
}

/// Names used by the historical characterization fixtures, confined to tests.
pub const LEGACY_ROUTES: &[(&str, Route)] = &[
    ("app/version", Route::Version),
    ("app/shutdown", Route::Shutdown),
    ("app/restart", Route::Restart),
    ("app/network", Route::Network),
    ("app/settings", Route::Settings),
    ("app/setPreferences", Route::SetPreferences),
    ("app/queue", Route::Queue),
    ("app/setQueue", Route::SetQueue),
    ("transfer/info", Route::TransferInfo),
    ("torrents/info", Route::Info),
    ("torrents/files", Route::Files),
    ("torrents/add", Route::Add),
    ("torrents/start", Route::Start),
    ("torrents/stop", Route::Stop),
    ("torrents/delete", Route::Delete),
    ("torrents/filePrio", Route::FilePriority),
    ("torrents/setUploadLimit", Route::SetUploadLimit),
    ("torrents/setShareLimits", Route::SetShareLimits),
    ("torrents/addTags", Route::AddTags),
    ("torrents/removeTags", Route::RemoveTags),
    (
        "torrents/toggleSequentialDownload",
        Route::ToggleSequentialDownload,
    ),
    (
        "torrents/toggleFirstLastPiecePrio",
        Route::ToggleFirstLastPiecePrio,
    ),
];
impl Route {
    pub fn from_name(name: &str) -> Option<Self> {
        LEGACY_ROUTES
            .iter()
            .find(|(route, _)| *route == name)
            .map(|(_, route)| *route)
    }
    pub fn name(self) -> &'static str {
        LEGACY_ROUTES
            .iter()
            .find(|(_, route)| *route == self)
            .expect("every route has a name")
            .0
    }
    pub fn is_control(self) -> bool {
        matches!(
            self,
            Self::Network | Self::Settings | Self::Restart | Self::Shutdown
        )
    }
}

impl EngineOp {
    pub fn parse(route: &str, body: &Request) -> Result<Option<Self>> {
        let Some(route) = Route::from_name(route) else {
            return Ok(None);
        };
        if route == Route::Restart && body.method != "POST" {
            return Ok(None);
        }
        let hashes = || -> Result<HashSelection> {
            let value = required(body, "hashes")?;
            Ok(if value == "all" {
                HashSelection::All
            } else {
                HashSelection::Hashes(
                    value
                        .split('|')
                        .filter(|hash| !hash.is_empty())
                        .map(str::to_owned)
                        .collect(),
                )
            })
        };
        let tags = || -> Result<Vec<String>> {
            Ok(required(body, "tags")?
                .split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned)
                .collect())
        };
        Ok(Some(match route {
            Route::Version => Self::Version,
            Route::Shutdown => Self::Shutdown,
            Route::Restart => Self::Restart,
            Route::Network => Self::Network,
            Route::Settings => Self::Settings,
            Route::SetPreferences => Self::SetPreferences,
            Route::Queue => Self::Queue,
            Route::SetQueue => Self::SetQueue {
                limit: required(body, "max_active_downloads")?.parse()?,
            },
            Route::TransferInfo => Self::TransferInfo,
            Route::Info => Self::Info,
            Route::Files => Self::Files {
                hash: required(body, "hash")?.into(),
            },
            Route::Start => Self::Start {
                hashes: hashes()?,
                skip_complete: body.arg("skip_complete") == Some("true"),
            },
            Route::Stop => Self::Stop { hashes: hashes()? },
            Route::Delete => Self::Delete {
                hashes: hashes()?,
                files: body.arg("deleteFiles") == Some("true"),
            },
            Route::FilePriority => {
                let priority: i64 = required(body, "priority")?.parse()?;
                if !matches!(priority, 0 | 1) {
                    bail!(
                        "the native engine supports skip (0) and normal (1) file priorities; high and maximum are unavailable"
                    );
                }
                Self::FilePriority {
                    hash: required(body, "hash")?.into(),
                    ids: required(body, "id")?
                        .split('|')
                        .map(str::parse)
                        .collect::<std::result::Result<_, _>>()?,
                    priority: priority as u8,
                }
            }
            Route::SetUploadLimit => {
                let limit = required(body, "limit")?.parse()?;
                if limit < 0 {
                    bail!("upload limit must be zero (unlimited) or a positive byte rate");
                }
                Self::SetUploadLimit {
                    hashes: hashes()?,
                    limit,
                }
            }
            Route::SetShareLimits => {
                let ratio = required(body, "ratioLimit")?.parse()?;
                if ratio != -2.0
                    && (ratio != 0.0
                        || !body
                            .arg("shareLimitAction")
                            .unwrap_or("Default")
                            .eq_ignore_ascii_case("stop"))
                {
                    bail!(
                        "native share limits support unlimited (-2) or stop on completion (ratio 0, Stop)"
                    );
                }
                Self::SetShareLimits {
                    hashes: hashes()?,
                    ratio,
                }
            }
            Route::AddTags => Self::AddTags {
                hashes: hashes()?,
                tags: tags()?,
            },
            Route::RemoveTags => Self::RemoveTags {
                hashes: hashes()?,
                tags: tags()?,
            },
            Route::ToggleSequentialDownload => Self::ToggleSequentialDownload,
            Route::ToggleFirstLastPiecePrio => Self::ToggleFirstLastPiecePrio,
            Route::Add => {
                let upload_limit = body
                    .arg("upLimit")
                    .map(str::parse::<i64>)
                    .transpose()?
                    .unwrap_or_else(|| {
                        if body
                            .arg("tags")
                            .unwrap_or("")
                            .split(',')
                            .any(|tag| tag.trim() == SEED_TAG)
                        {
                            0
                        } else {
                            NO_UPLOAD_LIMIT
                        }
                    });
                if upload_limit < 0 {
                    bail!("upload limit cannot be negative");
                }
                let ratio_limit = body
                    .arg("ratioLimit")
                    .map(str::parse::<f64>)
                    .transpose()?
                    .unwrap_or(if upload_limit == NO_UPLOAD_LIMIT {
                        0.0
                    } else {
                        -2.0
                    });
                if !matches!(ratio_limit, -2.0 | 0.0) {
                    bail!(
                        "native share limits support unlimited (-2) or stop on completion (ratio 0)"
                    );
                }
                let torrent = body
                    .file
                    .as_ref()
                    .map(|file| (file.name.clone(), file.bytes.clone()));
                let sources = if torrent.is_some() {
                    vec![String::new()]
                } else {
                    required(body, "urls")?
                        .lines()
                        .map(str::trim)
                        .filter(|source| !source.is_empty())
                        .map(str::to_owned)
                        .collect()
                };
                if sources.is_empty() {
                    bail!("no torrent was supplied");
                }
                Self::Add(AddRequest {
                    sources,
                    torrent,
                    save_path: body.arg("savepath").map(str::to_owned),
                    upload_limit,
                    ratio_limit,
                    stopped: body.arg("stopped") == Some("true")
                        || body.arg("paused") == Some("true"),
                    category: body.arg("category").unwrap_or("").into(),
                    tags: body.arg("tags").unwrap_or("").into(), origin: None,
                })
            }
        }))
    }
}

impl EngineDispatcher {
    /// Execute a saved test fixture through the typed engine.
    pub fn dispatch(&self, request: &Request) -> Option<Result<String>> {
        match EngineOp::parse(&request.path, request) {
            Ok(Some(op)) => Some(self.execute(op).and_then(|result| result.to_cli_json())),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }
}

impl ParkedLedger {
    pub fn dispatch(&self, request: &Request) -> Option<Result<String>> {
        match EngineOp::parse(&request.path, request) {
            Ok(Some(op)) => Some(self.execute(op).and_then(|result| {
                let legacy = result.to_cli_json()?;
                match result {
                    // Parked lists used Vec<Value>, unlike the live lists of
                    // structs. Retain that wire ordering at this legacy facade.
                    EngineResult::Torrents(_) | EngineResult::Files(_) => {
                        json(&serde_json::from_str::<serde_json::Value>(&legacy)?)
                    }
                    _ => Ok(legacy),
                }
            })),
            Ok(None) if request.path.starts_with("torrents/") => Some(Err(anyhow!(
                "connect ProtonVPN before changing downloads; saved downloads and settings remain available"
            ))),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }
}
pub(super) fn dispatch_parked_in(request: &Request, directory: &Path, lifecycle: &Lifecycle) -> Option<Result<String>> {
    ParkedLedger::new(directory.into(), lifecycle.clone()).dispatch(request)
}

pub(super) fn controls(request: &Request, snapshot: &crate::network_app::NetworkState, lifecycle: &Lifecycle) -> Option<Result<String>> {
    let route = Route::from_name(&request.path)?;
    if !route.is_control() || (matches!(route, Route::Restart | Route::Shutdown) && request.method != "POST") { return None; }
    match EngineOp::parse(&request.path, request) {
        Ok(Some(op)) => op.control(snapshot,lifecycle).map(|r|r.and_then(|r|r.to_cli_json())),
        Ok(None) => None, Err(error) => Some(Err(error)),
    }
}
