use super::*;

pub(super) fn response(id: Option<u64>, failure: Error) -> Frame {
    json!({"id":id,"ok":false,"error":failure})
}
pub(super) fn bye(reason: ByeReason) -> Frame {
    json!({"ev":"bye","reason":reason})
}
pub(super) fn add_timeout() -> Error {
    let mut failure = error(
        ErrorCode::Timeout,
        "URL add timed out; outcome is unknown",
        None,
    );
    failure.details = Some(serde_json::Map::from_iter([("after_s".into(), json!(95))]));
    failure
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(super) fn busy(limit: &str) -> Error {
    let mut failure = error(
        ErrorCode::Busy,
        "server limit reached; retry in a moment",
        None,
    );
    failure.details = Some(serde_json::Map::from_iter([("limit".into(), limit.into())]));
    failure
}
pub(super) fn line_overflow(bytes: usize) -> Error {
    let mut failure = error(ErrorCode::InvalidMessage, "line exceeds 65536 bytes", None);
    failure.details = Some(serde_json::Map::from_iter([("bytes".into(), json!(bytes))]));
    failure
}
pub(super) fn stopping() -> Error {
    let mut failure = error(ErrorCode::EngineUnavailable, "engine is stopping", None);
    failure.details = Some(serde_json::Map::from_iter([(
        "state".into(),
        "stopping".into(),
    )]));
    failure
}

/// Value's ordinary deserializer keeps the last duplicate key. Reject repeated
/// keys recursively before interpreting the strict typed request.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Unique, M::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, Unique>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!(
                            "duplicate key {}",
                            key.chars().take(80).collect::<String>()
                        )));
                    }
                    values.insert(key, value.0);
                }
                Ok(Unique(Value::Object(values)))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Unique, S::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<Unique>()? {
                    values.push(value.0);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Unique, E> {
                Ok(Unique(json!(value)))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
        }
        deserializer.deserialize_any(V)
    }
}
// The wire structs enforce semantic bounds too. Inspect field shapes first so
// a type error names its actual argument instead of serde's enclosing enum.
pub(super) fn argument_types(raw: &Value) -> Option<String> {
    if raw
        .get("ref")
        .is_some_and(|v| serde_json::from_value::<MutationRef>(v.clone()).is_err())
    {
        return Some("ref".into());
    }
    if raw
        .get("instance")
        .is_some_and(|v| serde_json::from_value::<Instance>(v.clone()).is_err())
    {
        return Some("instance".into());
    }
    let Some(args) = raw.get("args") else {
        return None;
    };
    let Some(args) = args.as_object() else {
        return Some("args".into());
    };
    let op = raw["op"].as_str().unwrap_or("");
    // Settings validate the entire patch against Config before any side effects,
    // retaining the exact offending key for both types and semantic bounds.
    if op == "app.settings.set" { return None; }
    for (key, value) in args {
        let valid = match key.as_str() {
            "interactive" | "glance" | "stage" | "stopped" | "seed" | "force" | "delete_files"
            | "on" | "unread_only" | "read" | "inbox" => value.is_boolean(),
            "feed" | "item" | "url" | "name" | "imdb" | "query" | "key" | "quality" => value.is_string(),
            "proto" | "file" | "sub" | "interval_ms" | "since_seq" => value.is_u64(),
            "bps" => value.is_u64() || (op == "torrents.set_upload_limit" && value.is_null()),
            "hash" => serde_json::from_value::<Hash>(value.clone()).is_ok(),
            "hashes" => serde_json::from_value::<Vec<Hash>>(value.clone()).is_ok(),
            "topics" => serde_json::from_value::<Vec<Topic>>(value.clone()).is_ok(),
            "states" => serde_json::from_value::<Vec<TorrentState>>(value.clone()).is_ok(),
            "files" | "include" | "skip" => value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_u64)),
            "origin" if op == "torrents.list" => value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string)),
            "reason" | "save_path" | "category" | "origin" => value.is_string(),
            "scope" => serde_json::from_value::<RestartScope>(value.clone()).is_ok(),
            "source" if op == "app.log" => value.is_string(),
            "level" => serde_json::from_value::<LogLevel>(value.clone()).is_ok(),
            "limit" => value.is_u64(),
            "source" => serde_json::from_value::<AddSource>(value.clone()).is_ok(),
            "client" => {
                let Some(client) = value.as_object() else {
                    return Some("client".into());
                };
                for (key, value) in client {
                    let valid = match key.as_str() {
                        "kind" => serde_json::from_value::<ClientKind>(value.clone()).is_ok(),
                        "name" => serde_json::from_value::<ClientName>(value.clone()).is_ok(),
                        "version" => value.is_string(),
                        _ => true,
                    };
                    if !valid {
                        return Some(format!("client.{key}"));
                    }
                }
                true
            }
            _ => true, // Unknown fields are rejected by the strict wire DTO.
        };
        if !valid {
            return Some(key.clone());
        }
    }
    None
}

