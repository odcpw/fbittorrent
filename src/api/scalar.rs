//! Constrained wire identifiers and closed enums. Open enums use strings so
//! newer reasons/origins/sources can be rendered as "other" by older clients.
use serde::{Deserialize, Deserializer, Serialize};
use std::{fmt, num::NonZeroU64};

pub type RequestId = NonZeroU64;

macro_rules! identifier {
    ($name:ident, $description:literal, $validate:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                if ($validate)(&value) {
                    Ok(Self(value))
                } else {
                    Err($description.into())
                }
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Self::new(String::deserialize(d)?).map_err(serde::de::Error::custom)
            }
        }
    };
}
identifier!(
    Hash,
    "hash must be 40 lowercase hex characters",
    |s: &str| super::validation::lower_hex(s, 40)
);
identifier!(
    Instance,
    "instance must be 32 lowercase hex characters",
    |s: &str| super::validation::lower_hex(s, 32)
);
identifier!(
    MutationRef,
    "ref must be at most 64 ASCII characters",
    |s: &str| super::validation::mutation_ref(s).is_ok()
);
identifier!(
    ClientName,
    "client name must be at most 64 characters",
    |s: &str| super::validation::text(s, 64, "client.name").is_ok()
);

macro_rules! closed_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
    };
}
closed_enum!(ClientKind {
    Cli,
    Agent,
    Tui,
    Bar,
    Test
});
closed_enum!(Launcher {
    Systemd,
    Foreground
});
closed_enum!(DaemonState {
    Starting,
    Running,
    Parked,
    Stopping
});
closed_enum!(TorrentState {
    Metadata,
    Checking,
    Downloading,
    Stalled,
    Queued,
    Paused,
    Done,
    Seeding,
    WaitingVpn,
    Error
});
closed_enum!(Intent { Run, Pause });
closed_enum!(UploadPolicy {
    DefaultCap,
    Unlimited,
    Custom
});
closed_enum!(RestartScope { Engine, Daemon });
closed_enum!(MutationState { Done, Pending });
closed_enum!(DoneState { Done });
closed_enum!(PendingState { Pending });
closed_enum!(TargetKind { File, Folder });
closed_enum!(NetworkReadiness {
    Ready,
    Waiting,
    Off
});
closed_enum!(PeerEncryption {
    Off,
    Prefer,
    Require
});
closed_enum!(Handler { Add, Confirm });
closed_enum!(LogLevel {
    Error,
    Warn,
    Info,
    Debug
});
closed_enum!(SnapshotReason {
    Subscribe,
    EngineRestart,
    Resync
});
closed_enum!(MutationOutcome {
    Done,
    Failed,
    Timeout
});
closed_enum!(ByeReason {
    Shutdown,
    Idle,
    Restart,
    Upgrade,
    SlowClient,
    HelloRequired,
    InvalidMessage
});

/// A P1 topic. Adding a topic is an additive protocol change; older clients
/// should ignore unsupported topics rather than inventing a store shape.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Topic {
    Shows,ShowReleases(String),
    RssFeeds, RssItems(String),
    Log,
    Summary,
    Torrents,
    Files(Hash),
    Pieces(Hash),
    Search(u64),
    Peers(Hash),
    Network,
    Settings,
    Events,
}
impl Topic {
    pub fn as_str(&self) -> String {
        match self {
            Self::Shows=>"shows".into(),Self::ShowReleases(id)=>format!("shows.releases:{id}"),
            Self::RssFeeds=>"rss.feeds".into(), Self::RssItems(id)=>format!("rss.items:{id}"),
            Self::Log => "log".into(),
            Self::Summary => "summary".into(),
            Self::Torrents => "torrents".into(),
            Self::Network => "network".into(),
            Self::Settings => "settings".into(),
            Self::Events => "events".into(),
            Self::Pieces(h) => format!("torrent:{h}.pieces"),
            Self::Files(h) => format!("torrent:{h}.files"),
            Self::Search(id) => format!("search:{id}"),
            Self::Peers(h) => format!("torrent:{h}.peers"),
        }
    }
}
impl Serialize for Topic {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.as_str())
    }
}
impl<'de> Deserialize<'de> for Topic {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "shows"=>Ok(Self::Shows),
            _ if s.starts_with("shows.releases:")&&s.len()>15&&s.len()<=143=>Ok(Self::ShowReleases(s[15..].into())),
            "rss.feeds"=>Ok(Self::RssFeeds),
            _ if s.starts_with("rss.items:") && s.len()>10 && s.len()<=138=>Ok(Self::RssItems(s[10..].into())),
            "log" => Ok(Self::Log),
            "summary" => Ok(Self::Summary),
            "torrents" => Ok(Self::Torrents),
            "network" => Ok(Self::Network),
            "settings" => Ok(Self::Settings),
            "events" => Ok(Self::Events),
            _ if s.starts_with("search:") => s[7..].parse::<u64>()
                .ok().filter(|id| *id > 0).map(Self::Search)
                .ok_or_else(|| serde::de::Error::custom("invalid search topic")),
            _ if s.starts_with("torrent:") && s.ends_with(".peers") => Hash::new(&s[8..s.len()-6]).map(Self::Peers).map_err(serde::de::Error::custom),
            _ if s.starts_with("torrent:") && s.ends_with(".pieces") => Hash::new(&s[8..s.len()-7]).map(Self::Pieces).map_err(serde::de::Error::custom),
            _ => s
                .strip_prefix("torrent:")
                .and_then(|s| s.strip_suffix(".files"))
                .ok_or_else(|| serde::de::Error::custom("unknown P1 topic"))
                .and_then(|h| {
                    Hash::new(h)
                        .map(Self::Files)
                        .map_err(serde::de::Error::custom)
                }),
        }
    }
}
