use super::wire::Wire;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};

struct Request {
    op: &'static str,
    args: Value,
    reply: mpsc::SyncSender<Result<Value, String>>,
}
/// One connection per UI, with bounded commands and prompt cancellation on drop.
pub(crate) struct Presence {
    commands: mpsc::SyncSender<Request>,
    closed: Arc<AtomicBool>,
    interrupt: UnixStream,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Presence {
    pub(in crate::tui) fn connect() -> Result<Arc<Self>> {
        let mut wire = Wire::connect()?;
        let interrupt = wire.interrupt()?;
        let (commands, requests) = mpsc::sync_channel::<Request>(4);
        let closed = Arc::new(AtomicBool::new(false));
        let stopping = closed.clone();
        let thread = std::thread::Builder::new()
            .name("fbittorrent-tui-presence".into())
            .spawn(move || {
                let mut heartbeat = Instant::now();
                while !stopping.load(Ordering::Acquire) {
                    match requests.try_recv() {
                        Ok(request) => {
                            let result = wire
                                .call(request.op, request.args)
                                .map_err(|error| error.to_string());
                            let failed = result.is_err();
                            let _ = request.reply.send(result);
                            if failed {
                                break;
                            }
                        }
                        Err(mpsc::TryRecvError::Disconnected) => break,
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                    if heartbeat.elapsed() >= Duration::from_secs(5) {
                        if wire
                            .call("app.client", json!({"interactive":true}))
                            .is_err()
                        {
                            break;
                        }
                        heartbeat = Instant::now();
                    }
                    match wire.receive() {
                        Ok(Some(frame)) if frame["ev"] == "bye" => break,
                        Err(_) => break,
                        _ => {}
                    }
                }
                stopping.store(true, Ordering::Release);
            })?;
        Ok(Arc::new(Self {
            commands,
            closed,
            interrupt,
            thread: Some(thread),
        }))
    }
    pub(in crate::tui) fn request(&self, op: &'static str, args: Value) -> Result<Value> {
        if self.closed.load(Ordering::Acquire) {
            bail!("fbittorrent stopped");
        }
        let (reply, result) = mpsc::sync_channel(1);
        self.commands
            .try_send(Request { op, args, reply })
            .map_err(|_| anyhow::anyhow!("API connection busy or closed"))?;
        result
            .recv_timeout(Duration::from_secs(4))
            .map_err(|_| anyhow::anyhow!("API request timed out; outcome is unconfirmed"))?
            .map_err(anyhow::Error::msg)
    }
    pub(in crate::tui) fn alive(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }
}
impl Drop for Presence {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.interrupt.shutdown(Shutdown::Both);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
