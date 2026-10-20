use super::*;

#[test]
fn verify_io_failure_never_counts_skipped_bytes() {
    let (dir, info, files, storage) = fixture();
    File::options()
        .write(true)
        .open(dir.path().join("2.bin"))
        .unwrap()
        .set_len(0)
        .unwrap();
    let mut metadata = info.info().clone();
    let mut digests = metadata.pieces.as_ref().to_vec();
    // A malicious/wrong digest must not match a skipped read's empty hasher.
    digests[40..60].copy_from_slice(&Sha1::new().finish());
    metadata.pieces = digests.into();
    let metadata = metadata.validate().unwrap();
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let ops = FileOps::new(&metadata, &storage, &files);
    let check = |range| {
        ops.initial_check_range(&cx, &AtomicU64::new(0), &storage.pause, range)
            .unwrap()
    };
    let serial = check(0..metadata.lengths().total_pieces());
    assert_eq!(
        serial.iter().by_vals().take(5).collect::<Vec<_>>(),
        [true, false, false, false, true]
    );
    for split in 1..metadata.lengths().total_pieces() {
        assert_eq!(
            check(0..split) | check(split..metadata.lengths().total_pieces()),
            serial
        );
    }
}
