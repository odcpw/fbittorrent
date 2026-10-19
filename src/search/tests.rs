//! Existing search/process/provider regression tests, moved unchanged.
    use super::*;

    static SEARCH_TESTS: Mutex<()> = Mutex::new(());

    fn session() -> std::sync::MutexGuard<'static, ()> {
        let guard = SEARCH_TESTS.lock().unwrap();
        begin_session().unwrap();
        guard
    }

    fn live_processes(group: i32) -> Vec<i32> {
        std::fs::read_dir("/proc")
            .unwrap()
            .flatten()
            .filter_map(|entry| {
                let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
                let (pid, tail) = stat.split_once(" (")?;
                let (_, tail) = tail.rsplit_once(") ")?;
                let fields: Vec<_> = tail.split_whitespace().collect();
                (fields.first() != Some(&"Z")
                    && fields.get(2).and_then(|value| value.parse::<i32>().ok()) == Some(group))
                .then(|| pid.parse().ok())
                .flatten()
            })
            .collect()
    }

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn scratch() -> (Scratch, ProviderStore) {
        let dir = std::env::temp_dir().join(format!(
            "fbittorrent-search-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        (
            Scratch(dir.clone()),
            ProviderStore {
                data: dir,
                legacy: None,
            },
        )
    }

    #[test]
    fn search_save_is_private_for_new_and_existing_stores() {
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        for replacement in [false, true] {
            if replacement {
                for path in [store.preferences(), store.data.join("search.lock")] {
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
                }
            }
            let _lock = store.lock().unwrap();
            let mut preferences = store.load().unwrap();
            preferences.enabled.insert("private_fixture".into(), replacement);
            store.save(&preferences).unwrap();
            for path in [store.preferences(), store.data.join("search.lock")] {
                let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "{}", path.display());
                println!("{}: {mode:o} after search save", path.display());
            }
            assert_eq!(store.load().unwrap().enabled["private_fixture"], replacement);
        }
    }

    #[test]
    fn nova_results_preserve_urls_sizes_providers_and_dates() {
        let providers = vec![SearchPlugin {
            name: "example".into(),
            url: "https://example.org/".into(),
            ..SearchPlugin::default()
        }];
        let result = parse_result("magnet:?xt=urn:btih:abc|Example S01E01|1234|100|2|https://example.org|https://example.org/1|1791547200\n", &providers).unwrap();
        assert_eq!(result.engine, "example");
        assert_eq!(result.file_size, 1234);
        assert_eq!(result.seeders, 100);
        assert_eq!(result.pub_date, 1791547200);
        assert!(parse_result("a provider printed a diagnostic", &providers).is_none());
    }

    #[test]
    fn providers_are_imported_without_changing_original_files() {
        let _session = session();
        let (_scratch, mut store) = scratch();
        let legacy = store.data.join("legacy");
        std::fs::create_dir_all(legacy.join("engines")).unwrap();
        let source = b"# VERSION: 1\nclass example:\n    name = 'Example & Friends'\n    url = 'https://example.org'\n    def search(self, query): pass\n";
        std::fs::write(legacy.join("engines/example.py"), source).unwrap();
        store.legacy = Some(legacy.clone());
        let providers = plugins_in(&store).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].full_name, "Example & Friends");
        assert_eq!(
            std::fs::read(legacy.join("engines/example.py")).unwrap(),
            source
        );
        store.set_enabled("example", false).unwrap();
        assert!(!plugins_in(&store).unwrap()[0].enabled);
        assert!(start_in(&store, "test").is_err());
    }

    #[test]
    fn selected_detail_result_runs_its_provider_download_hook() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(
            store.runtime().join("engines/example.py"),
            r#"class example:
    def download_torrent(self, parameter):
        import pathlib
        pathlib.Path('download-hook-called').write_text(parameter)
        print('magnet:?xt=urn:btih:0123456789012345678901234567890123456789 ' + parameter)
"#,
        )
        .unwrap();
        let result = SearchResult {
            file_url: "https://example.org/details/123".into(),
            engine: "example".into(),
            ..SearchResult::default()
        };
        assert_eq!(
            resolve_download_in(&store, &result, 5).unwrap(),
            ResolvedDownload::Url(
                "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into()
            )
        );
        assert_eq!(
            std::fs::read_to_string(store.runtime().join("download-hook-called")).unwrap(),
            result.file_url
        );
        assert!(jobs().lock().unwrap().processes.is_empty());
    }

    #[test]
    fn selected_provider_torrent_file_is_read_and_its_scratch_is_removed() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/example.py"), r#"class example:
    def download_torrent(self, parameter):
        import os, pathlib
        target = pathlib.Path(os.environ['TMPDIR']) / 'payload file.torrent'
        target.write_bytes(b'd4:infod6:lengthi1e4:name1:x12:piece lengthi4e6:pieces20:aaaaaaaaaaaaaaaaaaaaee')
        pathlib.Path('download-file-path').write_text(str(target))
        print(str(target) + ' https://example.org/download')
