//! Block before spawning any daemon thread; one sigwait consumer owns shutdown.
use super::lifecycle::{Lifecycle, StopReason};
use anyhow::{Context, Result};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn stop_set() -> libc::sigset_t {
    unsafe {
        let mut set = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, signal);
        }
        set
    }
}

pub struct Signals {
    old: libc::sigset_t,
    old_pipe: libc::sighandler_t,
    done: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Signals {
    pub fn install(lifecycle: Lifecycle) -> Result<Self> {
        let mut old = unsafe { std::mem::zeroed() };
        let set = stop_set();
        let code = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old) };
        if code != 0 {
            return Err(std::io::Error::from_raw_os_error(code).into());
        }
        let old_pipe = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let thread = std::thread::Builder::new()
            .name("fbittorrent-signal".into())
            .spawn(move || loop {
                let mut number = 0;
                let code = unsafe { libc::sigwait(&set, &mut number) };
                if finished.load(Ordering::Acquire) {
                    break;
                }
                if code == 0 {
                    lifecycle.request_stop(StopReason::Signal(number));
                } else {
                    eprintln!("ERROR daemon sigwait failed errno={code}");
                    lifecycle.request_stop(StopReason::Panic);
                    break;
                }
            })
            .context("starting daemon signal consumer");
        match thread {
            Ok(thread) => Ok(Self {
                old,
                old_pipe,
                done,
                thread: Some(thread),
            }),
            Err(error) => {
                unsafe {
                    libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
                    libc::signal(libc::SIGPIPE, old_pipe);
                }
                Err(error)
            }
        }
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        use std::os::unix::thread::JoinHandleExt;
        self.done.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            unsafe {
                libc::pthread_kill(thread.as_pthread_t(), libc::SIGTERM);
            }
            let _ = thread.join();
        }
        unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.old, std::ptr::null_mut());
            libc::signal(libc::SIGPIPE, self.old_pipe);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mask_inherited() {
        use std::os::unix::process::CommandExt;
        if std::env::var_os("FBT_SIGNAL_UNIT_CHILD").is_none() {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "daemon::signals::tests::mask_inherited",
                    "--nocapture",
                ])
                .env("FBT_SIGNAL_UNIT_CHILD", "1");
            // Mask before exec, so even libtest's main thread inherits it. The
            // parent test runner's other tests must never receive our SIGTERM.
            unsafe {
                command.pre_exec(|| {
                    let code =
                        libc::pthread_sigmask(libc::SIG_BLOCK, &stop_set(), std::ptr::null_mut());
                    if code == 0 {
                        Ok(())
                    } else {
                        Err(std::io::Error::from_raw_os_error(code))
                    }
                });
            }
            assert!(command.status().unwrap().success());
            return;
        }
        let lifecycle = Lifecycle::default();
        let signals = Signals::install(lifecycle.clone()).unwrap();
        std::thread::spawn(|| unsafe {
            let mut mask = std::mem::zeroed();
            assert_eq!(
                libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut mask),
                0
            );
            for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
                assert_eq!(libc::sigismember(&mask, signal), 1);
            }
            assert_eq!(libc::kill(libc::getpid(), libc::SIGTERM), 0);
        })
        .join()
        .unwrap();
        let observed = lifecycle.wait(std::time::Duration::from_secs(2));
        println!("worker inherited mask; sigwait received {observed:?}");
        assert_eq!(
            observed,
            Some(super::super::lifecycle::Request::Stop(StopReason::Signal(
                libc::SIGTERM
            )))
        );
        drop(signals);
    }
}
