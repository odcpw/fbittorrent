//! Colour-independent feedback distinguishes errors, confirmed success and hints.
use super::*;
#[path = "fixture.rs"]
mod fixture;
#[test]
fn low_chroma_status_markers_never_turn_pending_information_into_success() {
    for (name, source, low) in [
        (
            "owner",
            include_str!("../../../contracts/ui/themes/owner.toml"),
            true,
        ),
        (
            "tokyo-night",
            include_str!("../../../contracts/ui/themes/tokyo-night.toml"),
            false,
        ),
    ] {
        let mut app = fixture::app("torrents");
        app.theme = Theme::from_toml(source).unwrap();
        app.set_status("Waiting for confirmation", false);
        let text = super::golden::render(&app, 80, 24);
        assert_eq!(text.lines().last().unwrap(), "Waiting for confirmation");
        app.update(Msg::ActionDone(Ok("torrent paused".into())));
        let text = super::golden::render(&app, 80, 24);
        let line = text.lines().last().unwrap();
        assert_eq!(
            line,
            if low {
                "✓ torrent paused"
            } else {
                "torrent paused"
            }
        );
        app.update(Msg::RssAction {
            revision: app.rss_revision,
            label: "feed removed".into(),
            result: Ok(vec![]),
            error: None,
        });
        let text = super::golden::render(&app, 80, 24);
        assert_eq!(
            text.lines().last().unwrap(),
            if low {
                "✓ feed removed"
            } else {
                "feed removed"
            }
        );
        app.set_status("could not restore: bad cache", true);
        let text = super::golden::render(&app, 80, 24);
        let line = text.lines().last().unwrap();
        assert_eq!(
            line,
            if low {
                "! could not restore: bad cache"
            } else {
                "could not restore: bad cache"
            }
        );
        println!("theme={name} status={line:?}; confirmed success and information remain distinct");
    }
}

#[test]
fn modal_errors_use_the_theme_role_and_keep_a_worded_marker() {
    for (source, low) in [
        (
            include_str!("../../../contracts/ui/themes/owner.toml"),
            true,
        ),
        (
            include_str!("../../../contracts/ui/themes/tokyo-night.toml"),
            false,
        ),
    ] {
        let theme = Theme::from_toml(source).unwrap();
        assert_eq!(
            theme.err(),
            if low {
                theme.text().bold().reverse()
            } else {
                Style::new().fg(theme.red)
            }
        );
        if low {
            assert_eq!(theme.ok(), theme.text());
            assert_eq!(theme.warn(), theme.text());
        }
        for surface in ["files", "providers", "settings"] {
            let mut app = fixture::app(surface);
            app.theme = theme.clone();
            match surface {
                "files" => app.files.as_mut().unwrap().error = "cannot read payload".into(),
                "providers" => {
                    app.providers.as_mut().unwrap().error = "cannot read providers".into()
                }
                _ => app.settings.as_mut().unwrap().error = "cannot save settings".into(),
            }
            let frame = super::golden::render(&app, 80, 24);
            assert!(
                frame.contains(if low { "! cannot" } else { "cannot" }),
                "{surface}: {frame}"
            );
            assert_eq!(frame.contains("! cannot"), low, "{surface}");
        }
    }
}
