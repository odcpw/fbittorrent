//! Torrent identifiers used by the embedded client.
use std::{marker::PhantomData, str::FromStr};
use dht::Id20;
use serde::{Serialize, Deserialize};
use crate::session::TorrentId;

#[derive(Debug, Clone, Copy)]
pub enum TorrentIdOrHash {
    Id(TorrentId),
    Hash(Id20),
}

impl Serialize for TorrentIdOrHash {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            TorrentIdOrHash::Id(id) => id.serialize(serializer),
            TorrentIdOrHash::Hash(h) => h.as_string().serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for TorrentIdOrHash {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Default)]
        struct V<'de> {
            p: PhantomData<&'de ()>,
        }

        macro_rules! visit_int {
            ($v:expr) => {{
                let tid: TorrentId = $v.try_into().map_err(|e| E::custom(format!("{e:#}")))?;
                Ok(TorrentIdOrHash::from(tid))
            }};
        }

        impl<'de> serde::de::Visitor<'de> for V<'de> {
            type Value = TorrentIdOrHash;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("integer or 40 byte info hash")
            }

            fn visit_i64<E>(self, v: i64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                visit_int!(v)
            }

            fn visit_i128<E>(self, v: i128) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                visit_int!(v)
            }

            fn visit_u128<E>(self, v: u128) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                visit_int!(v)
            }

            fn visit_u64<E>(self, v: u64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                visit_int!(v)
            }

            fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                TorrentIdOrHash::parse(v).map_err(|e| {
                    E::custom(format!(
                        "expected integer or 40 byte info hash, couldn't parse string: {e:#}"
                    ))
                })
            }
        }

        deserializer.deserialize_any(V::default())
    }
}

impl std::fmt::Display for TorrentIdOrHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TorrentIdOrHash::Id(id) => write!(f, "{id}"),
            TorrentIdOrHash::Hash(h) => write!(f, "{h:?}"),
        }
    }
}

impl From<TorrentId> for TorrentIdOrHash {
    fn from(value: TorrentId) -> Self {
        TorrentIdOrHash::Id(value)
    }
}

impl From<Id20> for TorrentIdOrHash {
    fn from(value: Id20) -> Self {
        TorrentIdOrHash::Hash(value)
    }
}

impl<'a> TryFrom<&'a str> for TorrentIdOrHash {
    type Error = anyhow::Error;

    fn try_from(value: &'a str) -> std::result::Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TorrentIdOrHash {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        if s.len() == 40 {
            let id = Id20::from_str(s)?;
            return Ok(id.into());
        }
        let id: TorrentId = s.parse()?;
        Ok(id.into())
    }
}

