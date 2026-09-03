//! Client tests against a tiny in-process HTTP server that plays qBittorrent.
//! No network, no real backend: each test declares the routes it expects.

use crate::config::Config;
use crate::qbt::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
struct Captured {
    path: String,
    headers: String,
    body: Vec<u8>,
}

struct Route {
    prefix: &'static str,
    status: u16,
    body: String,
    set_cookie: Option<&'static str>,
}

fn route(prefix: &'static str, status: u16, body: impl Into<String>) -> Route {
    Route { prefix, status, body: body.into(), set_cookie: None }
}

/// Serve `routes` on a random loopback port; returns the base URL and the request log.
fn serve(routes: Vec<Route>) -> (String, Arc<Mutex<Vec<Captured>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let (head_end, content_len) = loop {
                let n = match s.read(&mut tmp) { Ok(0) | Err(_) => break (buf.len(), 0), Ok(n) => n };
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                    let cl = head.lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1)).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    if buf.len() >= pos + 4 + cl { break (pos, cl); }
                }
            };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let path = head.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("").to_string();
            let body = buf[(head_end + 4).min(buf.len())..(head_end + 4 + content_len).min(buf.len())].to_vec();
            log2.lock().unwrap().push(Captured { path: path.clone(), headers: head.clone(), body });
            let r = routes.iter().find(|r| path.starts_with(r.prefix));
            let (status, body, cookie) = match r {
                Some(r) => (r.status, r.body.clone(), r.set_cookie),
                None => (404, "Not Found".to_string(), None),
            };
            let cookie_line = cookie.map(|c| format!("Set-Cookie: {c}; HttpOnly; path=/\r\n")).unwrap_or_default();
            let resp = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: application/json\r\n{cookie_line}Connection: close\r\n\r\n{body}", body.len());
            let _ = s.write_all(resp.as_bytes());
            let _ = s.flush();
        }
    });
    (format!("http://127.0.0.1:{port}"), log)
}

fn client(url: &str) -> Client {
    let cfg = Config { url: url.to_string(), timeout_secs: 5, ..Config::default() };
    Client::from_config(&cfg)
}

fn opts(seed: bool) -> AddOptions {
    AddOptions { sequential: true, first_last: true, seed, stopped: true, save_path: None, category: None }
}

fn body_str(c: &Captured) -> String { String::from_utf8_lossy(&c.body).to_string() }

#[test]
fn torrents_parse_the_fields_the_ui_depends_on() {
    let json = r#"[{"hash":"abc","name":"Sintel","state":"stoppedDL","progress":0.5,"size":100,"up_limit":32768,"ratio_limit":0.0,"tags":"","seq_dl":true,"f_l_piece_prio":true},
                   {"hash":"def","name":"Seeded","state":"uploading","progress":1.0,"size":1,"up_limit":0,"ratio_limit":-2.0,"tags":"seed, other"}]"#;
    let (url, _) = serve(vec![route("/api/v2/torrents/info", 200, json)]);
    let ts = client(&url).torrents().unwrap();
    assert_eq!(ts.len(), 2);
    assert!(!ts[0].seed_allowed(), "32 KiB cap and no tag means upload off");
    assert!(ts[0].is_stopped() && ts[0].seq_dl && ts[0].f_l_piece_prio);
    assert!(ts[1].seed_allowed() && ts[1].has_tag("seed") && ts[1].is_seeding());
}

#[test]
fn add_url_sends_the_no_upload_defaults() {
    let (url, log) = serve(vec![route("/api/v2/torrents/add", 200, "Ok.")]);
    client(&url).add_url("magnet:?xt=urn:btih:0000000000000000000000000000000000000000", &opts(false)).unwrap();
    let b = body_str(&log.lock().unwrap()[0]);
    for needle in ["sequentialDownload=true", "firstLastPiecePrio=true", "stopped=true", "upLimit=32768", "ratioLimit=0", "shareLimitAction=Stop"] {
        assert!(b.contains(needle), "missing {needle} in {b}");
    }
    assert!(!b.contains("tags=seed"));
}

#[test]
fn add_url_with_seed_tags_instead_of_capping() {
    let (url, log) = serve(vec![route("/api/v2/torrents/add", 200, "Ok.")]);
    client(&url).add_url("magnet:?xt=urn:btih:0000000000000000000000000000000000000000", &opts(true)).unwrap();
    let b = body_str(&log.lock().unwrap()[0]);
    assert!(b.contains("tags=seed") && !b.contains("upLimit="), "{b}");
}

