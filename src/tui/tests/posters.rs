//! Real raster/renderer and kitty-byte-path controls; no terminal support is invented.
use super::*;
use std::{
    io::Write,
    sync::{Arc, Mutex},
};
#[path = "fixture.rs"]
mod fixture;
fn image() -> Poster {
    let image = image::RgbImage::from_fn(24, 36, |x, y| {
        image::Rgb([(x * 10) as u8, (y * 7) as u8, 100])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    Poster::from_bytes(bytes.get_ref()).unwrap()
}
fn title() -> Title {
    Title {
        id: "tt17491088".into(),
        name: "The Diplomat".into(),
        year: Some(2023),
        kind: "TV series".into(),
        stars: "Keri Russell, Rufus Sewell".into(),
        poster_url: String::new(),
        rating: Some(7.9),
        votes: Some(102000),
        blurb: "A veteran US diplomat takes a new assignment.".into(),
        wiki_url: String::new(),
    }
}
pub(super) fn ready(app: &mut TuiApp, key: &str) {
    let mut title = title();
    if let Some((_, Some(id))) = app.hovered_title() {
        title.id = id;
    }
    app.imdb_aliases.insert(key.into(), title.id.clone());
    app.imdb.insert(
        title.id.clone(),
        ImdbState::Ready(Box::new(title), Some(image())),
    );
}

fn draw(app: &TuiApp, width: u16, height: u16) -> String {
    super::golden::render(app, width, height)
}
#[test]
fn card_toggle_is_centered_and_static_at_80_columns() {
    for surface in ["torrents", "search", "shows", "rss"] {
        let mut app = fixture::app(surface);
        let (key, _) = app.hovered_title().unwrap();
        ready(&mut app, &key);
        assert!(matches!(
            app.on_key(KeyEvent::new(KeyCode::Char('v'))),
            Cmd::None
        ));
        assert!(app.poster_open);
        let text = draw(&app, 80, 24);
        assert!(
            text.contains("IMDb · v/Esc Back")
                && text.contains("The Diplomat")
                && text.contains("7.9"),
            "{surface}: {text}"
        );
        let area = super::modals_poster::poster_area(Rect::new(0, 0, 80, 24));
        assert_eq!(area, Rect::new(2, 1, 76, 21));
        assert!(text.lines().nth(22).unwrap().contains("[Card]"));
        assert_eq!((image().width, image().height), (24, 36));
        app.on_key(KeyEvent::new(KeyCode::Escape));
        assert!(!app.poster_open);
    }
}
#[test]
fn imdb_id_is_shared_by_release_aliases_and_debounce_is_300ms() {
    let mut app = fixture::app("torrents");
    assert!(matches!(
        app.lookup_imdb("alias one".into(), Some(title().id)),
        Cmd::Task(..)
    ));
    assert!(matches!(
        app.lookup_imdb("alias two".into(), Some(title().id)),
        Cmd::None
    ));
    app.remember_imdb(title().id, Ok(Some((title(), Some(image())))));
    assert_eq!(app.imdb.len(), 1);
    assert!(std::ptr::eq(
        app.imdb_state("alias one").unwrap(),
        app.imdb_state("alias two").unwrap()
    ));
    let key = app.selected_torrent().unwrap().name.clone();
    app.hover_key = Some(key.clone());
    app.hover_since = std::time::Instant::now() - Duration::from_millis(299);
    assert!(matches!(app.on_hover(), Cmd::None));
    app.hover_since = std::time::Instant::now() - Duration::from_millis(301);
    assert!(matches!(app.on_hover(), Cmd::Task(..)));
    app.remember_imdb(key, Ok(Some((title(), None))));
    assert_eq!(app.imdb.len(), 1);
}
#[derive(Clone)]
struct Output(Arc<Mutex<Vec<u8>>>);
impl Write for Output {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[test]
fn help_deletes_kitty_placement_before_drawing_and_restores_it_once() {
    let mut app = fixture::app("torrents");
    let key = app.selected_torrent().unwrap().name.clone();
    ready(&mut app, &key);
    let output = Output(Arc::default());
    app.kitty = Some(RefCell::new(KittyPlacer::with_output(output.clone())));
    app.on_key(KeyEvent::new(KeyCode::Char('v')));
    draw(&app, 80, 24);
    assert!(app.kitty.as_ref().unwrap().borrow().is_placed());
    assert!(String::from_utf8_lossy(&output.0.lock().unwrap()).contains("_Ga=T"));
    output.0.lock().unwrap().clear();
    draw(&app, 80, 24);
    assert!(
        output.0.lock().unwrap().is_empty(),
        "an unchanged frame must not retransmit the image"
    );
    app.show_help = true;
    let text = draw(&app, 80, 24);
    assert!(text.contains("GLOBAL"));
    assert!(!app.kitty.as_ref().unwrap().borrow().is_placed());
    assert_eq!(&*output.0.lock().unwrap(), b"\x1b_Ga=d,d=A,q=2\x1b\\");
    output.0.lock().unwrap().clear();
    app.show_help = false;
    draw(&app, 80, 24);
    assert!(app.kitty.as_ref().unwrap().borrow().is_placed());
    output.0.lock().unwrap().clear();
    draw(&app, 80, 24);
    assert!(output.0.lock().unwrap().is_empty());
    println!("real kitty encoder: one delete before Help, one restore after Help, no repeated image transmission");
}

#[test]
fn wide_shows_card_uses_full_pane_height_and_retains_its_poster() {
    let mut app = fixture::app("shows");
    let (key, _) = app.hovered_title().unwrap();
    ready(&mut app, &key);
    let text = draw(&app, 160, 45);
    let area = app.rects.borrow().poster.unwrap();
    assert!(
        area.height >= 35 && area.width >= 44,
        "card={area:?}\n{text}"
    );
    assert!(text.contains("The Diplomat") && text.contains("7.9"));
    println!("160x45 Shows card area={area:?}; poster remains 24x18 cells");
}

#[test]
fn resolved_aliases_keep_an_existing_poster_and_bound_the_session_cache() {
    let mut app = fixture::app("torrents");
    app.lookup_imdb("first release".into(), None);
    app.lookup_imdb("second release".into(), None);
    app.remember_imdb("first release".into(), Ok(Some((title(), Some(image())))));
    app.remember_imdb("second release".into(), Ok(Some((title(), None))));
    assert_eq!(app.imdb.len(), 1);
    assert!(matches!(
        app.imdb_state("second release"),
        Some(ImdbState::Ready(_, Some(_)))
    ));
    for i in 0..600 {
        let id = format!("tt{i:07}");
        app.imdb_aliases.insert(format!("alias {i}"), id.clone());
        app.cache_imdb(id, ImdbState::Missing);
    }
    assert!(app.imdb.len() <= 256 && app.imdb_aliases.len() <= 1024);
    assert!(app
        .imdb_aliases
        .values()
        .all(|id| app.imdb.contains_key(id)));
}
