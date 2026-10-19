//! History survives expiry; log filters and compact notification geometry are explicit.
use super::*;
#[path = "fixture.rs"]
mod fixture;
use crate::api::{LogEntry, LogLevel};
fn entry(seq: u64, level: LogLevel, source: &str, message: &str) -> LogEntry {
    LogEntry {
        seq,
        ts: 1_700_000_000,
        level,
        source: source.into(),
        message: message.into(),
        hash: None,
    }
}
#[test]
fn log_modal_keeps_failure() {
    let mut app = fixture::app("torrents");
    app.update(Msg::ActionDone(Err(
        "Could not pause: fixture permission denied".into(),
    )));
    assert_eq!(app.feedback.queue.visible_count(), 1);
    app.feedback.queue.visible_mut()[0].state.created_at =
        std::time::Instant::now() - Duration::from_secs(11);
    app.status_at = std::time::Instant::now() - Duration::from_secs(16);
    app.update(Msg::Hover);
    assert_eq!(app.feedback.queue.visible_count(), 0);
    assert!(app.status.is_empty());
    app.on_key(KeyEvent::new(KeyCode::Char('L')));
    let pane = super::golden::render(&app, 120, 30);
    assert!(pane.contains("Could not pause: fixture permission denied"));
    assert!(pane.contains("level: warn"));
    println!("10-second toast expired; 15-second status expired; L still shows the action failure\n{pane}");
}
#[test]
fn toasts_stack() {
    for (w, h) in [(80, 24), (120, 30), (160, 45)] {
        let mut app = fixture::app("torrents");
        app.set_success("first confirmed action");
        app.set_status("second action failed", true);
        let pane = super::golden::render(&app, w, h);
        let footer = app.rects.borrow().footer;
        let rects = app.feedback.rects.get();
        for rect in rects.into_iter().flatten() {
            assert_eq!(rect.height, 1);
            assert!(rect.width <= 60);
            assert!(rect.intersection(&footer).is_empty());
        }
        assert_eq!(rects[0].unwrap().y + 1, rects[1].unwrap().y);
        assert!(pane
            .lines()
            .nth(rects[0].unwrap().y as usize)
            .unwrap()
            .contains("first confirmed action"));
        assert!(pane
            .lines()
            .nth(rects[1].unwrap().y as usize)
            .unwrap()
            .contains("second action failed"));
        assert!(pane
            .lines()
            .nth(footer.y as usize)
            .unwrap()
            .contains("(?) Help"));
        println!("{w}x{h}: two toast rectangles={rects:?}; footer={footer:?}");
        app.set_success("third confirmed action");
        assert_eq!(app.feedback.queue.visible_count(), 2);
        assert_eq!(app.feedback.history.len(), 3);
    }
}
#[test]
fn log_filters_cycle_and_details_share_the_same_hash_scoped_history() {
    let mut app = fixture::app("torrents");
    app.store.logs.extend([
        entry(1, LogLevel::Info, "engine", "engine info control"),
        entry(2, LogLevel::Warn, "queue", "queue warning control"),
    ]);
    app.set_success("local confirmed action");
    app.on_key(KeyEvent::new(KeyCode::Char('L')));
    let pane = super::golden::render(&app, 120, 30);
    assert!(!pane.contains("engine info control"));
    assert!(pane.contains("queue warning control") && pane.contains("local confirmed action"));
    for level in ["info", "debug", "error", "warn"] {
        app.on_key(KeyEvent::new(KeyCode::Char('l')));
        let pane = super::golden::render(&app, 120, 30);
        assert!(pane.contains(&format!("level: {level}")));
        assert_eq!(app.feedback.log.borrow().viewer.len(),match level{"error"=>0,"warn"=>2,_=>3},"level filter applies to log rows; the status footer is independent");
    }
    for source in ["engine", "queue", "tui", "all"] {
        app.on_key(KeyEvent::new(KeyCode::Char('h')));
        let pane = super::golden::render(&app, 120, 30);
        assert!(pane.contains(&format!("source: {source}")));
    }
    app.on_key(KeyEvent::new(KeyCode::Escape));
    app.on_key(KeyEvent::new(KeyCode::Enter));
    app.details_tab(true);
    app.details_tab(true);
    assert_eq!(app.details.as_ref().unwrap().tab, DetailsTab::Log);
    let pane = super::golden::render(&app, 120, 30);
    assert!(pane.contains("[Log]") && pane.contains("local confirmed action"));
    assert!(!pane.contains("queue warning control"));
    app.on_key(KeyEvent::new(KeyCode::Char('[')));
    assert_eq!(app.details.as_ref().unwrap().tab, DetailsTab::Files);
    app.on_key(KeyEvent::new(KeyCode::Char(']')));app.on_key(KeyEvent::new(KeyCode::Char('L')));
    assert!(app.details.is_none(),"L closes either kind of log view");
}
#[test]
fn log_history_and_notifications_stay_bounded() {
    let mut feedback = feedback::Feedback::default();
    for index in 0..750 {
        feedback.push(&format!("{index}: {}\n", "x".repeat(3000)), true, None);
    }
    assert_eq!(feedback.history.len(), 500);
    assert!(feedback
        .history
        .iter()
        .all(|row| row.message.len() <= 2048 && !row.message.contains('\n')));
    assert_eq!(feedback.queue.visible_count(), 2);
    assert_eq!(feedback.queue.pending_count(), 0);
}
#[test]
fn log_modal_golden_120x30() {
    let mut app = fixture::app("torrents");
    app.store.logs.push_back(entry(
        1,
        LogLevel::Warn,
        "engine",
        "Tracker did not answer; retry scheduled",
    ));
    app.update(Msg::ActionDone(Err(
        "Could not pause: fixture permission denied".into(),
    )));
    app.feedback.history.back_mut().unwrap().ts = 1_700_000_001;
    app.on_key(KeyEvent::new(KeyCode::Char('L')));
    let actual = super::golden::render(&app, 120, 30);
    for text in ["Log · L/Esc Back", "level: warn", "source: all", "Tracker did not answer; retry scheduled", "Could not pause: fixture permission denied"] {
        assert!(actual.contains(text), "log lost {text:?}:\n{actual}");
    }
    assert_eq!(app.feedback.log.borrow().viewer.len(),2,"daemon and local failures share the viewer");
    assert_eq!(actual.lines().count(),30);
}

