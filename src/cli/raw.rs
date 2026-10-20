//! The response envelope is printed unchanged, including the API's typed errors.
use crate::{
    agent::{AppError, Code},
    api::{self, client::Client},
    config::Config,
};
use anyhow::Result;
use serde_json::{json, Value};
fn arguments(text: Option<String>, pairs: Vec<String>) -> Result<Value> {
    let mut args = text
        .map(|text| serde_json::from_str::<Value>(&text))
        .transpose()
        .map_err(|_| AppError::new(Code::Usage, "--json-args must be a JSON object"))?
        .unwrap_or_else(|| json!({}));
    let object = args
        .as_object_mut()
        .ok_or_else(|| AppError::new(Code::Usage, "--json-args must be a JSON object"))?;
    for pair in pairs {
        let (key, value) = pair
            .split_once('=')
            .filter(|(key, _)| !key.is_empty())
            .ok_or_else(|| AppError::new(Code::Usage, "--arg requires key=value"))?;
        if object.contains_key(key) {
            return Err(AppError::new(
                Code::Usage,
                format!(
                    "duplicate argument key: {}",
                    key.chars().take(64).collect::<String>()
                ),
            )
            .into());
        }
        object.insert(
            key.into(),
            serde_json::from_str(value).unwrap_or_else(|_| json!(value)),
        );
    }
    Ok(args)
}
fn print(frame: Value) -> Result<Code> {
    let exit = if frame["ok"] == true {
        0
    } else {
        let error: api::Error = serde_json::from_value(frame["error"].clone())?;
        error.code.exit_code()
    };
    println!("{frame}");
    Ok(match exit {
        0 => Code::Ok,
        2 => Code::Usage,
        3 => Code::Safety,
        4 => Code::Backend,
        5 => Code::Upstream,
        6 => Code::NotFound,
        7 => Code::Ambiguous,
        8 => Code::Timeout,
        _ => Code::Failure,
    })
}
pub(crate) fn catalog(json_mode: bool) -> Result<Code> {
    if json_mode {
        println!("{}", json!({"ok":true,"operations":api::ops::TABLE}));
    } else {
        std::print!("{}", api::ops::markdown());
    }
    Ok(Code::Ok)
}
pub(crate) fn run(
    cfg: &Config,
    json_mode: bool,
    no_start: bool,
    op: Option<String>,
    list: bool,
    pairs: Vec<String>,
    text: Option<String>,
    reference: Option<String>,
) -> Result<Code> {
    if list {
        return catalog(json_mode);
    }
    let op = op.ok_or_else(|| AppError::new(Code::Usage, "provide an API operation or --list"))?;
    let Some(operation) = api::ops::TABLE.iter().find(|row| row.name == op) else {
        let nearest = api::ops::nearest(&op);
        let mut error = api::ops::error(api::ErrorCode::UnknownOp, "unknown op", None);
        error.hint = Some(if nearest.is_empty() {
            "fbittorrent api --list".into()
        } else {
            format!("try: {}", nearest.join(", "))
        });
        error.details = Some(serde_json::Map::from_iter([(
            "nearest".into(),
            json!(nearest),
        )]));
        return print(json!({"id":null,"ok":false,"error":error}));
    };
    let args = arguments(text, pairs)?;
    if op == "hello" {
        let args = if args.as_object().unwrap().is_empty() {
            json!({"proto":1,"client":{"name":"raw-cli","kind":"agent","version":env!("CARGO_PKG_VERSION")}})
        } else {
            args
        };
        return print(Client::hello_envelope(args)?);
    }
    let client = match Client::connect_named(cfg, "raw-api") {
        Ok(client) => client,
        Err(error) if !operation.mutating => return Err(error),
        Err(_) => {
            api::client::ensure_daemon(cfg, no_start || !cfg.autostart)?;
            Client::connect_named(cfg, "raw-api")?
        }
    };
    print(client.envelope(&op, args, reference.as_deref())?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn args_are_typed_and_duplicates_are_rejected() {
        let args = arguments(
            Some(r#"{"hash":"abc"}"#.into()),
            vec!["on=true".into(), "count=2".into(), "path=/tmp/a=b".into()],
        )
        .unwrap();
        assert_eq!(
            args,
            json!({"hash":"abc","on":true,"count":2,"path":"/tmp/a=b"})
        );
        for text in ["[]", "null", "bad"] {
            assert!(arguments(Some(text.into()), vec![]).is_err());
        }
        assert!(arguments(None, vec!["missing".into()]).is_err());
        assert!(arguments(Some(r#"{"on":true}"#.into()), vec!["on=false".into()]).is_err());
    }
}
