//! Local feed and Shows commands.
use super::*;

pub(super) fn rss(c: &Client, sub: RssCmd, json: bool, cfg: &Config) -> Result<Code> {
    match sub {
        RssCmd::List => {
            let feeds = c.rss_feeds()?;
            emit(json, serde_json::json!({"ok": true, "count": feeds.len(), "feeds": feeds}), || {
                for f in &feeds {
                    let unread = f.articles.iter().filter(|a| !a.is_read).count();
                    println!("{:<30} {:>4} unread / {:>4}  {}{}", f.name, unread, f.articles.len(), f.url, if f.has_error { "  [error]" } else { "" });
                }
            });
        }
        RssCmd::Add { url, name } => {
            let n = name.clone().or_else(|| host_of(&url)).unwrap_or_else(|| url.clone());
            c.rss_add_feed(&url, Some(&n))?;
            emit(json, serde_json::json!({"ok": true, "action": "rss-add", "name": n, "url": url}), || println!("added feed {n}"));
        }
        RssCmd::Remove { name } => {
            let feeds = c.rss_feeds()?;
            let f = find_feed(&feeds, &name)?;
            c.rss_remove(&f.name)?;
            emit(json, serde_json::json!({"ok": true, "action": "rss-remove", "name": f.name}), || println!("removed {}", f.name));
        }
        RssCmd::Refresh { name } => {
            let feeds = c.rss_feeds()?;
            let names: Vec<String> = match name {
                Some(n) => vec![find_feed(&feeds, &n)?.name.clone()],
                None => feeds.iter().map(|f| f.name.clone()).collect(),
            };
            for n in &names { c.rss_refresh(n)?; }
            emit(json, serde_json::json!({"ok": true, "action": "rss-refresh", "feeds": names}), || println!("refreshing {} feed(s)", names.len()));
        }
        RssCmd::Items { name, unread } => {
            let feeds = c.rss_feeds()?;
            let f = find_feed(&feeds, &name)?;
            let items: Vec<_> = f.articles.iter().filter(|a| !unread || !a.is_read).collect();
            emit(json, serde_json::json!({"ok": true, "feed": f.name, "count": items.len(), "items": items}), || {
                for a in &items { println!("{}  {}  {}  {}", if a.is_read { " " } else { "*" }, qbt::ymd(a.ts), a.id, a.title); }
            });
        }
        RssCmd::Grab { name, article_id } => {
            let feeds = c.rss_feeds()?;
            let f = find_feed(&feeds, &name)?;
            let Some(a) = f.articles.iter().find(|a| a.id == article_id) else {
                return Err(AppError::new(Code::NotFound, format!("no item with id '{article_id}' in feed '{}'", f.name))
                    .hint(format!("fbittorrent rss items \"{}\" --json | jq '.items[].id'", f.name)).into());
            };
            let url = if a.torrent_url.is_empty() { &a.link } else { &a.torrent_url };
            c.add_url(url, &AddOptions {
                origin: None,
                sequential: cfg.native() || cfg.sequential, first_last: cfg.native() || cfg.first_last, seed: cfg.seed_by_default, stopped: false,
                save_path: cfg.save_path.clone(), category: cfg.category.clone(),
            })?;
            c.rss_mark_read(&f.name, Some(&a.id))?;
            if cfg.notify && !json && agent::stdout_is_tty() { notify("Torrent added", &a.title, false); }
            emit(json, serde_json::json!({"ok": true, "action": "rss-grab", "hash": magnet_hash(url), "name": a.title, "feed": f.name}), || println!("added: {}", a.title));
        }
    }
    Ok(Code::Ok)
}

