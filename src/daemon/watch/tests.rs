use super::*;
fn sample(prefix: u64) -> Sample {
    Sample {
        hash: "a".repeat(40),
        name: "Fixture".into(),
        file: 1,
        path: PathBuf::from("/nonexistent/fixture.mkv"),
        size: 64 * MIB,
        prefix,
        verified_bytes: prefix,
        last: true,
        downloading: true,
        active: true,
        dl_bps: Some(MIB),
        download_limit: None,
        verified: None,
    }
}
#[test]
fn only_active_video_samples_keep_the_background_clock_running() {
    let path = std::env::temp_dir().join(format!("fbt-watch-clock-{}.json", std::process::id()));
    let mut watch = Watch::open(path);
    assert!(!watch.needs_tick());
    let mut paused = sample(0);
    paused.active = false;
    paused.downloading = false;
    paused.dl_bps = None;
    assert!(watch.sample(vec![paused]).is_empty());
    assert!(!watch.needs_tick(), "paused video remains readable without periodic rescans");
    assert_eq!(watch.file_ready(&"a".repeat(40), 1), Some(false));
    let mut stalled = sample(0);
    stalled.downloading = false;
    stalled.dl_bps = Some(0);
    watch.sample(vec![stalled]);
    assert!(watch.needs_tick(), "an unpaused stalled video can receive pieces later");
    watch.sample(Vec::new());
    assert!(!watch.needs_tick());
    assert_eq!(watch.file_ready(&"a".repeat(40), 1), Some(false));
}
#[test]
fn prefix_from_bitfield() {
    let have = [true, false, true, true, false];
    assert_eq!(prefix(0, 8, 4, |i| have[i]), 4);
    assert_eq!(
        prefix(10, 7, 4, |i| have[i]),
        6,
        "own first piece starts mid-piece; earlier missing piece irrelevant"
    );
    assert_eq!(prefix(10, 6, 4, |i| have[i]), 6);
    assert_eq!(prefix(8, 8, 4, |_| false), 0);
    assert_eq!(prefix(0, 0, 4, |_| true), 0);
    println!("verified contiguous prefix clipped at each file offset and end");
}
#[test]
fn threshold_rules() {
    for (prefix, size, last, want) in [
        (32 * MIB - 1, 64 * MIB, true, false),
        (32 * MIB, 64 * MIB, false, false),
        (32 * MIB, 64 * MIB, true, true),
        (32 * MIB, 2048 * MIB, true, false),
        (62 * MIB, 2048 * MIB, true, true),
    ] {
        assert_eq!(ready(prefix, size, last), want);
        println!("prefix={prefix} size={size} last={last} ready={want}");
    }
}
#[test]
fn largest_video_file() {
    let files = [
        ("small.mkv", 4),
        ("large.iso", 50),
        ("large.MP4", 20),
        ("same.webm", 20),
        ("empty.ts", 0),
    ];
    assert_eq!(
        largest_video(
            files
                .iter()
                .enumerate()
                .map(|(i, (p, n))| (i, Path::new(p), *n))
        ),
        Some(2)
    );
    assert_eq!(
        largest_video([(0, Path::new("linux.iso"), 500)].into_iter()),
        None
    );
    println!("video extensions, largest selected size and first-index tie break");
}
#[test]
fn stall_free_rule() {
    for (prefix, rate, duration, want) in [
        (32, 1, 64., true),
        (32, 1, 16., false),
        (48, 1, 64., true),
        (16, 2, 16., false),
        (48, 2, 16., true),
    ] {
        let s = Sample {
            size: 64,
            prefix,
            dl_bps: Some(rate),
            ..sample(prefix)
        };
        let got = value(&s, Some(duration), false);
        assert_eq!(got.stall_free, Some(want));
        println!("prefix={prefix} size=64 duration={duration} rate={rate} stall_free={want} buffered={:?} finishes={:?}",got.buffered_s,got.finishes_in_s);
    }
    let mut s = sample(32 * MIB);
    s.downloading = false;
    assert_eq!(value(&s, Some(120.), false).stall_free, None);
}
#[test]
fn no_duration_fallback() {
    let s = sample(32 * MIB);
    let p = value(&s, None, false);
    assert!(p.ready);
    assert_eq!(p.buffered_s, None);
    assert_eq!(p.stall_free, None);
    assert_eq!(p.finishes_in_s, Some(32));
    assert!(estimate_rate(64 * MIB, 10000));
    assert!(!estimate_rate(64 * MIB, 100));
    assert_eq!(duration::parse(b"random bytes, not a container"), None);
    println!("no duration remains absent; 2h rate rule is only an estimate");
}
#[test]
fn throttled_readiness_survives_bursts_and_zero_rate_gaps() {
    let mut estimate = estimate::Estimate::default();
    let mut s = sample(32 * MIB);
    s.download_limit = Some(16 * 1024);
    let now = Instant::now();
    let mut states = Vec::new();
    for second in 0..24 {
        // Token-bucket bursts and intervening zero readings must never make
        // a 16 KiB/s link look capable of feeding a 1 MiB/s video.
        s.dl_bps = Some(if second % 3 == 0 { 4 * MIB } else { 0 });
        s.downloading = s.dl_bps != Some(0);
        let mut p = value(&s, Some(64.), false);
        estimate.apply(&s, Some(64.), now + Duration::from_secs(second), &mut p);
        assert_eq!(p.stall_free, Some(false), "second={second}");
        states.push(p.stall_free);
    }
    s.download_limit = None;
    s.dl_bps = Some(4 * MIB);
    s.downloading = true;
    for second in 24..28 {
        let mut p = value(&s, Some(64.), false);
        estimate.apply(&s, Some(64.), now + Duration::from_secs(second), &mut p);
        states.push(p.stall_free);
        if second >= 25 { assert_eq!(p.stall_free, Some(true)); }
    }
    assert_eq!(states.windows(2).filter(|pair| pair[0] != pair[1]).count(), 1);
    for second in 28..48 {
        s.dl_bps = Some(if second % 2 == 0 { MIB * 98 / 100 } else { MIB * 102 / 100 });
        let mut p = value(&s, Some(64.), false);
        estimate.apply(&s, Some(64.), now + Duration::from_secs(second), &mut p);
        assert_eq!(p.stall_free, Some(true), "threshold noise must stay inside hysteresis");
    }
    s.download_limit = Some(16 * 1024);
    for second in 48..51 {
        let mut p = value(&s, Some(64.), false);
        estimate.apply(&s, Some(64.), now + Duration::from_secs(second), &mut p);
        if second >= 49 { assert_eq!(p.stall_free, Some(false)); }
    }
}
#[test]
fn decorating_stalled_rows_keeps_the_known_stall_warning() {
    let mut watch = Watch::open(PathBuf::from("/nonexistent/watch-stalled-test.json"));
    let s = sample(0);
    watch.sample(vec![s]);
    watch.rows.get_mut(&"a".repeat(40)).unwrap().value.stall_free = Some(false);
    let mut row: api::Torrent = serde_json::from_value(json!({"hash":"a".repeat(40),"name":"Fixture",
        "state":"stalled","intent":"run","staged":false,"checked":true,"seed":false,
        "upload_policy":"off","save_path":"/tmp","added_at":1,"force":false,"dl_bps":0})).unwrap();
    watch.decorate(std::slice::from_mut(&mut row));
    assert_eq!(row.playable.as_ref().unwrap().stall_free, Some(false));
    row.state = api::TorrentState::Paused;
    watch.decorate(std::slice::from_mut(&mut row));
    assert_eq!(row.playable.as_ref().unwrap().stall_free, None);
}
#[test]
fn sustainable_bursty_link_does_not_flip_on_each_zero_rate_sample() {
    let mut estimate = estimate::Estimate::default();
    let mut s = sample(32 * MIB);
    s.download_limit = Some(2 * MIB);
    let now = Instant::now();
    s.dl_bps = Some(MIB * 125 / 100);
    let mut p = value(&s, Some(64.), false);
    estimate.apply(&s, Some(64.), now, &mut p);
    assert_eq!(p.stall_free, Some(true));
    for second in 1..20 {
        // Sustained 1.2 MiB/s beats the video's 1 MiB/s bitrate. Packet
        // delivery is bursty but the link and its cap have not changed.
        s.dl_bps = Some(if second % 2 == 0 { MIB * 24 / 10 } else { 0 });
        s.downloading = s.dl_bps != Some(0);
        estimate.apply(&s, Some(64.), now + Duration::from_secs(second), &mut p);
        assert_eq!(p.stall_free, Some(true), "second={second}");
    }
}
#[test]
fn verified_receipts_refresh_the_rate_after_a_cap_change() {
    let mut estimate = estimate::Estimate::default();
    let now = Instant::now();
    let mut s = sample(32 * MIB);
    s.download_limit = Some(16 * 1024);
    s.dl_bps = Some(16 * 1024);
    let mut p = value(&s, Some(64.), false);
    estimate.apply(&s, Some(64.), now, &mut p);
    assert_eq!(p.stall_free, Some(false));
    s.download_limit = Some(4 * MIB);
    estimate.apply(&s, Some(64.), now + Duration::from_secs(1), &mut p);
    assert_eq!(p.stall_free, Some(false), "lifting a cap alone is not evidence of a faster link");
    // All verified pieces measure incoming work, even when a preceding piece
    // delays the contiguous prefix and the speed counter has not caught up.
    s.verified_bytes += 2 * MIB;
    s.prefix += MIB;
    estimate.apply(&s, Some(64.), now + Duration::from_secs(2), &mut p);
    assert_eq!(p.stall_free, Some(true));
    assert!(p.finishes_in_s.unwrap() < p.buffered_s.unwrap());
}
#[test]
fn hysteresis_never_promises_stall_free_when_finish_exceeds_buffer() {
    let mut estimate = estimate::Estimate::default();
    let mut s = sample(32 * MIB);
    let now = Instant::now();
    let mut p = value(&s, Some(64.), false);
    estimate.apply(&s, Some(64.), now, &mut p);
    assert_eq!(p.stall_free, Some(true));
    s.dl_bps = Some(MIB * 95 / 100);
    estimate.apply(&s, Some(64.), now + Duration::from_secs(1), &mut p);
    assert!(p.finishes_in_s.unwrap() > p.buffered_s.unwrap());
    assert_eq!(p.stall_free, Some(false));
}
#[test]
fn container_headers_bounded_and_validated() {
    let mut mkv = vec![
        0x1a, 0x45, 0xdf, 0xa3, 0x80, 0x18, 0x53, 0x80, 0x67, 0xff, 0x15, 0x49, 0xa9, 0x66, 0x8b,
        0x44, 0x89, 0x88,
    ];
    mkv.extend_from_slice(&120000f64.to_be_bytes());
    assert_eq!(duration::parse(&mkv), Some(120.));
    let mut payload = mkv.clone();
    payload.extend_from_slice(&[0xa5; 4096]);
    assert_eq!(duration::parse(&payload), Some(120.));
    let mut mp4 = vec![
        0, 0, 0, 36, b'm', b'o', b'o', b'v', 0, 0, 0, 28, b'm', b'v', b'h', b'd',
    ];
    mp4.extend_from_slice(&[0; 12]);
    mp4.extend_from_slice(&1000u32.to_be_bytes());
    mp4.extend_from_slice(&120000u32.to_be_bytes());
    assert_eq!(duration::parse(&mp4), Some(120.));
    for i in 0..mp4.len() {
        let _ = duration::parse(&mp4[..i]);
    }
    for i in 0..mkv.len() {
        let _ = duration::parse(&mkv[..i]);
    }
    println!("Matroska Duration/TimecodeScale and ISO mvhd durations; all truncations safe");
}
#[test]
fn once_pause_ack_restart_and_lost_prefix() {
    let dir = std::env::temp_dir().join(format!(
        "fbt-watch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("watch.json");
    let mut w = Watch::open(path.clone());
    assert_eq!(w.sample(vec![sample(32 * MIB)]).len(), 1);
    assert!(w.sample(vec![sample(40 * MIB)]).is_empty());
    w.acknowledge(Some(&"a".repeat(40)));
    assert!(w.rows[&"a".repeat(40)].value.acked);
    let mut paused = sample(40 * MIB);
    paused.active = false;
    paused.downloading = false;
    paused.dl_bps = None;
    assert!(w.sample(vec![paused]).is_empty());
    assert!(w.rows[&"a".repeat(40)].value.ready);
    w.flush(Instant::now() + Duration::from_secs(1));
    assert!(!w.persistence.pending(), "restart observes a fully durable checkpoint");
    let mut restarted = Watch::open(path.clone());
    assert!(restarted.sample(vec![sample(40 * MIB)]).is_empty());
    assert!(restarted.sample(vec![sample(0)]).is_empty());
    assert_eq!(restarted.sample(vec![sample(32 * MIB)]).len(), 1);
    assert!(restarted.sample(vec![sample(0)]).is_empty());
    assert!(
        restarted.sample(vec![sample(32 * MIB)]).is_empty(),
        "at most once per daemon life"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    restarted.flush(Instant::now() + Duration::from_secs(1));
    drop((w, restarted));
    std::fs::remove_dir_all(dir).unwrap();
    println!("pause retains readiness; acknowledgement and restart suppression durable; a lost prefix re-arms next daemon");
}
