//! Search multi-select: marks, Shift+arrows, Esc, one confirmation per batch.
use super::*;
use serde_json::json;

fn app(names: &[&str]) -> TuiApp {
    let mut app = TuiApp::new(Config::default(), Screen::Search, Shows::default());
    app.kitty = None;
    app.search_focus_input = false;
    app.search_input.set_focused(false);
    app.search_wire = names.iter().enumerate().map(|(i, name)| serde_json::from_value(json!({
        "key": format!("{:040x}", i + 1), "name": name, "providers": ["fixture"],
        "size": 1024, "seeds": 10, "magnet": format!("magnet:?xt=urn:btih:{:040x}", i + 1),
    })).unwrap()).collect();
    app.search_results = app.search_wire.iter().map(remote::search::display).collect();
    app.search_list.borrow_mut().select(Some(0));
    app
}

fn press(app: &mut TuiApp, code: KeyCode) -> Cmd<Msg> { app.on_key(KeyEvent::new(code)) }
fn shift(app: &mut TuiApp, code: KeyCode) -> Cmd<Msg> {
    app.on_key(KeyEvent::new(code).with_modifiers(Modifiers::SHIFT))
}
fn marked(app: &TuiApp) -> Vec<String> {
    app.search_results.iter().filter(|r| app.search_mark_state(r).0).map(|r| r.file_name.clone()).collect()
}
fn cursor(app: &TuiApp) -> Option<usize> { app.search_list.borrow().selected() }

#[test]
fn space_marks_the_current_result_and_moves_on_and_space_again_unmarks() {
    let mut app = app(&["One", "Two", "Three"]);
    press(&mut app, KeyCode::Char(' '));
    assert_eq!((marked(&app), cursor(&app)), (vec!["One".to_string()], Some(1)));
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(marked(&app), ["One", "Two"]);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(marked(&app), ["One"], "Space toggles a marked result off");
    assert!(app.prompt.is_none() && app.search_adding.is_empty(), "marking alone adds nothing");
}

#[test]
fn shift_down_and_up_mark_a_contiguous_run() {
    let mut app = app(&["One", "Two", "Three", "Four"]);
    shift(&mut app, KeyCode::Down);
    assert_eq!((marked(&app), cursor(&app)), (vec!["One".to_string(), "Two".into()], Some(1)));
    shift(&mut app, KeyCode::Down);
    assert_eq!((marked(&app), cursor(&app)), (vec!["One".to_string(), "Two".into(), "Three".into()], Some(2)));
    press(&mut app, KeyCode::Escape);
    assert!(marked(&app).is_empty(), "Esc clears the marks first");
    shift(&mut app, KeyCode::Up);
    assert_eq!((marked(&app), cursor(&app)), (vec!["Two".to_string(), "Three".into()], Some(1)));
}

#[test]
fn enter_with_marks_asks_once_then_adds_every_marked_result() {
    let mut app = app(&["One", "Two", "Three"]);
    shift(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    let prompt = app.prompt.as_ref().expect("Enter with marks asks first");
    assert_eq!(prompt.kind, PromptKind::ConfirmSearchBatch { count: 2 });
    assert!(prompt.label.contains("Download 2 results") && prompt.label.contains("no upload"), "{}", prompt.label);
    assert!(app.search_adding.is_empty(), "nothing is added before the confirmation");
    press(&mut app, KeyCode::Char('y'));
    assert!(app.prompt.is_none() && marked(&app).is_empty());
    let adding: Vec<_> = app.search_results.iter().filter(|r| app.search_mark_state(r).1).map(|r| r.file_name.clone()).collect();
    assert_eq!(adding, ["One", "Two"], "both marked results are in flight; the third is untouched");
}

#[test]
fn enter_in_the_batch_prompt_confirms_like_y() {
    let mut app = app(&["One", "Two"]);
    shift(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_none() && marked(&app).is_empty());
    assert_eq!(app.search_adding.len(), 2);
}

#[test]
fn escape_in_the_prompt_cancels_and_keeps_the_marks() {
    let mut app = app(&["One", "Two"]);
    shift(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Escape);
    assert!(app.prompt.is_none() && app.search_adding.is_empty());
    assert_eq!(marked(&app), ["One", "Two"]);
}

#[test]
fn grab_receipts_mark_rows_added_and_added_rows_cannot_be_marked_again() {
    let mut app = app(&["One", "Two"]);
    shift(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('y'));
    let one = screens_search::search_identity(&app.search_results[0]);
    let two = screens_search::search_identity(&app.search_results[1]);
    app.update(Msg::SearchGrabbed { connection: None, added: vec![one], failed: vec![(two, "Two".into(), "upstream".into())] });
    assert_eq!(app.search_mark_state(&app.search_results[0]), (false, false, true));
    assert_eq!(app.search_mark_state(&app.search_results[1]), (false, false, false), "a failed grab can be tried again");
    let pane = golden::render(&app, 120, 30);
    assert!(pane.contains("+ In list · One"), "{pane}");
    app.search_list.borrow_mut().select(Some(0));
    press(&mut app, KeyCode::Char(' '));
    assert!(marked(&app).is_empty(), "an added result is not selectable");
}

#[test]
fn single_enter_without_marks_still_adds_directly() {
    let mut app = app(&["One", "Two"]);
    press(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_none());
    assert_eq!(app.search_mark_state(&app.search_results[0]), (false, true, false));
}

#[test]
fn shift_arrows_are_documented_for_both_surfaces() {
    let keys = actions::keymap("all");
    let text = keys.to_string();
    assert!(text.contains("search.select_down") && text.contains("Shift+↓"), "{text}");
    assert!(text.contains("search.select_up") && text.contains("Shift+↑"));
}
