//! Who "we" are (for the ownership lock) and the claim/release/heartbeat
//! policy built on top of `termnote-storage`'s atomic compare-and-swap
//! primitives (PRD §13-17, §61-63).

use termnote_core::{time::now_unix_ns, Session, SessionOwner, SessionStatus};
use termnote_storage::{sessions, SharedConn, StorageResult};

/// How often the owning terminal refreshes its heartbeat.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 2;
/// How long without a heartbeat before another terminal may consider a
/// session abandoned and safe to reclaim (PRD §63: never steal merely
/// because of a *temporary* delay -- this should comfortably exceed a few
/// missed heartbeats).
pub const STALE_AFTER_SECS: i64 = 10;

#[derive(Debug, Clone)]
pub struct SelfIdentity {
    pub pid: i32,
    pub host: String,
    pub terminal: String,
}

impl SelfIdentity {
    pub fn detect() -> Self {
        Self {
            pid: std::process::id() as i32,
            host: hostname(),
            terminal: current_tty(),
        }
    }
}

fn hostname() -> String {
    let mut buf = vec![0u8; 256];
    let ret = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if ret != 0 {
        return "unknown-host".to_string();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

fn current_tty() -> String {
    unsafe {
        let ptr = libc::ttyname(0);
        if ptr.is_null() {
            return "unknown-tty".to_string();
        }
        std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

pub enum ClaimResult {
    /// The session was unowned; we claimed it normally.
    Claimed,
    /// The session appeared owned, but its heartbeat was stale (PRD §61):
    /// the previous owner likely crashed. We claimed it anyway; the caller
    /// should surface a "recovering session" notice rather than a silent
    /// takeover.
    Recovered(SessionOwner),
    /// The session is actively owned elsewhere and its heartbeat is fresh;
    /// the caller must resolve this via the takeover prompt (PRD §14).
    OwnedByOther(SessionOwner),
}

/// Attempt to claim ownership. Succeeds outright if the session is unowned
/// or its owner's heartbeat has gone stale; otherwise reports who currently
/// owns it so the caller (the CLI) can present the takeover prompt from
/// PRD §14.
pub fn try_claim(db: &SharedConn, session: &Session, who: &SelfIdentity) -> StorageResult<ClaimResult> {
    let now = now_unix_ns();
    let stale_before = now - STALE_AFTER_SECS * 1_000_000_000;
    let previous_owner = session.owner.clone();

    let claimed = sessions::try_claim_ownership(
        db,
        &session.id,
        who.pid,
        &who.host,
        &who.terminal,
        now,
        stale_before,
    )?;

    if claimed {
        return Ok(match previous_owner {
            Some(owner) => ClaimResult::Recovered(owner),
            None => ClaimResult::Claimed,
        });
    }

    let fresh = sessions::get_by_id(db, &session.id)?
        .expect("session existed a moment ago and cannot have been hard-deleted concurrently by this single-user CLI flow");
    Ok(ClaimResult::OwnedByOther(
        fresh
            .owner
            .expect("try_claim_ownership only fails to claim when an owner is present"),
    ))
}

/// Unconditionally take ownership. Only used after the user has explicitly
/// chosen "continue here" (PRD §15) or after the previous owner has been
/// confirmed unreachable by the recovery flow.
pub fn force_claim(db: &SharedConn, session_id: &str, who: &SelfIdentity) -> StorageResult<()> {
    sessions::force_claim_ownership(db, session_id, who.pid, &who.host, &who.terminal)
}

pub fn release(db: &SharedConn, session_id: &str, status: SessionStatus) -> StorageResult<()> {
    sessions::release_ownership(db, session_id, status)
}
