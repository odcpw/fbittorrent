use super::*;
use crate::engine::tests::{review_record, wire};

#[test]
fn missing_files_discards_live_estimates_in_shared_torrent_model() {
    let record = review_record(&"a".repeat(40), Path::new("/private-downloads"));
    let mut view = TorrentView::unrestored(&record, None);
    view.activity = Activity::Live;
    view.state = TorrentState::Downloading;
    view.checked = true;
    view.progress = Some(0.25);
    view.completed_bytes = Some(1024);
    view.dl_bps = Some(4096);
    view.ul_bps = Some(2048);
    view.eta_s = Some(3);
    view.files_missing();
    let row = crate::api::ops::torrents::torrent(view.clone()).unwrap();
    let wire = serde_json::to_value(row).unwrap();
    assert_eq!(wire["state"], "error");
    assert_eq!(wire["reason"], "missing_files");
    assert_eq!(wire["checked"], false);
    for key in ["progress", "completed_bytes", "completed_at", "dl_bps", "ul_bps", "eta_s"] {
        assert!(wire.get(key).is_none(), "missing payload cannot retain {key}: {wire}");
    }
    assert_eq!(view.activity, Activity::Error);
    // CLI and TUI now consume this same typed API row; no second serializer.
}

#[test]
fn api_projection_keeps_owned_origin_and_unknown_observations() {
    let mut record = review_record(&"a".repeat(40), Path::new("/private-downloads"));
    record.display_name = Some("My display name".into());
    record.tags = "fbt-origin:obsolete-tag".into();
    let mut view = TorrentView::unrestored(&record, None);
    view.origin = Some("rss:my-feed".into());
    view.upload_limit_bps = Some(0);
    let row = crate::api::ops::torrents::torrent(view.clone()).unwrap();
    assert_eq!(row.name, "My display name");
    assert_eq!(row.origin.as_deref(), Some("rss:my-feed"));
    assert_eq!(row.state, crate::api::TorrentState::Metadata);
    assert!(!row.upload_enabled);
    let wire = serde_json::to_value(&row).unwrap();
    for key in ["progress", "size", "completed_bytes", "uploaded_bytes", "dl_bps",
        "ul_bps", "eta_s", "peers", "seeds", "upload_limit_bps", "playable"] {
        assert!(wire.get(key).is_none(), "unknown {key} must stay absent: {wire}");
    }
    view.state = TorrentState::Downloading;
    view.progress = Some(0.25);
    view.completed_bytes = Some(1024);
    view.uploaded_bytes = Some(0);
    view.peers = Some(2);
    view.seeds = Some(0);
    view.dl_bps = Some(4096);
    view.ul_bps = Some(0);
    view.download_limit_bps = Some(8192);
    view.effective_download_limit_bps = Some(4096);
    view.upload_enabled = true;
    view.upload_policy = UploadPolicy::Custom;
    view.upload_limit_bps = Some(2048);
    let row = crate::api::ops::torrents::torrent(view).unwrap();
    assert_eq!(row.state, crate::api::TorrentState::Downloading);
    assert_eq!(row.progress, Some(0.25));
    assert_eq!((row.peers, row.seeds), (Some(2), Some(0)));
    assert_eq!((row.completed_bytes, row.uploaded_bytes), (Some(1024), Some(0)));
    assert_eq!((row.dl_bps, row.ul_bps), (Some(4096), Some(0)));
    assert_eq!((row.download_limit_bps, row.effective_download_limit_bps), (Some(8192), Some(4096)));
    assert!(row.upload_enabled);
    assert_eq!(row.upload_policy, crate::api::UploadPolicy::Custom);
    assert_eq!(row.upload_limit_bps, Some(2048));
}

