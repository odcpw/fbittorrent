use super::*;
struct Tree(PathBuf);
impl Tree {
    fn new() -> Self {
        let path = crate::engine::tests::review_directory("move-fs");
        std::fs::create_dir_all(path.join("source/sub")).unwrap();
        std::fs::create_dir_all(path.join("target")).unwrap();
        Self(path)
    }
    fn source(&self) -> PathBuf {
        self.0.join("source")
    }
    fn target(&self) -> PathBuf {
        self.0.join("target")
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn strategy_by_device() {
    for (source, target, expected) in [(4, 4, Strategy::Rename), (4, 9, Strategy::Copy)] {
        assert_eq!(strategy(source, target), expected);
        println!(
            "MOVE-STRATEGY source_device={source} target_device={target} observed={expected:?}"
        );
    }
}
#[test]
fn anchored_rename_preserves_inode_and_rolls_back_without_commit() {
    let tree = Tree::new();
    let relative = PathBuf::from("sub/payload.bin");
    let path = tree.source().join(&relative);
    std::fs::write(&path, b"verified payload").unwrap();
    let inode = path.metadata().unwrap().ino();
    {
        let _transaction =
            Prepared::new(&tree.source(), &tree.target(), &[relative.clone()]).unwrap();
        assert!(!path.exists());
        assert_eq!(
            tree.target().join(&relative).metadata().unwrap().ino(),
            inode
        );
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"verified payload");
    assert!(!tree.target().join(&relative).exists());
    Prepared::new(&tree.source(), &tree.target(), &[relative.clone()])
        .unwrap()
        .commit()
        .unwrap();
    assert!(!path.exists());
    assert_eq!(
        tree.target().join(relative).metadata().unwrap().ino(),
        inode
    );
}
#[test]
fn copy_flushes_then_commits_without_changing_file_times() {
    let tree = Tree::new();
    let relative = PathBuf::from("sub/payload.bin");
    let path = tree.source().join(&relative);
    std::fs::write(&path, b"verified payload").unwrap();
    let before = path.metadata().unwrap();
    let transaction = Prepared::with_io(
        &tree.source(),
        &tree.target(),
        &[relative.clone()],
        |_, _| Strategy::Copy,
        |source, target| copy(&mut &*source, target),
    )
    .unwrap();
    assert!(path.exists());
    let target = tree.target().join(&relative);
    let after = target.metadata().unwrap();
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_ne!(after.ino(), before.ino());
    assert_eq!(std::fs::read(&target).unwrap(), b"verified payload");
    transaction.commit().unwrap();
    assert!(!path.exists());
}
#[test]
fn partial_copy_failure_leaves_source_and_removes_own_staging() {
    struct Failing<'a> {
        file: &'a mut File,
        written: usize,
    }
    impl Write for Failing<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.written >= 65536 {
                return Err(std::io::Error::other(
                    "fixture: copy failed after one buffer",
                ));
            }
            let size = self.file.write(bytes)?;
            self.written += size;
            Ok(size)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }
    let tree = Tree::new();
    let relative = PathBuf::from("payload.bin");
    let path = tree.source().join(&relative);
    let bytes = vec![0x3b; 200000];
    std::fs::write(&path, &bytes).unwrap();
    let result = Prepared::with_io(
        &tree.source(),
        &tree.target(),
        &[relative],
        |_, _| Strategy::Copy,
        |source, target| {
            copy(
                &mut &*source,
                &mut Failing {
                    file: target,
                    written: 0,
                },
            )
        },
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(tree.target()).unwrap().count(), 0);
    println!(
        "MOVE-COPY written_before_failure=65536 source_bytes_unchanged=true staging_removed=true"
    );
}
#[test]
fn symlinked_descendants_and_destination_collisions_never_replace_data() {
    use std::os::unix::fs::symlink;
    let tree = Tree::new();
    std::fs::write(tree.source().join("payload.bin"), b"source").unwrap();
    std::fs::write(tree.target().join("payload.bin"), b"existing target").unwrap();
    assert!(
        Prepared::new(
            &tree.source(),
            &tree.target(),
            &[PathBuf::from("payload.bin")]
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(tree.target().join("payload.bin")).unwrap(),
        b"existing target"
    );
    symlink(tree.target(), tree.source().join("link")).unwrap();
    assert!(
        Prepared::new(
            &tree.source(),
            &tree.target(),
            &[PathBuf::from("link/payload.bin")]
        )
        .is_err()
    );
    symlink(
        tree.source().join("payload.bin"),
        tree.source().join("leaf-link"),
    )
    .unwrap();
    assert!(
        Prepared::new(
            &tree.source(),
            &tree.target(),
            &[PathBuf::from("leaf-link")]
        )
        .is_err()
    );
    assert!(
        Prepared::new(
            &tree.source(),
            &tree.target(),
            &[PathBuf::from("../payload.bin")]
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(tree.source().join("payload.bin")).unwrap(),
        b"source"
    );
}
