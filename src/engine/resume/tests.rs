use super::*;
use crate::engine::tests::{review_directory, review_record};
fn local() -> (PathBuf, Record, Vec<u8>) {
    let root = review_directory("fast-resume");
    let downloads = root.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let name = "hello.txt";
    let mut metadata =
        b"d4:infod6:lengthi5e4:name9:hello.txt12:piece lengthi16384e6:pieces20:".to_vec();
    metadata.extend_from_slice(&[
        0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48, 0x2c,
        0xd9, 0xae, 0xa9, 0x43, 0x4d,
    ]);
    metadata.extend_from_slice(b"ee");
    let hash = librqbit::torrent_from_bytes(&metadata)
        .unwrap()
        .info_hash
        .as_string();
    std::fs::write(root.join(format!("{hash}.torrent")), &metadata).unwrap();
    std::fs::write(downloads.join(name), b"hello").unwrap();
    let mut record = review_record(&hash, &downloads);
    record.name = name.into();
    (root, record, metadata)
}
fn write_snapshot(store: &Store, record: &Record, metadata: &[u8]) {
    let bits = vec![0x80];
    let value = measurement(record, metadata, &bits).unwrap();
    let header = Header {
        hash: record.hash.clone(),
        folder: record.output_folder.clone(),
        selection: record.only_files.clone(),
        pieces: 1,
        files: stamps(record, metadata, &bits).unwrap(),
        size: value.size,
        have: value.have,
    };
    let h = serde_json::to_vec(&header).unwrap();
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&(h.len() as u32).to_be_bytes());
    bytes.extend(h);
    bytes.extend(bits);
    bytes.extend_from_slice(&crc(&bytes).to_be_bytes());
    atomic_write(&store.path(&record.hash).unwrap(), &bytes).unwrap();
    atomic_write(&store.directory.join("CLEAN"), store.boot_marker.as_ref().unwrap()).unwrap();
}
#[test]
fn clean_authority_is_single_use_and_stamps_selection_and_bits_are_checked() {
    let (root, record, metadata) = local();
    let store = Store::open(&root).unwrap();
    write_snapshot(&store, &record, &metadata);
    let store = Store::open(&root).unwrap();
    assert!(!store.directory.join("CLEAN").exists());
    let prepared = store.prepare(&root, &[record.clone()]).unwrap();
    assert_eq!(prepared.pending(&record.hash).unwrap().have, 5);
    assert_eq!(prepared.take(&record.hash).unwrap(), [0x80]);
    assert!(prepared.take(&record.hash).is_none());
    assert!(
        store
            .prepare(&root, &[record.clone()])
            .unwrap()
            .take(&record.hash)
            .is_none(),
        "clean authorization cannot survive an unclean restart"
    );
    // Record policy and bit geometry cannot silently reuse a prior selection.
    let mut changed = record.clone();
    changed.only_files = Some(vec![]);
    assert!(store.load(&root, &changed).is_err());
    std::fs::write(
        Path::new(&record.output_folder).join(&record.name),
        b"HELLO",
    )
    .unwrap();
    assert!(
        store.load(&root, &record).is_err(),
        "same-size payload write invalidates nanosecond mtime"
    );
    assert!(validate_bits(&[0x81], 1).is_err());
    assert!(validate_bits(&[], 1).is_err());
    println!("clean marker consumed; snapshot is single-use; exact selection, same-size modifications and malformed bitfields reject trust");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn corrupt_snapshot_falls_back_per_record_and_crash_ignores_even_valid_snapshots() {
    let (root, record, metadata) = local();
    let store = Store::open(&root).unwrap();
    write_snapshot(&store, &record, &metadata);
    std::fs::write(store.path(&record.hash).unwrap(), b"truncated").unwrap();
    let store = Store::open(&root).unwrap();
    assert!(store
        .prepare(&root, &[record.clone()])
        .unwrap()
        .take(&record.hash)
        .is_none());
    write_snapshot(&store, &record, &metadata);
    let store = Store::open(&root).unwrap();
    assert!(store
        .prepare(&root, &[record.clone()])
        .unwrap()
        .take(&record.hash)
        .is_some());
    let store = Store::open(&root).unwrap();
    assert!(store
        .prepare(&root, &[record.clone()])
        .unwrap()
        .take(&record.hash)
        .is_none());
    store.invalidate(&record.hash).unwrap();
    assert!(!store.path(&record.hash).unwrap().exists());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn periodic_does_not_authorize_trust_or_overwrite_the_drained_generation() {
    let (root, record, metadata) = local();
    let store = Store::open(&root).unwrap();
    write_snapshot(&store, &record, &metadata);
    // A live snapshot can exist, but no CLEAN marker means no trust.
    std::fs::remove_file(store.directory.join("CLEAN")).unwrap();
    let store = Store::open(&root).unwrap();
    assert!(store
        .prepare(&root, &[record.clone()])
        .unwrap()
        .take(&record.hash)
        .is_none());
    store.save(&[], &HashMap::new(), true).unwrap();
    let marker = std::fs::read(store.directory.join("CLEAN")).unwrap();
    store.save(&[record], &HashMap::new(), false).unwrap();
    assert_eq!(
        std::fs::read(store.directory.join("CLEAN")).unwrap(),
        marker
    );
    assert_eq!(
        std::fs::metadata(store.directory.join("CLEAN"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(&store.directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn parked_clean_exit_preserves_only_previously_earned_authority() {
    let (root, record, metadata) = local();
    let store = Store::open(&root).unwrap();
    store.preserve_parked_clean().unwrap();
    assert!(!store.directory.join("CLEAN").exists());
    write_snapshot(&store, &record, &metadata);
    let store = Store::open(&root).unwrap();
    assert!(!store.directory.join("CLEAN").exists());
    store.preserve_parked_clean().unwrap();
    assert!(store.directory.join("CLEAN").exists());
    store.abandon_clean();
    store.preserve_parked_clean().unwrap();
    assert!(!store.directory.join("CLEAN").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_checksum_rejects_payload_or_header_corruption() {
    let (root, record, metadata) = local();
    let store = Store::open(&root).unwrap();
    write_snapshot(&store, &record, &metadata);
    let path = store.path(&record.hash).unwrap();
    let original = std::fs::read(&path).unwrap();
    for index in [16, original.len() - 5] {
        let mut changed = original.clone();
        changed[index] ^= 1;
        std::fs::write(&path, changed).unwrap();
        assert!(format!("{:#}", store.load(&root, &record).err().unwrap())
            .contains("checksum mismatch"));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_different_boot_never_trusts_even_valid_unchanged_payload() {
    let (root, record, metadata) = local();
    let store = Store::open(&root).unwrap();
    write_snapshot(&store, &record, &metadata);
    let marker = store.directory.join("CLEAN");
    std::fs::write(&marker, b"boot-v2 00000000-0000-0000-0000-000000000000\n").unwrap();
    let store = Store::open(&root).unwrap();
    assert!(!marker.exists(), "untrusted token is still consumed");
    assert!(store.prepare(&root, &[record.clone()]).unwrap().take(&record.hash).is_none());
    // Current boot still supports the no-read process restart optimization.
    write_snapshot(&store, &record, &metadata);
    let store = Store::open(&root).unwrap();
    assert!(store.prepare(&root, &[record.clone()]).unwrap().take(&record.hash).is_some());
    std::fs::remove_dir_all(root).unwrap();
}
