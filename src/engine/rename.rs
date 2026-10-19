//! A cosmetic ledger override. Never mutate metadata or payload paths.
use super::*;
pub(crate) fn valid_name(name: &str) -> bool {
    name.chars().count() <= 512
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
}
pub(super) fn apply(ledger: &mut Ledger, hash: &str, name: &str) -> Result<()> {
    anyhow::ensure!(
        valid_name(name),
        "display name must be at most 512 characters without controls"
    );
    let record = ledger
        .records
        .iter_mut()
        .find(|r| r.hash == hash)
        .context("torrent record not found")?;
    let name = name.trim();
    record.display_name = (!name.is_empty()).then(|| name.to_owned());
    Ok(())
}
impl EngineDispatcher {
    pub(super) async fn rename(&self, cx: &Cx, hash: &str, name: &str) -> Result<()> {
        let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        apply(&mut ledger, hash, name)?;
        self.inner.save(cx, &ledger).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_validation() {
        assert!(valid_name(""));
        assert!(valid_name("  Café ✨  "));
        assert!(valid_name(&"é".repeat(512)));
        for bad in [
            "new\nname",
            "new\tname",
            "new\rname",
            "new\0name",
            "line\u{2028}name",
            &"é".repeat(513),
        ] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }
    #[test]
    fn display_name_survives_restore_and_metadata() {
        let root =
            std::env::temp_dir().join(format!("fbt-rename-{}-{}", std::process::id(), now()));
        let options = ops::AddRequest {
            sources: vec![
                "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&dn=Release.Name"
                    .into(),
            ],
            torrent: None,
            save_path: None,
            upload_limit: 0,
            ratio_limit: -1.0,
            stopped: true,
            category: String::new(),
            tags: String::new(),
            origin: None,
        };
        let record = Record::deferred_magnet(&options, &root).unwrap();
        let hash = record.hash.clone();
        let mut ledger = Ledger::default();
        ledger.records.push(record);
        apply(&mut ledger, &hash, "  My film  ").unwrap();
        save_parked_ledger(&root, &ledger).unwrap();
        let mut restored = read_ledger(&root.join("torrents.json")).unwrap();
        // The metadata-resolution path replaces only this original-name field.
        restored.records[0].name = "Verified metadata title".into();
        save_parked_ledger(&root, &restored).unwrap();
        let mut restored = read_ledger(&root.join("torrents.json")).unwrap();
        assert_eq!(restored.records[0].display_name.as_deref(), Some("My film"));
        assert_eq!(restored.records[0].name, "Verified metadata title");
        assert_eq!(restored.records[0].output_folder, root.to_string_lossy());
        apply(&mut restored, &hash, "").unwrap();
        assert!(restored.records[0].display_name.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