#[test]
fn incoming_logs_preserve_a_scrolled_viewport_and_reuse_unchanged_cache() {
    let mut app = fixture::app("torrents");
    for seq in 0..80 {
        app.store.logs.push_back(entry(
            seq,
            LogLevel::Error,
            "engine",
            &format!("retained message {seq:03}"),
        ));
    }
    app.on_key(KeyEvent::new(KeyCode::Char('L')));
    super::golden::render(&app, 120, 30);
    for _ in 0..5 {
        app.on_key(KeyEvent::new(KeyCode::Up));
    }
    let before = super::golden::render(&app, 120, 30);
    let first = before
        .lines()
        .find(|line| line.contains("retained message"))
        .unwrap();
    app.store.logs.push_back(entry(
        80,
        LogLevel::Error,
        "engine",
        "newly arrived message",
    ));
    app.feedback.dirty.set(true);
    let after = super::golden::render(&app, 120, 30);
    assert_eq!(
        first,
        after
            .lines()
            .find(|line| line.contains("retained message"))
            .unwrap()
    );
    assert!(!app.feedback.dirty.get());
    for _ in 0..32 {
        assert_eq!(super::golden::render(&app, 120, 30), after);
    }
    println!(
        "new log preserves the first visible line; 32 unchanged frames preserve the cached view"
    );
}

#[test]
fn confirmation_and_settings_remain_above_the_log(){
    let mut app=fixture::app("torrents");
    app.on_key(KeyEvent::new(KeyCode::Char('L')));app.on_key(KeyEvent::new(KeyCode::Char('Q')));
    assert_eq!(actions::focused(&app),actions::Scope::Prompt);
    app.on_key(KeyEvent::new(KeyCode::Escape));assert!(app.prompt.is_none()&&app.log_open);
    app.settings=fixture::app("settings").settings;
    assert_eq!(actions::focused(&app),actions::Scope::Modal(actions::ModalKind::Settings));
    assert!(super::golden::render(&app,120,30).contains("Download settings"));
}
