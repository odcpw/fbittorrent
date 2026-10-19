//! Live session policy; persistence is owned by the daemon settings transaction.
use super::*;

pub(super) fn apply_limits(session: &Session, cfg: &Config) {
    let (down, up) = if cfg.alt_limits_on {
        (cfg.alt_download_limit_bps, cfg.alt_upload_limit_bps)
    } else {
        (cfg.download_limit_bps, cfg.upload_limit_bps)
    };
    session
        .ratelimits
        .set_download_bps(down.and_then(std::num::NonZeroU32::new));
    session
        .ratelimits
        .set_upload_bps(up.and_then(std::num::NonZeroU32::new));
}
impl EngineDispatcher {
    pub(crate) fn apply_settings(&self, cfg: &Config) -> Result<()> {
        cfg.validate()?;
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        self.runtime.block_on(async {
            let cx = self.runtime.cx();
            let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
            self.inner
                .max_active_downloads
                .store(cfg.max_active_downloads, Ordering::Release);
            apply_limits(&self.inner.session, cfg);
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_settings_replace_session_limiters_without_restarting() {
        let root = std::env::temp_dir().join(format!("fbt-settings-limits-{}", std::process::id()));
        let app =
            super::super::tests::test_app(&root, &root.join("downloads"), Ledger::default(), None);
        let dispatcher = app.dispatcher();
        let mut cfg = Config {
            upload_limit_bps: Some(65536),
            download_limit_bps: Some(131072),
            alt_upload_limit_bps: Some(16384),
            alt_download_limit_bps: Some(32768),
            ..Config::default()
        };
        dispatcher.apply_settings(&cfg).unwrap();
        let limits = &dispatcher.inner.session.ratelimits;
        assert_eq!(limits.get_upload_bps().unwrap().get(), 65536);
        assert_eq!(limits.get_download_bps().unwrap().get(), 131072);
        cfg.alt_limits_on = true;
        dispatcher.apply_settings(&cfg).unwrap();
        assert_eq!(limits.get_upload_bps().unwrap().get(), 16384);
        assert_eq!(limits.get_download_bps().unwrap().get(), 32768);
        cfg.alt_upload_limit_bps = None;
        cfg.alt_download_limit_bps = None;
        dispatcher.apply_settings(&cfg).unwrap();
        assert!(limits.get_upload_bps().is_none());
        assert!(limits.get_download_bps().is_none());
        println!("same live session: primary limits -> alternative limits -> unlimited; torrent policy independent");
        drop(dispatcher);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
