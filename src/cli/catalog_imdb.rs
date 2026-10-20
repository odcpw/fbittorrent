//! Both catalogue sources return the same API IMDb card.
use super::{catalog_search, fallback};
use crate::{agent::Code, api, config::Config};
use anyhow::Result;

pub(crate) fn arguments(query: &str, refresh: bool) -> api::ImdbLookupArgs {
    let id = crate::imdb::service::valid_id(query);
    api::ImdbLookupArgs {
        id: id.then(|| query.to_owned()),
        name: (!id).then(|| query.to_owned()),
        refresh: Some(refresh),
    }
}
pub(crate) fn run(cfg: &Config, query: String, refresh: bool, json_mode: bool) -> Result<Code> {
    if query.trim().is_empty() {
        crate::emit(
            json_mode,
            serde_json::json!({"ok":true,"found":false,"query":query}),
            || println!("no match"),
        );
        return Ok(Code::Ok);
    }
    let refresh = refresh || crate::imdb::ratings_stale();
    let card = if let Some(client) = catalog_search::running(cfg, "imdb") {
        let reply: api::ImdbLookupResult = serde_json::from_value(client.call(
            "imdb.lookup",
            serde_json::to_value(arguments(&query, refresh))?,
        )?)?;
        reply.card
    } else {
        fallback::imdb(&query, refresh)?
    };
    let Some(card) = card else {
        crate::emit(
            json_mode,
            serde_json::json!({"ok":true,"found":false,"query":query}),
            || println!("no match"),
        );
        return Ok(Code::Ok);
    };
    crate::emit(json_mode, serde_json::json!({"ok":true,"found":true,"card":card}), || {
        println!(
            "{} ({}) [{}]  {}",
            card.title,
            card.year.map(|v| v.to_string()).unwrap_or_default(),
            card.kind,
            card.url
        );
        match (card.rating, card.votes) {
            (Some(r), Some(v)) => println!("★ {r:.1}  ({v} votes)"),
            _ => println!("★ n/a"),
        }
        if let Some(stars) = &card.stars { println!("{stars}"); }
        if let Some(blurb) = &card.blurb { println!("\n{blurb}\n{}", card.wiki_url.as_deref().unwrap_or("")); }
        if let Some(path) = card.poster_path {
            println!("poster: {path}");
        }
    });
    Ok(Code::Ok)
}
