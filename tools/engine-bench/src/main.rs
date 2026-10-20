//! fbt-bench-seed: deterministic payloads, metainfo and a loopback seed swarm
//! for measuring the fbittorrent engine. Measurement tooling only.
//!
//!   make    --out F --name N --size B --piece-len B --key K --tracker URL [--cache D] [--public]
//!   serve   --torrents DIR --peers N --tracker-port P [--rate BYTES_PER_S_PER_CONN] [--ready F]
//!   write   --file F --size B --key K
//!   verify  --file F --size B --key K

mod bencode;
mod payload;
mod seed;
mod sha1;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn args() -> (String, HashMap<String, String>) {
    let mut argv = std::env::args().skip(1);
    let command = argv.next().unwrap_or_default();
    let mut options = HashMap::new();
    let rest: Vec<String> = argv.collect();
    let mut i = 0;
    while i < rest.len() {
        let key = rest[i].trim_start_matches("--").to_string();
        let value = rest.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
        i += if value.is_some() { 2 } else { 1 };
        options.insert(key, value.unwrap_or_else(|| "true".into()));
    }
    (command, options)
}

fn need<'a>(options: &'a HashMap<String, String>, key: &str) -> &'a str {
    options.get(key).map(String::as_str).unwrap_or_else(|| {
        eprintln!("missing --{key}");
        std::process::exit(2)
    })
}

fn number(options: &HashMap<String, String>, key: &str) -> u64 {
    let text = need(options, key);
    let parsed = if let Some(hex) = text.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else {
        text.parse().ok()
    };
    parsed.unwrap_or_else(|| {
        eprintln!("--{key} must be a number");
        std::process::exit(2)
    })
}

fn main() {
    let (command, options) = args();
    let prefix = options.get("prefix").map(|path| std::fs::read(path).expect("read media prefix")).unwrap_or_default();
    let result: Result<(), String> = match command.as_str() {
        "make" => {
            let spec = payload::Spec {
                name: need(&options, "name").to_string(),
                size: number(&options, "size"),
                piece_len: number(&options, "piece-len") as u32,
                key: number(&options, "key"),
                private: !options.contains_key("public"),
                prefix: prefix.clone(),
            };
            let cache = options.get("cache").map(PathBuf::from);
            payload::piece_hashes(&spec, cache.as_deref())
                .map_err(|e| e.to_string())
                .and_then(|pieces| {
                    let info = payload::info_bytes(&spec, &pieces);
                    let torrent = payload::torrent_bytes(&spec, &info, need(&options, "tracker"));
                    std::fs::write(need(&options, "out"), &torrent).map_err(|e| e.to_string())?;
                    println!(
                        "{{\"info_hash\":\"{}\",\"pieces\":{},\"info_bytes\":{}}}",
                        payload::hex(&sha1::digest(&info)),
                        spec.piece_count(),
                        info.len()
                    );
                    Ok(())
                })
        }
        "serve" => {
            let dir = Path::new(need(&options, "torrents"));
            let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
                .map_err(|e| e.to_string())
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1)
                })
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "torrent"))
                .collect();
            paths.sort();
            paths
                .iter()
                .map(|p| seed::Hosted::load(p))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|torrents| {
                    let rate = options.get("rate").map_or(0, |_| number(&options, "rate"));
                    seed::Server::new(torrents, rate, options.contains_key("probe-upload"))
                        .run(
                            number(&options, "peers") as usize,
                            number(&options, "tracker-port") as u16,
                            options.get("ready").map(Path::new),
                        )
                        .map_err(|e| e.to_string())
                })
        }
        "write" => payload::write_file(
            Path::new(need(&options, "file")),
            number(&options, "size"),
            number(&options, "key"),
            &prefix,
        )
        .map_err(|e| e.to_string()),
        "verify" => {
            match payload::verify_file(Path::new(need(&options, "file")), number(&options, "size"), number(&options, "key"), &prefix) {
                Ok(None) => {
                    println!("{{\"ok\":true}}");
                    Ok(())
                }
                Ok(Some(at)) => {
                    println!("{{\"ok\":false,\"first_difference\":{at}}}");
                    std::process::exit(1)
                }
                Err(e) => Err(e.to_string()),
            }
        }
        _ => Err(format!("unknown command {command:?}; see the source header for usage")),
    };
    if let Err(error) = result {
        eprintln!("fbt-bench-seed {command}: {error}");
        std::process::exit(1);
    }
}
