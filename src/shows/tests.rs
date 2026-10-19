    use super::*;

    #[test]
    fn shows_save_is_private_for_new_and_existing_stores() {
        let dir = std::env::temp_dir().join(format!("fbt-shows-private-{}", std::process::id()));
        let path = dir.join("shows.json");
        let mut shows = Shows::default();
        shows.follow(&Title { id: "tt1234001".into(), name: "Private fixture".into(), ..Default::default() });
        for replacement in [false, true] {
            if replacement {
                for file in [&path, &path.with_extension("json.lock")] {
                    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o644)).unwrap();
                }
            }
            shows.save_to(&path).unwrap();
            for file in [&path, &path.with_extension("json.lock")] {
                let mode = std::fs::metadata(file).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "{}", file.display());
                println!("{}: {mode:o} after Shows save", file.display());
            }
            assert_eq!(Shows::load_from(&path).unwrap().shows[0].title, "Private fixture");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn raw(title: &str, s: i64, e: i64, seeds: i64) -> Raw {
        Raw {
            title: title.into(),
            magnet: format!("magnet:?xt=urn:btih:{title}"),
            season: s,
            episode: e,
            seeds,
            size: 1,
            released: 0,
        }
    }

    #[test]
    fn quality_persists() {
        let root=std::env::temp_dir().join(format!("fbt-quality-{}",std::process::id()));
        let path=root.join("shows.json");
        let mut all=Shows::default();
        all.follow(&Title {id:"tt1234001".into(),name:"Local quality".into(),..Default::default()});
        for quality in [Some("2160p"),Some("1080p"),Some("720p"),Some("480p"),None] {
            all.set_quality("tt1234001",quality).unwrap();all.save_to(&path).unwrap();
            all=Shows::load_from(&path).unwrap();
            assert_eq!(all.get("tt1234001").unwrap().quality.as_deref(),quality);
        }
        assert!(all.set_quality("tt1234001",Some("4k")).is_err());
        assert!(all.get("tt1234001").unwrap().quality.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn picks_preferred_quality_then_seeds() {
        let eps = reduce(
            vec![
                raw("Show.S01E01.720p", 1, 1, 900),
                raw("Show.S01E01.1080p", 1, 1, 50),
                raw("Show.S01E02.2160p", 1, 2, 10),
                raw("Show.S01E02.720p", 1, 2, 500),
            ],
            "1080p",
        );
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].key, "S01E02"); // newest first
        assert_eq!(eps[0].title, "Show.S01E02.720p"); // no 1080p: most seeds wins
        assert_eq!(eps[1].title, "Show.S01E01.1080p");
        assert_eq!(eps[1].quality, "1080p");
        assert_eq!(eps[1].candidates, 2);
    }

    #[test]
    fn falls_back_to_release_name_and_drops_junk() {
        let eps = reduce(
            vec![
                raw("Show.S03E04.1080p.WEB", 0, 0, 5),
                raw("Show.Complete.Season.Pack", 0, 0, 5),
                Raw {
                    magnet: String::new(),
                    ..raw("Show.S03E05", 3, 5, 5)
                },
            ],
            "1080p",
        );
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].key, "S03E04");
    }

    #[test]
    fn states_and_counts() {
        let mut s = Show {
            imdb: "tt1".into(),
            title: "T".into(),
            ..Default::default()
        };
        s.episodes = reduce(
            vec![raw("a", 1, 1, 1), raw("b", 1, 2, 1), raw("c", 1, 3, 1)],
            "",
        );
        assert_eq!(s.new_count(), 3);
        s.mark_grabbed("S01E03", "magnet:x");
        assert!(s.toggle_seen("S01E02"));
        assert_eq!(s.new_count(), 1);
        assert_eq!(s.state("S01E03"), EpState::Grabbed);
        assert_eq!(s.state("S01E02"), EpState::Seen);
        assert_eq!(s.mark_all_seen(), 1);
        assert_eq!(s.new_count(), 0);
        assert!(!s.toggle_seen("S01E02"));
        assert_eq!(s.new_count(), 1);
    }

    #[test]
    fn roundtrip_and_find() {
        let dir = std::env::temp_dir().join(format!("fbt-shows-{}", std::process::id()));
        let p = dir.join("shows.json");
        let mut all = Shows::default();
        let t = Title {
            id: "tt0903747".into(),
            name: "Breaking Bad".into(),
            year: Some(2008),
            ..Default::default()
        };
        assert!(all.follow(&t));
        assert!(!all.follow(&t));
        all.get_mut("tt0903747")
            .unwrap()
            .seen
            .insert("S01E01".into());
        all.save_to(&p).unwrap();
        let back = Shows::load_from(&p).unwrap();
        assert_eq!(back.shows.len(), 1);
        assert!(back.get("tt0903747").unwrap().seen.contains("S01E01"));
        assert_eq!(back.find("breaking").unwrap().imdb, "tt0903747");
        assert!(back.find("nothing").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn eztv_json_shape() {
        let body = r#"{"torrents_count":2,"limit":100,"page":1,"torrents":[
          {"title":"Show S05E16 1080p WEB","magnet_url":"magnet:?xt=urn:btih:aa","season":"5","episode":"16","seeds":120,"size_bytes":"1500000000","date_released_unix":1700000000},
          {"title":"Show S05E16 720p HDTV","magnet_url":"magnet:?xt=urn:btih:bb","season":"5","episode":"16","seeds":300,"size_bytes":"700000000","date_released_unix":1700000100}]}"#;
        let r: EztvResp = serde_json::from_str(body).unwrap();
        assert_eq!(r.torrents_count, 2);
        let eps = reduce(
            r.torrents
                .into_iter()
                .map(|t| Raw {
                    title: t.title,
                    magnet: t.magnet_url,
                    season: as_i64(&t.season),
                    episode: as_i64(&t.episode),
                    seeds: t.seeds,
                    size: as_i64(&t.size_bytes),
                    released: t.date_released_unix,
                })
                .collect(),
            "1080p",
        );
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].magnet, "magnet:?xt=urn:btih:aa");
        assert_eq!(eps[0].size, 1_500_000_000);
    }

    #[test]
    fn refresh_retains_old_unread_releases_and_per_key_history() {
        let mut s = Show::default();
        s.episodes = reduce(
            vec![
                raw("Show.S01E01.720p", 1, 1, 10),
                raw("Show.S01E02.720p", 1, 2, 10),
            ],
            "720p",
        );
        s.mark_grabbed("S01E01", "magnet:old");
        assert!(!s.toggle_seen("S01E01")); // unread is independent of downloaded
        let new = reduce(
            vec![
                raw("Show.S03E01.1080p", 3, 1, 20),
                raw("Show.S01E01.1080p", 1, 1, 50),
            ],
            "1080p",
        );
        assert_eq!(s.merge_episodes(new), 1);
        assert_eq!(s.episodes.len(), 3);
        assert_eq!(s.episodes[0].key, "S03E01");
        assert!(s.episode("S01E02").is_some()); // missing from this source response
        assert_eq!(s.episode("S01E01").unwrap().quality, "1080p");
        assert_eq!(
            s.grabbed.get("S01E01").map(String::as_str),
            Some("magnet:old")
        );
        assert!(s.is_new("S01E01"));
        assert_eq!(s.new_count(), 3);
        assert_eq!(s.merge_episodes(Vec::new()), 0);
        assert_eq!(s.episodes.len(), 3);
    }

    #[test]
    fn season_packs_are_separate_from_episodes_and_survive_read_changes() {
        let eps = reduce(
            vec![
                raw("Show.S02.COMPLETE.720p", 2, 0, 50),
                raw("Show.S02.COMPLETE.1080p", 2, 0, 10),
                raw("Show.S02E01.1080p", 2, 1, 10),
                raw("Show.Complete.Season.3.1080p", 0, 0, 20),
                raw("Show.1x04.720p", 0, 0, 10),
            ],
            "1080p",
        );
        assert_eq!(eps.len(), 4);
        let pack = eps.iter().find(|e| e.key == "S02").unwrap();
        assert!(pack.is_season_pack());
        assert_eq!(pack.quality, "1080p");
        assert_eq!(pack.candidates, 2);
        let mut s = Show {
            episodes: eps,
            ..Default::default()
        };
        assert!(s.toggle_seen("S02"));
        assert!(!s.is_new("S02"));
        assert!(s.is_new("S02E01"));
        assert!(s.is_new("S03"));
        assert!(s.episode("S01E04").is_some());
    }

    #[test]
    fn concurrent_saves_merge_read_changes_without_resurrecting_unfollowed_shows() {
        let dir = std::env::temp_dir().join(format!("fbt-shows-merge-{}", std::process::id()));
        let p = dir.join("shows.json");
        let t = Title {
            id: "tt1".into(),
            name: "Show".into(),
            ..Default::default()
        };
        let mut initial = Shows::default();
        initial.follow(&t);
        initial.get_mut("tt1").unwrap().episodes = reduce(
            vec![raw("Show.S01E01", 1, 1, 1), raw("Show.S01E02", 1, 2, 1)],
            "",
        );
        initial.save_to(&p).unwrap();
        let mut tui = Shows::load_from(&p).unwrap();
        let mut cli = Shows::load_from(&p).unwrap();
        cli.get_mut("tt1").unwrap().toggle_seen("S01E01");
        cli.save_to(&p).unwrap();
        tui.get_mut("tt1")
            .unwrap()
            .mark_grabbed("S01E02", "magnet:second");
        tui.save_to(&p).unwrap();
        tui.save_to(&p).unwrap(); // another unchanged save retains the CLI edit
        let merged = Shows::load_from(&p).unwrap();
        assert_eq!(merged.get("tt1").unwrap().new_count(), 0);
        assert!(merged.get("tt1").unwrap().seen.contains("S01E01"));
        assert!(merged.get("tt1").unwrap().grabbed.contains_key("S01E02"));
        let mut removal = Shows::load_from(&p).unwrap();
        removal.unfollow("tt1");
        removal.save_to(&p).unwrap();
        tui.get_mut("tt1").unwrap().toggle_seen("S01E01");
        tui.save_to(&p).unwrap();
        assert!(Shows::load_from(&p).unwrap().shows.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_history_is_an_error_and_is_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("fbt-shows-corrupt-{}", std::process::id()));
        let p = dir.join("shows.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, "{broken history").unwrap();
        assert!(Shows::load_from(&p).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{broken history");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn source_page(page: usize, total: i64) -> EztvResp {
        EztvResp {
            torrents_count: total,
            torrents: (0..100)
                .map(|i| EztvItem {
                    title: format!("Show.S01E{:02}.1080p", (page - 1) * 100 + i + 1),
                    magnet_url: format!("magnet:{page}:{i}"),
                    season: 1.into(),
                    episode: ((page - 1) * 100 + i + 1).into(),
                    seeds: 1,
                    size_bytes: 1.into(),
                    date_released_unix: 10_000 - ((page - 1) * 100 + i) as i64,
                })
                .collect(),
        }
    }

    #[test]
    fn incremental_check_fetches_boundary_and_one_overlap_page() {
        let mut pages = Vec::new();
        let check = fetch_pages("1080p", Some(9_950), true, |p, _| {
            pages.push(p);
            Ok(source_page(p, 20_000))
        })
        .unwrap();
        assert_eq!(pages, [1, 2]);
        assert_eq!(check.episodes.len(), 200);
        assert!(check.catalogue_complete);
        assert!(check.warning.is_none());
    }

    #[test]
    fn large_first_catalogue_has_an_explicit_budget_warning() {
        let mut pages = 0;
        let check = fetch_pages("1080p", None, false, |p, _| {
            pages += 1;
            Ok(source_page(p, 20_000))
        })
        .unwrap();
        assert_eq!(pages, MAX_PAGES);
        assert_eq!(check.episodes.len(), MAX_PAGES * 100);
        assert!(!check.catalogue_complete);
        assert!(check
            .warning
            .unwrap()
            .contains("older source releases may be missing"));
    }
