use super::*;
use std::os::unix::fs::PermissionsExt;
fn indexer() -> Indexer {
    Indexer {
        name: "fixture".into(),
        url: "http://127.0.0.1:1234/api".into(),
        api_key_file: None,
    }
}
#[test]
fn parse_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../agent_ergonomics_audit/audit/regression_tests/fixtures/torznab/query.json"
    ))
    .unwrap();
    let rows = xml::parse(fixture["xml"].as_str().unwrap().as_bytes(), &indexer()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].seeders, 2);
    assert_eq!(rows[0].file_size, 262144);
    assert_eq!(normalize::btih(&rows[0].file_url), Some("1".repeat(40)));
    println!(
        "real Torznab fixture maps title, magnet, size and seeders to shared normalized model"
    );
}
#[test]
fn xml_enclosure_attributes_dates_and_unknowns() {
    let source=br#"<rss xmlns:torznab="urn:fixture"><channel><item><title>A &amp; B</title><pubDate>09 Oct 2026 12:00:00 +0000</pubDate><enclosure url="http://127.0.0.1:1234/file.torrent" length="1234"/><torznab:attr name="seeders" value="9"/></item><item><title>Unknown</title><link>magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</link></item></channel></rss>"#;
    let rows = xml::parse(source, &indexer()).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].file_name, "A & B");
    assert_eq!(rows[0].pub_date, 1791547200);
    assert_eq!(rows[0].seeders, 9);
    assert_eq!(rows[1].seeders, -1);
    assert!(normalize::item(&rows[1]).seeds.is_none());
    assert!(xml::parse(
        b"<!DOCTYPE rss [<!ENTITY leak SYSTEM 'file:///etc/passwd'>]><rss/>",
        &indexer()
    )
    .is_err());
    println!("enclosures, XML entities, dates and absent measurements; document types refused");
}
#[test]
fn keyed_non_loopback_http_is_refused_before_network() {
    for (url, allowed) in [
        ("http://example.invalid/api", false),
        ("https://example.invalid/api", true),
        ("http://127.0.0.1:1234/api", true),
        ("http://[::1]:1234/api", true),
        ("http://localhost:1234/api", true),
        ("https://user:secret@example.invalid/api", false),
        ("https://example.invalid/api?apikey=inline", false),
    ] {
        assert_eq!(config::endpoint(url, true).is_ok(), allowed);
        println!("scheme/host/inline credential validation allowed={allowed}");
    }
}
#[test]
fn credentials_refuse_wrong_modes_symlinks_and_echoes() {
    let path = std::env::temp_dir().join(format!(
        "fbt-indexer-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    let key = path.join("key");
    std::fs::write(&key, b"fixture-key\n").unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut entry = indexer();
    entry.api_key_file = Some(key.clone());
    assert!(entry.validate().is_ok());
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    let error = entry.validate().unwrap_err().to_string();
    assert!(!error.contains("fixture-key"));
    assert_eq!(
        std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o644
    );
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let symlink = path.join("link");
    std::os::unix::fs::symlink(&key, &symlink).unwrap();
    entry.api_key_file = Some(symlink);
    assert!(entry.validate().is_err());
    let cfg = path.join("indexers.toml");
    std::fs::write(
        &cfg,
        toml::to_string(&config::Config {
            torznab: vec![indexer()],
        })
        .unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(config::read(&cfg).unwrap().torznab.len(), 1);
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(config::read(&cfg).is_err());
    assert_eq!(
        std::fs::metadata(&cfg).unwrap().permissions().mode() & 0o777,
        0o644
    );
    std::fs::remove_dir_all(path).unwrap();
    println!("config/key files enforce 0600 and O_NOFOLLOW, refuse without repairing; errors contain no credential");
}
