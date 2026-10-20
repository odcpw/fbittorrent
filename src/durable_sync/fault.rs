//! Test-only, private subprocesses: never install seccomp in the suite runner.
use std::os::unix::fs::DirBuilderExt;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) fn deny_setup(errno: i32) {
    let filter = [
        libc::sock_filter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        libc::sock_filter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: libc::SYS_io_uring_setup as u32,
        },
        libc::sock_filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: 0x0005_0000 | errno as u32,
        },
        libc::sock_filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: 0x7fff_0000,
        },
    ];
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr() as *mut _,
    };
    // SAFETY: the kernel copies the BPF program. TSYNC applies this only to
    // our isolated child and its threads; all syscalls except ring setup work.
    unsafe {
        assert_eq!(
            libc::prctl(
                libc::PR_SET_NO_NEW_PRIVS,
                1 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong
            ),
            0
        );
        assert_eq!(
            libc::syscall(
                libc::SYS_seccomp,
                1 as libc::c_ulong,
                1 as libc::c_ulong,
                &program
            ),
            0
        );
    }
    assert_eq!(
        io_uring::IoUring::new(2).err().unwrap().raw_os_error(),
        Some(errno),
        "negative control: real ring setup must be denied"
    );
}

pub(crate) fn isolated(errors: &[i32], run: impl FnOnce(i32)) {
    const CHILD: &str = "FBT_TEST_DURABLE_SYNC_ERRNO";
    if let Ok(errno) = std::env::var(CHILD) {
        run(errno.parse().unwrap());
        return;
    }
    let test = std::thread::current().name().unwrap().to_owned();
    for errno in errors {
        let root = std::env::temp_dir().join(format!(
            "fbt-fsync-fault-{}-{}-{}",
            std::process::id(),
            errno,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", &test, "--nocapture", "--test-threads=1"])
            .env(CHILD, errno.to_string())
            .current_dir(&root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in [
            "HOME",
            "TMPDIR",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_RUNTIME_DIR",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
        ] {
            command.env(key, &root);
        }
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        let output = child.wait_with_output().unwrap();
        println!("errno={errno}: {}", String::from_utf8_lossy(&output.stdout));
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            output.status.success(),
            "fault child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "child must execute its test"
        );
    }
}

pub(crate) fn root() -> PathBuf {
    std::env::current_dir().unwrap()
}
