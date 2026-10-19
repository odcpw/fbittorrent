//! Available API read operations; one existing presence connection owns transport.
use super::*;
mod client;
pub(super) mod search;
pub(super) mod imdb;
mod metadata;
pub(super) use client::ScreenClient;
mod subscription;
use crate::api;
use serde::de::DeserializeOwned;
use serde_json::json;
pub(super) use subscription::{verb as mutation_verb, ApiSub};

pub(super) struct ReadSnapshot {
    pub torrents: api::ListResult,
    pub transfer: api::TransferInfo,
    pub network: api::NetworkResult,
}
fn request<T: DeserializeOwned>(
    connection: &attach::Presence,
    op: &'static str,
    args: serde_json::Value,
) -> Result<T> {
    Ok(serde_json::from_value(connection.request(op, args)?)?)
}
pub(super) fn read(connection: &attach::Presence) -> Result<ReadSnapshot> {
    Ok(ReadSnapshot {
        torrents: request(connection, "torrents.list", json!({}))?,
        transfer: request(connection, "transfer.info", json!({}))?,
        network: request(connection, "app.network", json!({}))?,
    })
}
pub(super) fn files(connection: &attach::Presence, hash: &str) -> Result<Vec<TorrentFile>> {
    let reply: api::FilesResult = request(connection, "torrents.files", json!({"hash":hash}))?;
    let mut files = reply.files;
    files.sort_by(|left, right| {
        left.sort_key
            .cmp(&right.sort_key)
            .then(left.index.cmp(&right.index))
    });
    Ok(files.iter().map(format::file).collect())
}
pub(super) fn set_files(
    connection: &attach::Presence,
    hash: &str,
    ids: &[i64],
    priority: i64,
) -> Result<Vec<TorrentFile>> {
    anyhow::ensure!(
        matches!(priority, 0 | 1),
        "native file priority supports skip/normal"
    );
    let indices = ids
        .iter()
        .map(|index| u64::try_from(*index))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let field = if priority == 0 { "skip" } else { "include" };
    connection.request_confirmed("torrents.set_files", json!({"hash":hash,(field):indices}))?;
    files(connection, hash)
}
impl TuiApp {
    pub(super) fn apply_api_read(&mut self, result: Result<ReadSnapshot, String>) -> Cmd<Msg> {
        match result {
            Ok(snapshot) => {
                let result = (|| -> Result<(Vec<Torrent>, TransferInfo)> {
                    let call = api::Call::TorrentsList {
                        args: api::ListArgs {
                            states: None,
                            origin: None,
                        },
                    };
                    self.store
                        .reply(&call, api::Reply::TorrentsList(snapshot.torrents))?;
                    self.store.network = Some(snapshot.network.state);
                    self.store.transfer = Some(snapshot.transfer.clone());
                    let rows = self
                        .store
                        .sorted_torrents(store::TorrentSort::Added, true)
                        .into_iter()
                        .map(format::torrent)
                        .collect();
                    Ok((
                        rows,
                        format::transfer(&snapshot.transfer, self.store.network.as_ref()),
                    ))
                })();
                self.update(Msg::Torrents(result.map_err(|error| error.to_string())))
            }
            Err(error) => self.update(Msg::Torrents(Err(error))),
        }
    }
    pub(super) fn complete_no_seed(&self, torrent: &Torrent) -> bool {
        self.store.torrents.get(&torrent.hash).map_or_else(
            || torrent.complete_no_seed(),
            |row| (row.state == api::TorrentState::Done || row.completed_at.is_some()) && !row.seed,
        )
    }
    pub(super) fn torrent_paused(&self, torrent: &Torrent) -> bool {
        self.store.torrents.get(&torrent.hash).map_or_else(
            || torrent.is_stopped(),
            |row| row.intent == api::Intent::Pause,
        )
    }
    pub(super) fn network_banner(&self) -> (bool, std::borrow::Cow<'_, str>) {
        if let Some(network) = &self.store.network {
            return (network.ready, network.message.as_str().into());
        }
        if self.cfg.native() && self.presence.is_some() {
            return (false, "Connecting…".into());
        }
        let network = crate::network_app::state(&self.cfg);
        (network.ready, network.message.into())
    }
    pub(super) fn seed_allowed(&self, torrent: &Torrent) -> bool {
        self.store
            .torrents
            .get(&torrent.hash)
            .map_or_else(|| torrent.seed_allowed(), |row| row.seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn torrents() -> Vec<api::Torrent> {
        let text = include_str!("../../contracts/ipc/v1/torrents.ndjson");
        let frame = text
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|line| line["frame"]["result"]["torrents"].is_array())
            .unwrap();
        serde_json::from_value(frame["frame"]["result"]["torrents"].clone()).unwrap()
    }
    #[test]
    fn authoritative_api_read_populates_the_mirror_and_preserves_unknown_values() {
        let mut app = TuiApp::new(Config::default(), Screen::Torrents, Shows::default());
        let hello = include_str!("../../contracts/ipc/v1/torrents.ndjson")
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|line| line["frame"]["result"]["caps"].is_array())
            .unwrap();
        app.store
            .hello(serde_json::from_value(hello["frame"]["result"].clone()).unwrap());
        let rows = torrents();
        let metadata = rows
            .iter()
            .find(|row| row.state == api::TorrentState::Metadata)
            .unwrap()
            .hash
            .clone();
        let snapshot = ReadSnapshot {
            torrents: api::ListResult { torrents: rows },
            transfer: api::TransferInfo {
                session_downloaded: 0,
                session_uploaded: 0,
                dl_bps: None,
                ul_bps: None,
            },
            network: api::NetworkResult {
                state: api::NetworkState {
                    ready: false,
                    message: "Waiting for fixture VPN".into(),
                    selection: None,
                    interface: None,
                },
                interfaces: vec![],
            },
        };
        app.update(Msg::ApiRead {
            connection: None,
            result: Ok(snapshot),
        });
        assert!(app.online, "{}", app.backend_error);
        assert_eq!(app.network_banner().1, "Waiting for fixture VPN");
        let row = &app.store.torrents[metadata.as_str()];
        assert!(row.progress.is_none() && row.size.is_none() && row.dl_bps.is_none());
        let legacy = app
            .torrents
            .iter()
            .find(|row| row.hash == metadata.as_str())
            .unwrap();
        let metrics = format::Metrics {
            legacy,
            api: Some(row),
        };
        assert_eq!(metrics.speed(), "↓ —   ↑ —");
        assert_eq!(metrics.peers(true), "unknown");
        assert_eq!(metrics.ratio(), "—");
        assert!(!legacy.progress_known);
        assert_eq!(
            format::rate(app.store.transfer.as_ref().unwrap().dl_bps),
            "—"
        );
    }
    #[test]
    fn completed_without_a_date_and_custom_upload_cap_keep_their_actual_seed_intent() {
        let mut app = TuiApp::new(Config::default(), Screen::Torrents, Shows::default());
        let mut row = torrents()
            .into_iter()
            .find(|row| row.state == api::TorrentState::Done)
            .unwrap();
        row.completed_at = None;
        let displayed = format::torrent(&row);
        app.store.torrents.insert(row.hash.to_string(), row.clone());
        assert!(app.complete_no_seed(&displayed));
        row.seed = false;
        row.state = api::TorrentState::Downloading;
        row.upload_limit_bps = Some(65536);
        app.store.torrents.insert(row.hash.to_string(), row.clone());
        let displayed = format::torrent(&row);
        assert!(!app.seed_allowed(&displayed));
        let upload = format::Metrics {
            legacy: &displayed,
            api: Some(&row),
        }
        .upload();
        assert!(
            upload.contains("64") && upload.contains("cap") && !upload.contains("seeding allowed"),
            "{upload}"
        );
    }
    #[test]
    fn responses_from_an_old_connection_cannot_change_the_new_models_state() {
        let mut app = TuiApp::new(Config::default(), Screen::Torrents, Shows::default());
        app.poll_inflight = true;
        app.online = true;
        app.update(Msg::ApiRead {
            connection: Some(42),
            result: Err("old connection failed".into()),
        });
        assert!(app.online && app.poll_inflight);
        assert!(app.backend_error.is_empty());
    }

