use super::*;
#[test]
fn missing_data_resets_completion() {
    let mut record = crate::engine::tests::review_record(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        Path::new("/fixture"),
    );
    record.completion_on = 123;
    apply_measurement(&mut record, 245760, 262144, 123);
    assert_eq!(record.completion_on, 0);
    let view = view::TorrentView::parked(&record, Some((262144, None)));
    assert!(view.completed_at.is_none());
    assert!(!record.dormant_on_restore());
    println!("missing piece: completion_on=0, completed_at absent, no dormant restore");
}

#[test]
fn unchanged_verified_data_keeps_its_original_completion_time() {
    let mut record = crate::engine::tests::review_record(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        Path::new("/fixture"),
    );
    apply_measurement(&mut record, 262144, 262144, 123);
    assert_eq!(record.completion_on, 123);
}
