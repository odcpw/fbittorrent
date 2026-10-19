//! IMDb reads own no engine handles and never start an engine slot.
use super::{ApiResult, error};
use crate::api::*;
use crate::imdb::service::{self, Service};
use serde_json::json;

fn upstream(value: anyhow::Error, source: &str) -> Error {
    let mut failure = error(ErrorCode::Upstream, &format!("{value:#}"), None);
    failure.details = Some(serde_json::Map::from_iter([(
        "source".into(),
        json!(source),
    )]));
    failure
}

pub fn lookup(service: &Service, args: &ImdbLookupArgs) -> ApiResult<Reply> {
    if args.name.is_some() == args.id.is_some() {
        return Err(error(
            ErrorCode::InvalidArgs,
            "provide exactly one of name or id",
            Some("name"),
        ));
    }
    if args
        .name
        .as_ref()
        .is_some_and(|s| s.trim().is_empty() || s.chars().count() > 1024)
    {
        return Err(error(
            ErrorCode::InvalidArgs,
            "name must contain 1..1024 characters",
            Some("name"),
        ));
    }
    if args.id.as_ref().is_some_and(|id| !service::valid_id(id)) {
        return Err(error(
            ErrorCode::InvalidArgs,
            "expected an IMDb title id",
            Some("id"),
        ));
    }
    let title = service
        .lookup(
            args.name.as_deref(),
            args.id.as_deref(),
            args.refresh.unwrap_or(false),
        )
        .map_err(|e| upstream(e, "imdb"))?;
    let card = title.map(|t| {
        let poster_path = if t.poster_url.is_empty() {
            None
        } else {
            match service::poster(&t) {
                Ok(path) => Some(path.to_string_lossy().into_owned()),
                Err(e) => {
                    // Preserve the useful card; retain the typed upstream diagnostic.
                    eprintln!(
                        "WARN imdb poster {}",
                        serde_json::to_string(&upstream(e, "imdb.poster")).unwrap()
                    );
                    None
                }
            }
        };
        let optional = |s: String| (!s.is_empty()).then_some(s);
        let url = t.url();
        ImdbCard {
            id: t.id,
            title: t.name,
            kind: t.kind,
            url,
            year: t.year,
            rating: t.rating.map(|r| (f64::from(r) * 10.0).round() / 10.0),
            votes: t.votes,
            stars: optional(t.stars),
            blurb: optional(t.blurb),
            wiki_url: optional(t.wiki_url),
            poster_url: optional(t.poster_url),
            poster_path,
        }
    });
    Ok(Reply::ImdbLookup(ImdbLookupResult { card }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_reject_ambiguous_or_unsafe_queries_before_io() {
        for args in [
            json!({}),
            json!({"name":"x","id":"tt1"}),
            json!({"id":"tt../../x"}),
            json!({"name":" "}),
        ] {
            let failure =
                lookup(&Service::default(), &serde_json::from_value(args).unwrap()).unwrap_err();
            assert_eq!(failure.code, ErrorCode::InvalidArgs);
        }
        assert!(serde_json::from_value::<ImdbLookupArgs>(json!({"name":"x","typo":true})).is_err());
    }
}