"#).unwrap();
        let result = SearchResult {
            file_url: "opaque-provider-id".into(),
            engine: "example".into(),
            ..Default::default()
        };
        let resolved = resolve_download_in(&store, &result, 5).unwrap();
        let ResolvedDownload::Torrent { filename, bytes } = resolved else {
            panic!("expected a torrent file");
        };
        assert_eq!(filename, "payload file.torrent");
        assert!(librqbit::torrent_from_bytes(&bytes).is_ok());
        let temporary =
            std::fs::read_to_string(store.runtime().join("download-file-path")).unwrap();
        assert!(!Path::new(&temporary).exists());
        assert!(!Path::new(&temporary).parent().unwrap().exists());
    }

    #[test]
    fn direct_search_download_bypasses_python_even_after_shutdown() {
        let _session = session();
        let (_scratch, store) = scratch();
        shutdown_all();
        for url in [
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
            "https://example.org/payload.torrent?token=fixture",
        ] {
            let result = SearchResult {
                file_url: url.into(),
                engine: "../../untrusted-name".into(),
                ..Default::default()
            };
            assert_eq!(
                resolve_download_in(&store, &result, 5).unwrap(),
                ResolvedDownload::Url(url.into())
            );
        }
        assert!(!store.data.exists());
        assert!(jobs().lock().unwrap().processes.is_empty());
    }

    #[test]
    fn selected_download_hook_obeys_closed_spawn_gate() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(
            store.runtime().join("engines/example.py"),
            "raise RuntimeError('must never import after shutdown')",
        )
        .unwrap();
        shutdown_all();
        let result = SearchResult {
            file_url: "https://example.org/details/123".into(),
            engine: "example".into(),
            ..Default::default()
        };
        assert!(
            format!("{:#}", resolve_download_in(&store, &result, 5).unwrap_err())
                .contains("closing")
        );
        assert!(jobs().lock().unwrap().processes.is_empty());
    }

    #[test]
    fn selected_download_timeout_stops_provider_children() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(
            store.runtime().join("engines/example.py"),
            r#"class example:
    def download_torrent(self, parameter):
        import os, pathlib, subprocess, sys, time
        subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
        pathlib.Path('download-group').write_text(str(os.getpgrp()))
        time.sleep(60)
"#,
        )
        .unwrap();
        let result = SearchResult {
            file_url: "https://example.org/details/123".into(),
            engine: "example".into(),
            ..Default::default()
        };
        let failure = resolve_download_in(&store, &result, 1).unwrap_err();
        assert!(format!("{failure:#}").contains("timed out"));
        let group = std::fs::read_to_string(store.runtime().join("download-group"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(live_processes(group).is_empty());
        assert!(jobs().lock().unwrap().processes.is_empty());
    }

    #[test]
    fn active_search_streams_results_and_stop_kills_python_process_group() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/example.py"), r#"class example:
    name = 'Example'
    url = 'https://example.org'
    def search(self, query):
        import os, subprocess, sys, time
        from novaprinter import prettyPrinter
        child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
        open('provider-child.pid', 'w').write(str(child.pid))
        prettyPrinter({'link':'magnet:?xt=urn:btih:abc','name':'Fixture','size':123,'seeds':4,'leech':1,'engine_url':self.url})
        time.sleep(60)
"#).unwrap();
        let id = start_in(&store, "fixture").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if results(id, 100, 0).unwrap().total > 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "search never streamed its fixture result"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let process_group = job(id).unwrap().pid.load(Ordering::Acquire);
        stop(id).unwrap();
        assert_eq!(results(id, 100, 0).unwrap().status, "Stopped");
        // Zombies can briefly remain until the host reaps orphaned processes;
        // verify every member is terminated rather than testing /proc absence.
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            if let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) {
                if let Some((_, tail)) = stat.rsplit_once(") ") {
                    let fields: Vec<_> = tail.split_whitespace().collect();
                    if fields.get(2).and_then(|value| value.parse::<i32>().ok())
                        == Some(process_group)
                    {
                        assert_eq!(
                            fields[0], "Z",
                            "live process remained in search process group"
                        );
                    }
                }
            }
        }
        delete(id).unwrap();
    }

    #[test]
    fn shutdown_while_provider_metadata_loads_closes_the_spawn_gate() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/example.py"), "import pathlib,time\npathlib.Path('metadata-started').write_text('yes')\ntime.sleep(60)\nclass example:\n name='Example'\n url='https://example.org'\n def search(self,query): pass\n").unwrap();
        let worker_store = store.clone();
        let starting = thread::spawn(move || start_in(&worker_store, "fixture"));
        let deadline = Instant::now() + Duration::from_secs(8);
        while !store.runtime().join("metadata-started").exists() {
            assert!(Instant::now() < deadline, "metadata runner never started");
            thread::sleep(Duration::from_millis(20));
        }
        let groups = jobs().lock().unwrap().processes.clone();
        assert!(!groups.is_empty());
        shutdown_all();
        assert!(starting.join().unwrap().is_err());
        assert!(start_in(&store, "late query")
            .unwrap_err()
            .to_string()
            .contains("closing"));
        let registry = jobs().lock().unwrap();
        assert!(registry.jobs.is_empty());
        assert!(registry.processes.is_empty());
        drop(registry);
        for group in groups {
            assert!(live_processes(group).is_empty());
        }
        begin_session().unwrap();
        let mut command = python(&store.runtime());
        command.args(["-c", "print('reopened')"]);
        assert_eq!(
            capture(command, Duration::from_secs(2)).unwrap(),
            b"reopened\n"
        );
    }

    #[test]
    fn provider_failure_keeps_partial_results_and_reports_the_failure() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/example.py"), "class example:\n name='Example'\n url='https://example.org'\n def search(self,query):\n  from novaprinter import prettyPrinter\n  prettyPrinter({'link':'magnet:?xt=urn:btih:abc','name':'Result before failure','size':'1 GiB','seeds':4,'leech':1,'engine_url':self.url})\n  raise RuntimeError('fixture provider failed')\n").unwrap();
        let id = start_in(&store, "fixture").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let response = loop {
            let response = results(id, 100, 0).unwrap();
            if response.status != "Running" {
                break response;
            }
            assert!(Instant::now() < deadline, "failing provider did not finish");
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].file_size, 1 << 30);
        assert!(response.error.unwrap().contains("fixture provider failed"));
        delete(id).unwrap();
    }

    #[test]
    fn nova_parent_death_kills_provider_created_subprocesses() {
        let _session = session();
        let (_scratch, store) = scratch();
        store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/example.py"), "class example:\n name='Example'\n url='https://example.org'\n def search(self,query):\n  import pathlib,subprocess,sys,time\n  child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'])\n  pathlib.Path('provider-child.pid').write_text(str(child.pid))\n  time.sleep(60)\n").unwrap();
        let mut command = python(&store.runtime());
        command.args(["-c", r#"import ctypes,os,pathlib,signal,subprocess,sys,time
def prepare():
    os.setpgid(0,0)
    ctypes.CDLL(None).prctl(1,signal.SIGTERM,0,0,0)
child=subprocess.Popen([sys.executable,'nova2.py','example','all','fixture'],preexec_fn=prepare,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
pathlib.Path('nova.pid').write_text(str(child.pid))
deadline=time.monotonic()+5
while not pathlib.Path('provider-child.pid').exists():
    if time.monotonic()>deadline: raise RuntimeError('provider child never started')
    time.sleep(.02)
os._exit(0)
"#]);
        capture(command, Duration::from_secs(8)).unwrap();
        let group: i32 = std::fs::read_to_string(store.runtime().join("nova.pid"))
            .unwrap()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !live_processes(group).is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        let live = live_processes(group);
        // Always clean up a regressed fixture before reporting the failure.
        if !live.is_empty() {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        assert!(
            live.is_empty(),
            "provider subprocesses survived Nova parent death: {live:?}"
        );
    }

    #[test]
    fn dedupe_key_btih() {
        let first = SearchResult { file_url: "magnet:?xt=urn:btih:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&dn=first".into(), ..Default::default() };
        let second = SearchResult { file_url: "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&tr=elsewhere".into(), ..Default::default() };
        assert_eq!(normalize::dedupe_key(&first), normalize::dedupe_key(&second));
        assert_eq!(normalize::dedupe_key(&first), "a".repeat(40));
        let (_scratch, store) = scratch();
        std::fs::create_dir_all(&store.data).unwrap();
        let path = store.data.join("metadata.torrent");
        let bytes = b"d4:infod6:lengthi1e4:name1:x12:piece lengthi4e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";
        std::fs::write(&path,bytes).unwrap();
        assert_eq!(normalize::torrent_hash(path.to_str().unwrap()).unwrap(), librqbit::torrent_from_bytes(bytes).unwrap().info_hash.as_string());
    }

    #[test]
    fn dedupe_key_title_size() {
        let first = SearchResult { file_name: "  Sintel   2010 ".into(), file_size: 1234, file_url: "https://alpha.invalid/one".into(), engine: "alpha".into(), seeders: 1, ..Default::default() };
        let second = SearchResult { file_name: "sintel 2010".into(), file_url: "https://beta.invalid/two".into(), engine: "beta".into(), seeders: 9, ..first.clone() };
        assert_eq!(normalize::dedupe_key(&first),normalize::dedupe_key(&second));
        let items = normalize::aggregate(&[first.clone(),second.clone()],5000);
        assert_eq!(items.len(),1); assert_eq!(items[0].providers, vec!["alpha","beta"]); assert_eq!(items[0].seeds,Some(9));
        assert_eq!(items[0].url.as_deref(),Some(second.file_url.as_str()));
        let different = SearchResult { file_size: 1235, ..first };
        assert_ne!(normalize::dedupe_key(&different),normalize::dedupe_key(&second));
    }

    #[test]
    fn mojibake_roundtrip() {
        for (raw,expected) in [("è¾›ç‰¹å°”","辛特尔"),("Amélie","Amélie"),("Straße","Straße"),("Sintel","Sintel")] {
            assert_eq!(normalize::repair_name(raw),expected); println!("{raw:?} -> {expected:?}");
        }
    }

    #[test]
    fn raw_provider_bytes_survive_normalization_and_capture_is_bounded() {
        let _session = session(); let (_scratch,store) = scratch(); store.ensure_runtime().unwrap();
        std::fs::write(store.runtime().join("engines/raw.py"),r#"class raw:
 name='Raw'
 url='https://raw.invalid'
 def search(self,query):
  print('magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|è¾›ç‰¹å°”|123|9|-1|https://raw.invalid',flush=True)
  print('x'*(3*1024*1024),flush=True)
"#).unwrap();
        let id = start_in(&store,"fixture").unwrap(); let deadline = Instant::now()+Duration::from_secs(8);
        while !job(id).unwrap().done.load(Ordering::Acquire) { assert!(Instant::now()<deadline); thread::sleep(Duration::from_millis(20)); }
        let selected=job(id).unwrap(); let state=selected.state.lock().unwrap();
        assert!(std::str::from_utf8(&state.raw).unwrap().contains("è¾›ç‰¹å°”"));
        assert!(state.raw.len() <= MAX_CAPTURE_BYTES); assert!(state.error.contains("exceeded 2 MiB"));
        let items=normalize::aggregate(&state.results,5000); assert_eq!(items[0].name,"辛特尔"); assert_eq!(items[0].leechers,None);
        println!("retained raw={} bytes, normalized={:?}, error={}",state.raw.len(),items,state.error);
        drop(state); delete(id).unwrap();
    }
