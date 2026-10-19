use super::*;
use serde_json::json;

#[test]
fn restart_class_list() {
    assert_eq!(view(&Config::default()).restart_class, RESTART_CLASS);
    for (key, value) in [
        ("peer_encryption", json!("require")),
        ("network_interface", json!("fixture0")),
        ("peer_port", json!(6881)),
    ] {
        let (_, result) =
            prepare(&Config::default(), json!({key:value}).as_object().unwrap()).unwrap();
        assert_eq!(result.applied, [key]);
        assert!(result.restart, "{key}");
        println!("{key}: durable acknowledgement restart=true");
    }
}
#[test]
fn invalid_value_names_key() {
    let bad = json!({
        "max_active_downloads":0, "peer_encryption":"bad", "network_interface":"../bad",
        "save_path":"", "seed_by_default":"yes", "autostart":1, "idle_exit_secs":-1,
        "notify":null, "timeout_secs":0, "handler":"exec", "open_command":" ",
        "clipboard_watch":0, "reduce_motion":null, "peer_port":65536, "show_quality":"",
        "download_limit_bps":1,"upload_limit_bps":1,"alt_download_limit_bps":1,
        "alt_upload_limit_bps":1,"alt_limits_on":0,
    });
    assert_eq!(bad.as_object().unwrap().len(), KEYS.len());
    for (key, value) in bad.as_object().unwrap() {
        let failure =
            prepare(&Config::default(), json!({key:value}).as_object().unwrap()).unwrap_err();
        assert_eq!(failure.code, ErrorCode::InvalidArgs);
        assert_eq!(failure.details.unwrap()["key"], *key);
        println!("{key}: invalid_args names exact key");
    }
    for key in ["restart_class", "password", "unknown"] {
        let failure =
            prepare(&Config::default(), json!({key:true}).as_object().unwrap()).unwrap_err();
        assert_eq!(failure.details.unwrap()["key"], key);
    }
}
#[test]
fn live_keys_apply_without_restart() {
    let patch = json!({"max_active_downloads":7,"save_path":"/tmp/fbt-settings",
        "seed_by_default":true,"autostart":false,"idle_exit_secs":0,"notify":false,
        "timeout_secs":30,"handler":"add","open_command":"mpv --keep-open=yes",
        "clipboard_watch":true,"reduce_motion":true,"show_quality":"720p",
        "download_limit_bps":65536,"upload_limit_bps":32768,"alt_download_limit_bps":16384,
        "alt_upload_limit_bps":16384,"alt_limits_on":true});
    let (cfg, reply) = prepare(&Config::default(), patch.as_object().unwrap()).unwrap();
    assert!(!reply.restart);
    assert_eq!(reply.applied.len(), patch.as_object().unwrap().len());
    let (roundtrip, duplicate) = prepare(&cfg, patch.as_object().unwrap()).unwrap();
    assert!(duplicate.applied.is_empty());
    assert!(!duplicate.restart);
    let (cleared, _) = prepare(
        &roundtrip,
        json!({"upload_limit_bps":null}).as_object().unwrap(),
    )
    .unwrap();
    assert_eq!(cleared.upload_limit_bps, None);
    assert_eq!(view(&cfg).handler, Handler::Add);
    println!(
        "{} live keys applied; exact repeat is a no-op; null clears optional limit",
        reply.applied.len()
    );
}
#[test]
fn settings_fixture_roundtrips() {
    crate::api::fixtures::check_group(
        "settings",
        include_str!("../../../../contracts/ipc/v1/settings.ndjson"),
    )
    .unwrap();
}
