//! Render daemon-owned IMDb cards and bounded local poster resources.
use super::*;
use std::{io::Read, os::unix::fs::OpenOptionsExt};

pub(in crate::tui) fn lookup(
    connection: &attach::Presence,
    name: &str,
    id: Option<&str>,
) -> Result<Option<(Title, Option<Poster>)>> {
    let args = if let Some(id) = id.filter(|id| id.starts_with("tt")) {
        json!({"id":id})
    } else {
        json!({"name":name})
    };
    let reply: api::ImdbLookupResult = request(connection, "imdb.lookup", args)?;
    Ok(reply.card.map(|card| {
        let poster = card.poster_path.as_ref().and_then(|path| {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
                .ok()?;
            let info = file.metadata().ok()?;
            if !info.is_file() || info.len() > 8 * 1024 * 1024 {
                return None;
            }
            let mut bytes = Vec::new();
            file.take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            (bytes.len() <= 8 * 1024 * 1024)
                .then(|| Poster::from_bytes(&bytes))
                .flatten()
        });
        (
            Title {
                id: card.id,
                name: card.title,
                year: card.year,
                kind: card.kind,
                rating: card.rating.map(|r| r as f32),
                votes: card.votes,
                stars: card.stars.unwrap_or_default(),
                blurb: card.blurb.unwrap_or_default(),
                wiki_url: card.wiki_url.unwrap_or_default(),
                poster_url: card.poster_url.unwrap_or_default(),
            },
            poster,
        )
    }))
}