    #[test]
    fn vpn_waiting_controls_follow_saved_intent_and_known_completion() {
        let mut app = TuiApp::new(Config::default(), Screen::Torrents, Shows::default());
        let mut row = torrents().into_iter().find(|row|row.state==api::TorrentState::Done).unwrap();
        row.state=api::TorrentState::WaitingVpn;
        row.reason=Some("vpn_down".into());
        row.intent=api::Intent::Pause;
        row.completed_at=None;
        row.seed=false;
        app.store.torrents.insert(row.hash.to_string(),row.clone());
        app.torrents=vec![format::torrent(&row)];
        app.tor_list.borrow_mut().select(Some(0));
        let action=actions::all().find(|action|action.id=="torrents.pause").unwrap();
        assert_eq!((action.label)(&app).as_deref(),Some("Resume"));
        assert!(app.torrent_paused(&app.torrents[0]));
        assert!(!app.complete_no_seed(&app.torrents[0]));
        let paused=app.footer_signature(120);
        row.intent=api::Intent::Run;
        app.store.torrents.insert(row.hash.to_string(),row.clone());
        assert_eq!((action.label)(&app).as_deref(),Some("Pause"));
        assert!(!app.torrent_paused(&app.torrents[0]));
        assert_ne!(paused,app.footer_signature(120),"intent changes must refresh the legend while the VPN state stays the same");
        row.completed_at=Some(123);
        app.store.torrents.insert(row.hash.to_string(),row.clone());
        assert!((action.label)(&app).is_none(),"known complete no-seed records never offer Resume");
        row.seed=true;
        app.store.torrents.insert(row.hash.to_string(),row);
        assert_eq!((action.label)(&app).as_deref(),Some("Pause"));
    }
}