pub(super) fn shows_cmd(cfg: &Config, c: &Client, sub: ShowsCmd, json: bool) -> Result<Code> {
    use shows::{EpState, Shows};
    let mut all = Shows::load()?;
    let print_eps = |s: &shows::Show, eps: &[&shows::Episode]| {
        for e in eps {
            let mark = match s.state(&e.key) { EpState::New => "*", EpState::Grabbed => "↓", EpState::Seen => " " };
            println!("{mark} {}  {:>5}  {:>9}  {:>4} seeds  {}", e.key, e.quality, human_bytes(e.size), e.seeds, e.title);
        }
    };
    match sub {
        ShowsCmd::List => {
            emit(json, serde_json::json!({"ok": true, "count": all.shows.len(), "new": all.new_total(), "path": shows::path(),
                "shows": all.shows.iter().map(|s| s.to_json()).collect::<Vec<_>>()}), || {
                if all.shows.is_empty() { println!("no shows followed yet: fbittorrent shows follow \"<title or tt id>\""); }
                for s in &all.shows {
                    let checked = if s.last_checked > 0 { qbt::ymd(s.last_checked) } else { "never".into() };
                    println!("{:<10}  {:<40} {:>3} new / {:>3} eps  checked {}{}", s.imdb, short(&s.label(), 40), s.new_count(), s.episodes.len(), checked,
                        s.error.as_deref().map(|e| format!("  [error: {e}]")).unwrap_or_default());
                }
            });
        }
        ShowsCmd::Follow { query, quality } => {
            let q = query.join(" ");
            if q.trim().is_empty() { return Err(AppError::new(Code::Usage, "give a title or an IMDb id").hint("fbittorrent shows follow \"The Bear\"").into()); }
            let t = shows::resolve_title(&q)?;
            let added = all.follow(&t);
            let s = all.get_mut(&t.id).unwrap();
            if quality.is_some() { s.quality = quality; }
            let fresh = shows::refresh(s, &cfg.show_quality).ok();
            let (label, n_new, n_eps) = (s.label(), s.new_count(), s.episodes.len());
            let warning = s.error.clone();
            let catalogue_complete = s.catalogue_complete;
            all.save()?;
            emit(json, serde_json::json!({"ok": true, "action": "shows-follow", "added": added, "imdb": t.id, "title": t.name, "year": t.year,
                "kind": t.kind, "episodes": n_eps, "new": n_new, "fetched": fresh.is_some(), "warning": warning, "catalogue_complete": catalogue_complete}), || {
                    println!("{} {label} [{}]: {n_eps} releases on EZTV, {n_new} new", if added { "following" } else { "already following" }, t.id);
                    if let Some(warning) = &warning { println!("warning: {warning}"); }
                });
        }
        ShowsCmd::Unfollow { show } => {
            let id = all.find(&show)?.imdb.clone();
            let s = all.unfollow(&id).unwrap();
            all.save()?;
            emit(json, serde_json::json!({"ok": true, "action": "shows-unfollow", "imdb": s.imdb, "title": s.title}), || println!("unfollowed {}", s.label()));
        }
        ShowsCmd::Check { show } => {
            let ids: Vec<String> = match show {
                Some(sel) => vec![all.find(&sel)?.imdb.clone()],
                None => all.shows.iter().map(|s| s.imdb.clone()).collect(),
            };
            let mut report = Vec::new();
            let mut failed = 0;
            for id in &ids {
                let s = all.get_mut(id).unwrap();
                let fresh = match shows::refresh(s, &cfg.show_quality) { Ok(n) => n, Err(_) => { failed += 1; 0 } };
                report.push((s.imdb.clone(), s.label(), fresh, s.new_count(), s.error.clone()));
            }
            all.save()?;
            emit(json, serde_json::json!({"ok": true, "action": "shows-check", "checked": ids.len(), "failed": failed, "new": all.new_total(),
                "shows": report.iter().map(|(id, l, f, n, e)| serde_json::json!({"imdb": id, "title": l, "fresh": f, "new": n, "error": e})).collect::<Vec<_>>()}), || {
                for (_, l, f, n, e) in &report {
                    match e { Some(e) => println!("{l}: {e}"), None => println!("{l}: {f} fresh, {n} new") }
                }
                for s in all.shows.iter().filter(|s| ids.contains(&s.imdb)) {
                    let eps: Vec<&shows::Episode> = s.episodes.iter().filter(|e| s.is_new(&e.key)).collect();
                    if eps.is_empty() { continue; }
                    println!("\n{}", s.label());
                    print_eps(s, &eps);
                }
            });
            if failed > 0 && failed == ids.len() { return Ok(Code::Upstream); }
        }
        ShowsCmd::Episodes { show, new } => {
            let s = all.find(&show)?;
            let eps: Vec<&shows::Episode> = s.episodes.iter().filter(|e| !new || s.is_new(&e.key)).collect();
            emit(json, serde_json::json!({"ok": true, "imdb": s.imdb, "title": s.title, "last_checked": s.last_checked, "count": eps.len(),
                "warning": s.error, "catalogue_complete": s.catalogue_complete,
                "episodes": eps.iter().map(|e| s.episode_json(e)).collect::<Vec<_>>()}), || {
                if eps.is_empty() { println!("{}: nothing listed{}", s.label(), if s.last_checked == 0 { " (never checked: fbittorrent shows check)" } else { "" }); }
                if let Some(warning) = &s.error { println!("warning: {warning}"); }
                print_eps(s, &eps);
            });
        }
        ShowsCmd::Grab { show, episode } => {
            let id = all.find(&show)?.imdb.clone();
            let s = all.get_mut(&id).unwrap();
            let Some(e) = s.episode(&episode).cloned() else {
                return Err(AppError::new(Code::NotFound, format!("{} has no episode {}", s.title, episode.to_uppercase()))
                    .hint(format!("fbittorrent shows episodes \"{}\" --json", s.imdb)).into());
            };
            c.add_url(&e.magnet, &AddOptions {
                origin: None,
                sequential: cfg.native() || cfg.sequential, first_last: cfg.native() || cfg.first_last, seed: cfg.seed_by_default, stopped: false,
                save_path: cfg.save_path.clone(), category: cfg.category.clone(),
            })?;
            s.mark_grabbed(&e.key, &e.magnet);
            let title = s.title.clone();
            all.save()?;
            if cfg.notify && !json && agent::stdout_is_tty() { notify("Torrent added", &e.title, false); }
            emit(json, serde_json::json!({"ok": true, "action": "shows-grab", "imdb": id, "show": title, "episode": e.key, "hash": magnet_hash(&e.magnet), "name": e.title}),
                || println!("added: {}", e.title));
        }
        ShowsCmd::Seen { show, episodes } => {
            let id = all.find(&show)?.imdb.clone();
            let s = all.get_mut(&id).unwrap();
            let mut changed = Vec::new();
            if episodes.iter().any(|e| e == "all") {
                let n = s.mark_all_seen();
                changed.push(serde_json::json!({"all": true, "marked": n}));
            } else {
                if episodes.is_empty() { return Err(AppError::new(Code::Usage, "give release keys like S01E02 or S01, or `all`").into()); }
                for k in &episodes {
                    let key = k.to_ascii_uppercase();
                    if s.episode(&key).is_none() {
                        return Err(AppError::new(Code::NotFound, format!("{} has no episode {key}", s.title))
                            .hint(format!("fbittorrent shows episodes \"{}\" --json", s.imdb)).into());
                    }
                    let seen = s.toggle_seen(&key);
                    changed.push(serde_json::json!({"episode": key, "seen": seen}));
                }
            }
            let (title, n_new) = (s.title.clone(), s.new_count());
            all.save()?;
            emit(json, serde_json::json!({"ok": true, "action": "shows-seen", "imdb": id, "show": title, "changed": changed, "new": n_new}),
                || println!("{title}: {n_new} new left"));
        }
    }
    Ok(Code::Ok)
}

pub(super) fn find_feed<'a>(feeds: &'a [qbt::RssFeed], name: &str) -> Result<&'a qbt::RssFeed> {
    if let Some(f) = feeds.iter().find(|f| f.name == name) { return Ok(f); }
    let low = name.to_lowercase();
    let m: Vec<&qbt::RssFeed> = feeds.iter().filter(|f| f.name.to_lowercase().contains(&low)).collect();
    match m.len() {
        1 => Ok(m[0]),
        0 => Err(AppError::new(Code::NotFound, format!("no feed named '{name}'"))
            .hint("fbittorrent rss list --json | jq -r '.feeds[].name'").into()),
        n => Err(AppError::new(Code::Ambiguous, format!("'{name}' matches {n} feeds: {}", m.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "))).into()),
    }
}
