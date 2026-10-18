use super::*;
use crate::engine::tests::review_record;

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
fn normalized_unknowns_are_absent_and_legacy_sentinels_stay_at_the_boundary() {
    let mut record = review_record(
        "0000000000000000000000000000000000000001",
        Path::new("/fixture/downloads"),
    );
    record.completion_on = -1;
    record.name = "é".repeat(1030);
    let view = TorrentView::unrestored(&record, None);
    let normalized = serde_json::to_value(&view).unwrap();
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
        "origin",
        "imdb_id",
        "queue_pos",
        "playable",
    ] {
        assert!(normalized.get(key).is_none(), "unknown {key}: {normalized}");
    }
    assert_eq!(view.name.chars().count(), 1024);
    assert_eq!(normalized["state"], "metadata");
    assert_eq!(normalized["intent"], "run");
    assert_eq!(normalized["upload_policy"], "default_cap");
    assert_eq!(normalized["seed"], false);
    let legacy: Torrent = serde_json::from_str(&view.to_legacy_json().unwrap()).unwrap();
    assert_eq!(legacy.completion_on, -1);
    assert_eq!(legacy.eta, 8640000);
    assert_eq!(legacy.name, record.name);
    assert_eq!(legacy.state, "metaDL");
    let parked = TorrentView::parked(
        &record,
        Some((5, Some(PathBuf::from("/fixture/downloads/file")))),
    );
    let normalized = serde_json::to_value(&parked).unwrap();
    assert_eq!(normalized["state"], "waiting_vpn");
    assert_eq!(normalized["reason"], "vpn_down");
    assert_eq!(normalized["size"], 5);
    assert!(normalized.get("progress").is_none());
    record.paused = true;
    let parked = TorrentView::parked(&record, None);
    let legacy: Torrent = serde_json::from_str(&parked.to_legacy_json().unwrap()).unwrap();
    assert_eq!(legacy.state, "stoppedDL");
    assert_eq!(parked.state, TorrentState::WaitingVpn);
}

#[test]
fn policy_and_restore_reason_are_normalized_without_fabricated_completion() {
    let mut record = review_record(
        "0000000000000000000000000000000000000001",
        Path::new("/fixture"),
    );
    for (limit, ratio, policy, seed) in [
        (NO_UPLOAD_LIMIT, 0.0, UploadPolicy::DefaultCap, false),
        (0, -2.0, UploadPolicy::Unlimited, true),
        (65536, -2.0, UploadPolicy::Custom, true),
        (0, 0.0, UploadPolicy::Custom, false),
    ] {
        record.upload_limit = limit;
        record.ratio_limit = ratio;
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
    let mut view = TorrentView::record(&record, facts, Source::Handle);
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
        Source::Handle,
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
        Source::Handle,
    );
    stalled.populate_stats(&stats, true);
    assert_eq!(stalled.state, TorrentState::Stalled);
    assert_eq!(stalled.dl_bps, Some(0));
    assert!(stalled.eta_s.is_none() && stalled.ratio.is_none());
    println!(
        "normalized stats: progress=0.2 completed=2MiB uploaded=1MiB dl=1MiB/s ul=0.5MiB/s eta=8s ratio=0.5 peers=3; paused/zero-rate ETA absent"
    );
}
