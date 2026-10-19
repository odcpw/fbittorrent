//! Stop admission, drain accepted work and close each client with bye.

use super::*;

impl Server {
    pub fn stop_accepting(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        if let Some(thread) = self.accept.take() {
            let _ = thread.join();
        }
    }
    pub fn drain(&mut self, reason: StopReason) {
        self.stop_accepting();
        self.shared.pool.close();
        for thread in self.dispatch.drain(..) {
            let _ = thread.join();
        }
        let reason = match reason {
            StopReason::Idle => ByeReason::Idle,
            StopReason::Upgrade => ByeReason::Upgrade,
            _ => ByeReason::Shutdown,
        };
        for client in self.shared.clients.lock().unwrap().values() {
            let mut pending = client.pending.lock().unwrap();
            for id in pending.keys() {
                client.send(response(Some(*id), stopping()));
            }
            pending.clear();
            drop(pending);
            client.finish(reason);
        }
        for thread in self.connections.lock().unwrap().drain(..) {
            let _ = thread.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.drain(StopReason::Quit);
        let _ = std::fs::remove_file(&self.path);
    }
}
