use super::*;
#[path="fixture.rs"]mod fixture;
fn totals()->crate::api::SampledStats {
    crate::api::SampledStats {history:crate::api::RateHistory {download:vec![],upload:vec![],interval_s:1},session:Some(crate::api::SessionTotals {downloaded:8*1024*1024,uploaded:0,started_at:1}),all_time:Some(crate::api::AllTimeTotals {downloaded:16*1024*1024,uploaded:1024,completed:2})}
}
#[test]fn statistics_footer_follows_confirmed_totals_and_stays_inside_log() {
    let mut app=fixture::app("torrents");app.log_open=true;app.online=true;app.store.stale=false;
    let unknown=super::golden::render(&app,120,30);assert!(!unknown.contains("all time"));
    app.store.stats=Some(totals());
    for (w,h) in [(80,24),(120,30),(160,45)] {
        let frame=super::golden::render(&app,w,h);
        assert!(frame.contains("session ↓ 8.0 MiB ↑ 0 B"),"{frame}");
        assert!(frame.contains("all time ↓ 16.0 MiB ↑ 1.0 KiB · 2 completed"),"{frame}");
        assert!(frame.contains("L/Esc Back") && frame.contains("(?) Help"));
        println!("confirmed statistics at {w}×{h}:\n{frame}");
    }
    app.store.stats.as_mut().unwrap().session.as_mut().unwrap().downloaded=0;
    app.store.stats.as_mut().unwrap().all_time.as_mut().unwrap().completed=3;
    let updated=super::golden::render(&app,120,30);assert!(updated.contains("session ↓ 0 B"));assert!(updated.contains("3 completed"));
    app.store.stale=true;assert!(!super::golden::render(&app,120,30).contains("all time"));
}