#[test]
fn api_projection_refuses_invalid_owned_identifiers_and_timestamps() {
    let record = review_record(&"a".repeat(40), Path::new("/private-downloads"));
    let mut view = TorrentView::unrestored(&record, None);
    view.hash = "not-a-hash".into();
    assert_eq!(crate::api::ops::torrents::torrent(view).unwrap_err().code, crate::api::ErrorCode::Internal);
    for completed in [false, true] {
        let mut view = TorrentView::unrestored(&record, None);
        if completed {view.completed_at = Some(-1);} else {view.added_at = -1;}
        assert_eq!(crate::api::ops::torrents::torrent(view).unwrap_err().code, crate::api::ErrorCode::Internal);
    }
}

#[test]
fn state_mapping_table() {
    use TorrentState::*;
    // Index bits: durable pause, stop-on-complete, finished, handle paused.
    const PAUSED: [TorrentState; 16] = [
        Paused, Queued, Paused, Paused, Paused, Queued, Done, Done, Paused, Paused, Paused, Paused,
        Paused, Paused, Paused, Paused,
    ];
    const LIVE_IDLE: [TorrentState; 16] = [
        Stalled, Queued, Seeding, Seeding, Stalled, Queued, Done, Done, Paused, Paused, Paused,
        Paused, Paused, Paused, Paused, Paused,
    ];
    const LIVE_RECEIVING: [TorrentState; 16] = [
        Downloading,
        Queued,
        Seeding,
        Seeding,
        Downloading,
        Queued,
        Done,
        Done,
        Paused,
        Paused,
        Paused,
        Paused,
        Paused,
        Paused,
        Paused,
        Paused,
    ];
    let natives = [
        None,
        Some(TorrentStatsState::Initializing { paused: false }),
        Some(TorrentStatsState::Initializing { paused: true }),
        Some(TorrentStatsState::Paused),
        Some(TorrentStatsState::Live),
        Some(TorrentStatsState::Error),
    ];
    let mut rows = 0;
    for native in &natives {
        for bits in 0..16 {
            for receiving in [false, true] {
                for check_cancelled in [false, true] {
                    for parked in [false, true] {
                        let activity = match native {
                            None => Activity::Metadata,
                            Some(TorrentStatsState::Initializing { .. }) => Activity::Checking,
                            Some(TorrentStatsState::Paused) => Activity::Paused,
                            Some(TorrentStatsState::Live) => Activity::Live,
                            Some(TorrentStatsState::Error) => Activity::Error,
                        };
                        let facts = StateFacts {
                            activity,
                            record_paused: bits & 8 != 0,
                            stop_on_complete: bits & 4 != 0,
                            finished: bits & 2 != 0,
                            paused: bits & 1 != 0,
                            receiving,
                            check_cancelled,
                            parked,
                        };
                        let expected = if parked {
                            WaitingVpn
                        } else {
                            match native {
                                None => Metadata,
                                Some(TorrentStatsState::Error) => Error,
                                Some(TorrentStatsState::Initializing { .. })
                                    if !check_cancelled =>
                                {
                                    Checking
                                }
                                Some(
                                    TorrentStatsState::Initializing { .. }
                                    | TorrentStatsState::Paused,
                                ) => PAUSED[bits],
                                Some(TorrentStatsState::Live) => {
                                    if receiving {
                                        LIVE_RECEIVING[bits]
                                    } else {
                                        LIVE_IDLE[bits]
                                    }
                                }
                            }
                        };
                        let observed = derive_state(facts);
                        assert_eq!(
                            observed.0, expected,
                            "native={native:?}, row={facts:?}, observed={observed:?}"
                        );
                        let reason = match expected {
                            WaitingVpn => Some("vpn_down"),
                            Queued => Some("queue_full"),
                            Done => Some("complete_no_seed"),
                            Paused if facts.record_paused => Some("user"),
                            _ => None,
                        };
                        assert_eq!(
                            observed.1, reason,
                            "native={native:?}, row={facts:?}, observed={observed:?}"
                        );
                        rows += 1;
                    }
                }
            }
        }
    }
    println!(
        "state_mapping_table: {rows} rows; every rqbit state × handle pause × finished × completion policy × saved pause, plus receiving/cancelled/parked, maps once"
    );
    assert_eq!(rows, 768);
}

