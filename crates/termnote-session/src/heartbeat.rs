//! Background heartbeat thread (PRD §62).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use termnote_storage::{sessions, SharedConn};

use crate::ownership::HEARTBEAT_INTERVAL_SECS;

pub struct HeartbeatHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl HeartbeatHandle {
    /// Wrap an already-spawned thread + stop flag (used by the ownership
    /// watchdog, which shares this same stop/join lifecycle but runs
    /// different logic per tick).
    pub fn from_raw(stop: Arc<AtomicBool>, join: JoinHandle<()>) -> Self {
        Self { stop, join: Some(join) }
    }

    /// Signal the thread to stop and wait for it to finish. Uses a short
    /// polling interval internally so shutdown stays responsive (PRD §95)
    /// rather than blocking on the full heartbeat period.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

pub fn spawn(db: SharedConn, session_id: String) -> HeartbeatHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    let join = std::thread::Builder::new()
        .name("termnote-heartbeat".into())
        .spawn(move || {
            let chunk = Duration::from_millis(200);
            let mut elapsed = Duration::ZERO;
            let period = Duration::from_secs(HEARTBEAT_INTERVAL_SECS);
            // Beat once immediately so a freshly claimed session doesn't
            // look stale before the first tick.
            let _ = sessions::heartbeat(&db, &session_id, termnote_core::time::now_unix_ns());
            while !stop_clone.load(Ordering::SeqCst) {
                std::thread::sleep(chunk);
                elapsed += chunk;
                if elapsed >= period {
                    elapsed = Duration::ZERO;
                    let _ = sessions::heartbeat(&db, &session_id, termnote_core::time::now_unix_ns());
                }
            }
        })
        .expect("failed to spawn heartbeat thread");

    HeartbeatHandle { stop, join: Some(join) }
}
