use super::*;

fn app() -> TuiApp {
    TuiApp::new(Config::default(), Screen::Torrents, Shows::default())
}
#[test]
fn detach_never_requests_owner_shutdown() {
    for key in [
        KeyEvent::new(KeyCode::Char('q')),
        KeyEvent::new(KeyCode::Char('c')).with_modifiers(Modifiers::CTRL),
    ] {
        let mut app = app();
        assert!(matches!(app.on_key(key), Cmd::Quit));
        assert!(app.presence.is_none());
        assert_eq!(app.quit_notice, !key.modifiers.contains(Modifiers::CTRL));
    }
}
#[test]
fn uppercase_quit_requires_only_lowercase_y_when_busy() {
    for code in [
        KeyCode::Char('y'),
        KeyCode::Char('Y'),
        KeyCode::Char('q'),
        KeyCode::Char('Q'),
        KeyCode::Enter,
        KeyCode::Escape,
    ] {
        let mut app = app();
        app.torrents = vec![Torrent {
            state: "downloading".into(),
            ..Default::default()
        }];
        assert!(matches!(
            app.on_key(KeyEvent::new(KeyCode::Char('Q'))),
            Cmd::None
        ));
        let prompt = app.prompt.as_ref().unwrap();
        assert_eq!(prompt.kind, PromptKind::Shutdown);
        assert!(prompt
            .label
            .contains("Stop fbittorrent? 1 downloads pause until it starts again. y/N"));
        let command = app.on_key(KeyEvent::new(code));
        assert_eq!(
            matches!(command, Cmd::Task(..)),
            code == KeyCode::Char('y'),
            "key={code:?}"
        );
        assert!(app.prompt.is_none());
        assert!(app.presence.is_none());
    }
}
#[test]
fn two_failed_polls_show_stopped_and_enter_reconnects() {
    let mut app = app();
    app.update(Msg::Torrents(Err("closed".into())));
    assert!(!app.daemon_stopped);
    app.update(Msg::Torrents(Err("closed".into())));
    assert!(app.daemon_stopped);
    assert_eq!(
        actions::focused(&app),
        actions::Scope::Modal(actions::ModalKind::Stopped)
    );
    assert!(matches!(
        app.on_key(KeyEvent::new(KeyCode::Enter)),
        Cmd::Task(..)
    ));
    app.update(Msg::Torrents(Ok((vec![], TransferInfo::default()))));
    assert!(!app.daemon_stopped);
    assert_eq!(app.failed_polls, 0);
}
#[test]
fn notice_is_once_private_and_preserves_other_preferences() {
    use std::os::unix::fs::PermissionsExt;
    let directory = std::env::temp_dir().join(format!("fbt-p115-notices-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("notices.json");
    std::fs::write(&path, br#"{"first_quit_seen":false,"another_notice":true}"#).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(notices::first_quit(&directory).unwrap());
    assert!(!notices::first_quit(&directory).unwrap());
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({"first_quit_seen":true,"another_notice":true})
    );
    assert_eq!(
        std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for name in ["notices.json", "notices.lock"] {
        assert_eq!(
            std::fs::metadata(directory.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    std::fs::remove_file(&path).unwrap();
    assert!(notices::first_quit(&directory).unwrap());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn request_byte_cap_counts_json_escaping_before_queueing() {
    let healthy = serde_json::json!({"text":"\"".repeat(32_000)});
    let encoded = wire::Wire::encode(&healthy).unwrap();
    assert!(encoded.ends_with(b"\n") && encoded.len() <= crate::api::LINE_IN);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&encoded).unwrap(),
        healthy
    );
    let oversized = serde_json::json!({"text":"\"".repeat(40_000)});
    assert!(
        wire::Wire::encode(&oversized).is_err(),
        "escaped data must not bypass the byte cap"
    );
}
#[test]
fn domain_errors_keep_the_transport_distinct_from_malformed_replies() {
    let success = serde_json::json!({"id":2,"ok":true,"result":{}});
    assert!(wire::Wire::result("app.client", &success).is_ok());
    let missing = serde_json::json!({"id":2,"ok":false,"error":{"code":"not_found","message":"missing torrent","retryable":false}});
    let error = wire::Wire::result("torrents.get", &missing).unwrap_err();
    assert!(error.downcast_ref::<wire::RemoteError>().is_some());
    let invalid = serde_json::json!({"id":2,"ok":true,"result":{}});
    let error = wire::Wire::result("torrents.list", &invalid).unwrap_err();
    assert!(
        error.downcast_ref::<wire::RemoteError>().is_none(),
        "malformed data must close the connection"
    );
}

#[test]
fn protocol_mismatch_is_preserved_as_a_persistent_banner_before_decoding_foreign_shapes() {
    for frame in [
        serde_json::json!({"id":1,"ok":false,"error":{"code":"proto_unsupported","message":"proto unsupported","retryable":false,"details":{"server_proto":2}}}),
        serde_json::json!({"id":1,"ok":true,"result":{"proto":2,"a_future_shape":true}}),
    ] {
        let error=wire::Wire::result("hello",&frame).unwrap_err();
        let failure=Failure::capture(error);
        assert!(failure.permanent);
        assert!(failure.message.contains("This fbittorrent speaks API 1; the running fbittorrent speaks API 2"),"{}",failure.message);
    }
}