#[test]
fn add_accepts_the_json_summary_of_qbittorrent_5() {
    let (url, _) = serve(vec![route("/api/v2/torrents/add", 200, r#"{"success_count":1,"failure_count":0,"pending_count":0}"#)]);
    client(&url).add_url("magnet:?xt=urn:btih:0000000000000000000000000000000000000000", &opts(false)).unwrap();
    let (url, _) = serve(vec![route("/api/v2/torrents/add", 200, r#"{"success_count":0,"failure_count":1}"#)]);
    let err = client(&url).add_url("magnet:?x", &opts(false)).unwrap_err().to_string();
    assert!(err.contains("could not add"), "{err}");
}

#[test]
fn add_file_posts_a_multipart_body_with_the_torrent() {
    let (url, log) = serve(vec![route("/api/v2/torrents/add", 200, "Ok.")]);
    let bytes = b"d8:announce3:abc4:infod4:name1:x12:piece lengthi1e6:pieces0:6:lengthi0eee".to_vec();
    client(&url).add_file("my file.torrent", &bytes, &opts(false)).unwrap();
    let c = log.lock().unwrap()[0].clone();
    assert!(c.headers.to_ascii_lowercase().contains("multipart/form-data; boundary="), "{}", c.headers);
    let b = body_str(&c);
    assert!(b.contains("name=\"torrents\"; filename=\"my file.torrent\""), "{b}");
    assert!(b.contains("name=\"upLimit\"\r\n\r\n32768"), "{b}");
    assert!(c.body.windows(bytes.len()).any(|w| w == &bytes[..]), "torrent bytes not in body");
}

#[test]
fn http_errors_become_actionable_messages() {
    let (url, _) = serve(vec![route("/api/v2/torrents/start", 404, "Not Found")]);
    let err = client(&url).start(&["abc".into()]).unwrap_err().to_string();
    assert!(err.contains("no endpoint") && err.contains("torrents/start"), "{err}");
    let (url, _) = serve(vec![route("/api/v2/torrents/info", 403, "Forbidden")]);
    let err = client(&url).torrents().unwrap_err().to_string();
    assert!(err.contains("refused"), "{err}");
}

#[test]
fn the_session_cookie_is_carried_to_the_next_request() {
    // Search jobs are per-session in qBittorrent, so losing the cookie breaks search.
    let mut first = route("/api/v2/app/version", 200, "v5.2.3");
    first.set_cookie = Some("QBT_SID_8080=sid123");
    let (url, log) = serve(vec![first, route("/api/v2/torrents/info", 200, "[]")]);
    let c = client(&url);
    c.version().unwrap();
    c.torrents().unwrap();
    let second = log.lock().unwrap()[1].clone();
    assert!(second.headers.contains("QBT_SID_8080=sid123"), "cookie not sent: {}", second.headers);
}

#[test]
fn reconcile_only_touches_untagged_uploading_torrents() {
    let info = r#"[{"hash":"aaaa","name":"fresh","progress":0.1,"seq_dl":true,"f_l_piece_prio":true,"up_limit":0,"tags":"","added_on":9999999999},
                   {"hash":"bbbb","name":"seeded","progress":0.1,"seq_dl":true,"f_l_piece_prio":true,"up_limit":0,"tags":"seed","added_on":9999999999}]"#;
    let (url, log) = serve(vec![
        route("/api/v2/torrents/info", 200, info),
        route("/api/v2/torrents/setUploadLimit", 200, ""),
        route("/api/v2/torrents/setShareLimits", 200, ""),
        route("/api/v2/torrents/removeTags", 200, ""),
        route("/api/v2/torrents/stop", 200, ""),
    ]);
    let n = client(&url).reconcile(true, true, false, 0).unwrap();
    assert_eq!(n, 1, "only the untagged torrent gets the cap");
    let bodies: Vec<String> = log.lock().unwrap().iter().filter(|c| c.path.contains("setUploadLimit")).map(body_str).collect();
    assert_eq!(bodies.len(), 1);
    assert!(bodies[0].contains("hashes=aaaa") && !bodies[0].contains("bbbb"), "{bodies:?}");
}

#[test]
fn rfc2822_dates_parse_and_sort_newest_first() {
    let a = parse_rfc2822("02 Sep 2026 03:58:00 +0000");
    let b = parse_rfc2822("Tue, 01 Sep 2026 23:58:00 -0100");
    assert!(a > b);
    assert_eq!(ymd(a), "2026-09-02");
    assert_eq!(parse_rfc2822("garbage"), 0);
}
