    use super::*;
    #[path="fixture.rs"]mod fixture;
    #[test] fn history_render_tracks_data_and_preserves_navigation() {
        let mut app=fixture::app("torrents");app.online=true;app.store.stale=false;
        let summary=include_str!("../../../contracts/ipc/v1/subscribe.ndjson").lines().filter_map(|line|serde_json::from_str::<serde_json::Value>(line).ok()).find(|r|r["frame"]["topic"]=="summary").unwrap();
        app.store.summary=Some(serde_json::from_value(summary["frame"]["data"].clone()).unwrap());
        app.store.summary.as_mut().unwrap().history=Some((1..=60).map(|n|n*4096).collect());
        let ramp=crate::tui::golden::render(&app,120,30);let header=ramp.lines().nth(1).unwrap();
        assert!(header.chars().any(|c|"▁▂▃▄▅▆▇█".contains(c)),"{header}");
        assert!(header.contains("Torrents")&&header.contains("RSS")&&header.contains("↓"),"{header}");
        app.store.summary.as_mut().unwrap().history=Some(vec![0;60]);let zero=crate::tui::golden::render(&app,120,30);
        assert!(!zero.lines().nth(1).unwrap().chars().any(|c|"▁▂▃▄▅▆▇█".contains(c)));
        for width in [40,80,120,160] {let _=crate::tui::golden::render(&app,width,30);}
        app.online=false;let offline=crate::tui::golden::render(&app,120,30);assert!(!offline.lines().nth(1).unwrap().chars().any(|c|"▁▂▃▄▅▆▇█".contains(c)));
        println!("received throttled ramp at 120×30:\n{ramp}");
    }