#[test]
fn normalized_unknowns_are_absent_from_the_wire() {
    let mut record = review_record(
        "0000000000000000000000000000000000000001",
        Path::new("/fixture/downloads"),
    );
    record.completion_on = -1;
    record.name = "é".repeat(1030);
    let view = TorrentView::unrestored(&record, None);
    let normalized = wire(&view);
    for key in [
        "reason",
        "error",
        "progress",
        "size",
        "completed_bytes",
        "uploaded_bytes",
        "dl_bps",
        "ul_bps",
        "eta_s",
        "peers",
        "seeds",
        "ratio",
        "content_path",
        "completed_at",
        "imdb_id",
        "queue_pos",
        "playable",
    ] {
        assert!(normalized.get(key).is_none(), "unknown {key}: {normalized}");
    }
    assert_eq!(view.name.chars().count(), 1024);
    assert_eq!(normalized["state"], "metadata");
    assert_eq!(normalized["intent"], "run");
    assert_eq!(normalized["upload_policy"], "off");
    assert_eq!(normalized["seed"], false);
    assert_eq!(normalized["origin"], "manual");
    assert_eq!(view.original_name(), record.name);
    let parked = TorrentView::parked(
        &record,
        Some((5, Some(PathBuf::from("/fixture/downloads/file")))),
    );
    let normalized = wire(&parked);
    assert_eq!(normalized["state"], "waiting_vpn");
    assert_eq!(normalized["reason"], "vpn_down");
    assert_eq!(normalized["size"], 5);
    assert!(normalized.get("progress").is_none());
    record.paused = true;
    let parked = TorrentView::parked(&record, None);
    assert_eq!(parked.state, TorrentState::WaitingVpn);
    assert_eq!(parked.intent, Intent::Pause);
}

#[test]
fn policy_and_restore_reason_are_normalized_without_fabricated_completion() {
    let mut record = review_record(
        "0000000000000000000000000000000000000001",
        Path::new("/fixture"),
    );
    for (limit, upload, policy, seed) in [
        (0, None, UploadPolicy::Off, false),
        (0, Some(true), UploadPolicy::Unlimited, true),
        (65536, Some(true), UploadPolicy::Custom, true),
        (65536, Some(false), UploadPolicy::Off, false),
        (0, None, UploadPolicy::Off, false),
    ] {
        record.upload_limit = limit;
        record.upload = upload;
        let view = TorrentView::unrestored(&record, None);
        assert_eq!((view.upload_policy, view.seed), (policy, seed));
    }
    let restore = RestoreTask {
        generation: 1,
        outcome: RestoreOutcome::Failed {
            reason: "restore_failed",
            message: "torrent metadata lookup timed out".into(),
        },
        last_error: Some("torrent metadata lookup timed out".into()),
        task: None,
        cancel: CancellationToken::new(),
    };
    let view = TorrentView::unrestored(&record, Some(&restore));
    assert_eq!(view.state, TorrentState::Error);
    assert_eq!(view.reason.as_deref(), Some("metadata_timeout"));
    assert!(view.error.is_some());
    assert!(view.completed_at.is_none());
}

#[test]
fn retained_missing_files_error_keeps_its_reason_after_stop_and_during_retry() {
    use crate::api::validation::Validate;
    let mut record = review_record(
        "0000000000000000000000000000000000000001",
        Path::new("/fixture"),
    );
    record.paused = true;
    let message = format!("{} restore failed: missing files", record.name);
    for (outcome, state) in [
        (
            RestoreOutcome::Failed { reason: "missing_files", message: message.clone() },
            TorrentState::Error,
        ),
        (RestoreOutcome::Stopped, TorrentState::Paused),
        (RestoreOutcome::Checking, TorrentState::Checking),
    ] {
        let restore = RestoreTask {
            generation: 1, outcome, last_error: Some(message.clone()),
            task: None, cancel: CancellationToken::new(),
        };
        let view = TorrentView::unrestored(&record, Some(&restore));
        assert_eq!(view.state, state);
        assert_eq!(view.reason.as_deref(), Some("missing_files"));
        assert_eq!(view.error.as_deref(), Some(message.as_str()));
        crate::api::ops::torrents::torrent(view).unwrap().validate().unwrap();
    }
    let recovered = TorrentView::unrestored(&record, None);
    assert!(recovered.error.is_none());
    assert_ne!(recovered.reason.as_deref(), Some("missing_files"));
    println!("missing-files error/reason survive stop and retry; paused/checking states and errors stay intact; clearing the error clears its reason");
}