pub(super) fn decode(bytes: &[u8]) -> Result<Request, Invalid> {
    let malformed = |message: &str| Invalid {
        id: None,
        error: error(ErrorCode::InvalidMessage, message, None),
        close: true,
    };
    if bytes.len() > LINE_IN {
        return Err(Invalid {
            id: None,
            error: line_overflow(bytes.len()),
            close: true,
        });
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let raw =
        match Unique::deserialize(&mut decoder).and_then(|value| decoder.end().map(|_| value.0)) {
            Ok(value) => value,
            Err(e) if e.to_string().contains("duplicate key") => {
                return Err(Invalid {
                    id: None,
                    error: error(ErrorCode::InvalidArgs, "duplicate object key", None),
                    close: false,
                })
            }
            Err(_) => return Err(malformed("expected one UTF-8 JSON object")),
        };
    let Some(object) = raw.as_object() else {
        return Err(malformed("expected a JSON object"));
    };
    if let Some(key) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "id" | "op" | "args" | "ref" | "instance"))
    {
        return Err(Invalid {
            id: None,
            error: error(
                ErrorCode::InvalidArgs,
                "unknown top-level field",
                Some(&key.chars().take(80).collect::<String>()),
            ),
            close: false,
        });
    }
    let Some(id) = raw.get("id").and_then(Value::as_u64).filter(|id| *id > 0) else {
        return Err(Invalid {
            id: None,
            error: error(
                ErrorCode::InvalidArgs,
                "id must be a positive integer",
                Some("id"),
            ),
            close: false,
        });
    };
    if !raw.get("op").is_some_and(Value::is_string) {
        return Err(Invalid {
            id: Some(id),
            error: error(ErrorCode::InvalidArgs, "op must be a string", Some("op")),
            close: false,
        });
    }
    if let Some(op) = raw.get("op").and_then(Value::as_str) {
        if !implemented(op) {
            let mut failure = error(
                ErrorCode::UnknownOp,
                &format!(
                    "unknown operation {:?}",
                    op.chars().take(80).collect::<String>()
                ),
                None,
            );
            let nearest: Vec<_> = ops::TABLE
                .iter()
                .filter(|row| implemented(row.name) && distance(op, row.name) <= 2)
                .take(3)
                .map(|row| row.name)
                .collect();
            failure.details = Some(serde_json::Map::from_iter([(
                "nearest".into(),
                json!(nearest),
            )]));
            return Err(Invalid {
                id: Some(id),
                error: failure,
                close: false,
            });
        }
    }
    if let Some(key) = argument_types(&raw) {
        let mut failure = error(ErrorCode::InvalidArgs, "invalid args", Some(&key));
        failure.hint = Some("inspect fbittorrent log".into());
        if matches!(
            key.as_str(),
            "interactive"
                | "glance"
                | "stage"
                | "stopped"
                | "seed"
                | "force"
                | "delete_files"
                | "on"
        ) {
            failure
                .details
                .as_mut()
                .unwrap()
                .insert("expected".into(), json!("boolean"));
        }
        return Err(Invalid {
            id: Some(id),
            error: failure,
            close: false,
        });
    }
    serde_json::from_value(raw).map_err(|e| {
        let text = e.to_string();
        let key = text
            .split('`')
            .nth(1)
            .or_else(|| {
                ["topics", "hashes", "bps", "since_seq", "limit", "source"]
                    .into_iter()
                    .find(|key| text.starts_with(&format!("{key}:")))
            })
            .unwrap_or("args");
        Invalid {
            id: Some(id),
            error: error(
                ErrorCode::InvalidArgs,
                &text.chars().take(80).collect::<String>(),
                Some(key),
            ),
            close: false,
        }
    })
}
// TABLE remains the frozen full contract. Subscription routing uses the bus.
pub(super) fn implemented(op: &str) -> bool {
    ops::TABLE.iter().any(|row| row.name == op)
}
pub(super) fn distance(a: &str, b: &str) -> usize {
    if a.len() > 128 {
        return usize::MAX;
    }
    let mut row: Vec<_> = (0..=b.chars().count()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut prior = row[0];
        row[0] = i + 1;
        for (j, y) in b.chars().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(prior + usize::from(x != y));
            prior = old;
        }
    }
    *row.last().unwrap()
}
