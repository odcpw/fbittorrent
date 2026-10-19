#[test]
fn resolved_admission_saves_missing_metadata_without_replacing_existing_cache() {
    let (app,directory,records)=dormant_fixture("resolved-metadata-cache",123,false,false);
    let dispatcher=app.dispatcher();let record=&records[0];
    let path=directory.join(format!("{}.torrent",record.hash));let bytes=std::fs::read(&path).unwrap();
    dispatcher.runtime.block_on(async {
        let cx=dispatcher.runtime.cx();
        let metadata=dispatcher.inner.resolve_record(cx,record).await.unwrap();
        std::fs::remove_file(&path).unwrap();
        let handle=dispatcher.inner.admit_record(cx,record,true,metadata).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(),bytes,"resolved metainfo is durable before admission succeeds");
        handle.wait_until_initialized(cx).await.unwrap();
        assert_eq!(handle.info_hash().as_string(),record.hash);
        use std::os::unix::fs::MetadataExt;
        let before=std::fs::metadata(&path).unwrap();
        let metadata=dispatcher.inner.resolve_record(cx,record).await.unwrap();
        dispatcher.inner.admit_record(cx,record,true,metadata).await.unwrap();
        let after=std::fs::metadata(&path).unwrap();
        assert_eq!((after.ino(),after.modified().unwrap()),(before.ino(),before.modified().unwrap()),"existing cache fingerprints remain stable");
    });
}