#[test]
fn normalized_stats_have_real_counters_and_conditional_eta_and_ratio() {
    let record = review_record(
        "0000000000000000000000000000000000000001",
        Path::new("/fixture"),
    );
    let facts = StateFacts {
        activity: Activity::Live,
        paused: false,
        finished: false,
        stop_on_complete: true,
        record_paused: false,
        check_cancelled: false,
        receiving: true,
        parked: false,
    };
    let mut stats = librqbit::TorrentStats {
        state: TorrentStatsState::Live,
        file_progress: vec![],
        error: None,
        progress_bytes: 2 * 1024 * 1024,
        uploaded_bytes: 1024 * 1024,
        total_bytes: 10 * 1024 * 1024,
        finished: false,
        live: Some(Default::default()),
    };
    let live = stats.live.as_mut().unwrap();
    live.download_speed.mbps = 1.0;
    live.upload_speed.mbps = 0.5;
    live.snapshot.peer_stats.live = 3;
    let mut view = TorrentView::record(&record, facts, false);
    view.populate_stats(&stats, true);
    assert_eq!(view.state, TorrentState::Downloading);
    assert_eq!(view.progress, Some(0.2));
    assert_eq!(view.ratio, Some(0.5));
    assert_eq!(view.eta_s, Some(8));
    assert_eq!(view.completed_bytes, Some(2 * 1024 * 1024));
    assert_eq!(view.uploaded_bytes, Some(1024 * 1024));
    assert_eq!(view.dl_bps, Some(1024 * 1024));
    assert_eq!(view.ul_bps, Some(512 * 1024));
    assert_eq!(view.peers, Some(3));
    let mut paused = TorrentView::record(
        &record,
        StateFacts {
            activity: Activity::Paused,
            ..facts
        },
        false,
    );
    paused.populate_stats(&stats, true);
    assert!(
        paused.eta_s.is_none()
            && paused.dl_bps.is_none()
            && paused.ul_bps.is_none()
            && paused.peers.is_none()
    );
    stats.progress_bytes = 0;
    stats.uploaded_bytes = 0;
    stats.live.as_mut().unwrap().download_speed.mbps = 0.0;
    let mut stalled = TorrentView::record(
        &record,
        StateFacts {
            receiving: false,
            ..facts
        },
        false,
    );
    stalled.populate_stats(&stats, true);
    assert_eq!(stalled.state, TorrentState::Stalled);
    assert_eq!(stalled.dl_bps, Some(0));
    assert!(stalled.eta_s.is_none() && stalled.ratio.is_none());
    println!(
        "normalized stats: progress=0.2 completed=2MiB uploaded=1MiB dl=1MiB/s ul=0.5MiB/s eta=8s ratio=0.5 peers=3; paused/zero-rate ETA absent"
    );
}

#[test]
fn rechecking_set_forces_checking() {
    for paused in [false, true] {
        let record = crate::engine::tests::review_record("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", Path::new("/fixture"));
        let mut view = TorrentView::record(&record, StateFacts { activity: Activity::Checking,
            paused, finished: false, stop_on_complete: true, record_paused: paused,
            check_cancelled: true, receiving: false, parked: false }, false);
        view.rechecking();
        assert_eq!(view.state, TorrentState::Checking);
        assert!(!view.checked);
        assert!(view.completed_at.is_none());
    }
}
