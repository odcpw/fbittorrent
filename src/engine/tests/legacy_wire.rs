// These expected bytes came from the production handlers at main 69e8377,
// before the typed-operation change. See legacy-wire-69e8377/origin.json.
#[test]
fn legacy_wire_json_matches_main_69e8377() {
    let directory = review_directory("p119-wire");
    std::fs::create_dir_all(&directory).unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args([
        "--ignored", "--exact", "--nocapture",
        "engine::tests::legacy_wire_fixture_probe",
    ]).env("FBT_P119_WIRE_ROOT", &directory);
    for (key, name) in [
        ("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
        ("XDG_RUNTIME_DIR", "runtime"), ("XDG_CACHE_HOME", "cache"),
        ("XDG_STATE_HOME", "state"),
    ] {
        let path = directory.join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        command.env(key, path);
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "wire probe failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    macro_rules! frozen {
        ($name:literal) => { ($name, include_bytes!(concat!("legacy-wire-69e8377/", $name, ".json")).as_slice()) };
    }
    let mut differences = Vec::new();
    for (name, expected) in [
        frozen!("parked-info"), frozen!("parked-files-multi"),
        frozen!("parked-files-single"), frozen!("parked-queue"),
        frozen!("parked-empty-info"), frozen!("parked-empty-queue"),
        frozen!("live-queue"), frozen!("live-files"),
        frozen!("settings-unbound"), frozen!("settings-bound"),
    ] {
        let actual = std::fs::read(directory.join("actual").join(format!("{name}.json"))).unwrap();
        if actual != expected {
            let byte = actual.iter().zip(expected).position(|(a, b)| a != b)
                .unwrap_or(actual.len().min(expected.len()));
            differences.push(format!("{name}: first difference at byte {byte}; expected {} bytes, actual {} bytes", expected.len(), actual.len()));
        } else {
            println!("legacy wire main 69e8377: {name} byte-identical ({} bytes)", actual.len());
        }
    }
    assert!(differences.is_empty(), "{}\nActual artifacts retained in {}", differences.join("\n"), directory.display());
    std::fs::remove_dir_all(directory).unwrap();
}
