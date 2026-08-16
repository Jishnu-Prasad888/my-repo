//! Top-level session lifecycle orchestration: creating a session, attaching
//! to one (including the single-terminal-ownership takeover flow from PRD
//! §14-16), and handing off into the recorder's blocking event loop.
//!
//! This module intentionally knows nothing about `clap` or terminal
//! prompts: the takeover decision is provided by the caller as a plain
//! closure, so the CLI (or, eventually, the TUI) owns all presentation.

use termnote_core::{LoggingSettings, SessionOwner, SessionStatus};
use termnote_storage::{sessions, SharedConn};

use crate::error::{SessionError, SessionResult};
use crate::ownership::{self, ClaimResult, SelfIdentity};
use crate::recorder::{self, StopReason};

/// What the caller wants to do when another terminal currently owns the
/// session (PRD §14).
pub enum TakeoverChoice {
    ContinueHere,
    ContinueInPreviousTerminal,
    Cancel,
}

/// Outcome of an `attach` call.
pub enum AttachOutcome {
    /// We ran the recorder loop and it eventually stopped for `reason`.
    Ran(StopReason),
    /// The user chose "continue in previous terminal": we never touched
    /// ownership and simply didn't start recording here.
    DeclinedTakeover,
}

/// Create a brand new session and immediately start recording in it
/// (PRD §11, §102: "the user simply uses the terminal normally").
pub fn new_session(db: &SharedConn, name: &str, logging: LoggingSettings) -> SessionResult<StopReason> {
    let shell = recorder::detect_shell_path()?;
    let created = sessions::create_session(db, name, Some(&shell), None)?;
    let who = SelfIdentity::detect();
    ownership::force_claim(db, &created.id, &who)?;
    let session = sessions::get_by_id(db, &created.id)?.expect("just created");
    recorder::run(db, &session, logging, false).map_err(Into::into)
}

/// Attach to an existing session. If it's currently owned by another,
/// live, terminal, `prompt` is invoked with that owner's info to decide
/// what happens next (PRD §14-16). `recovered` is invoked (informationally
/// only) if we're taking over from a session whose owner appears to have
/// crashed (PRD §61), so the caller can print a "recovering session"
/// notice before the recorder takes over the screen.
pub fn attach_session(
    db: &SharedConn,
    name: &str,
    logging: LoggingSettings,
    prompt: impl FnOnce(&SessionOwner) -> TakeoverChoice,
    recovered: impl FnOnce(&SessionOwner),
) -> SessionResult<AttachOutcome> {
    let session = sessions::require_by_name(db, name)?;
    if session.is_archived() {
        return Err(SessionError::Core(termnote_core::CoreError::InvalidSessionStatus(
            "session is archived; run `termnote restore` first".to_string(),
        )));
    }

    let who = SelfIdentity::detect();
    // Snapshot before claiming: `try_claim`/`force_claim` immediately flips
    // the session's status to ACTIVE, so "was this session previously new
    // vs. previously detached/recovered" must be decided from this
    // pre-claim view, not from a re-fetch afterward.
    let is_resume = !matches!(session.status, SessionStatus::New);

    match ownership::try_claim(db, &session, &who)? {
        ClaimResult::Claimed => {}
        ClaimResult::Recovered(previous) => recovered(&previous),
        ClaimResult::OwnedByOther(owner) => match prompt(&owner) {
            TakeoverChoice::ContinueHere => {
                ownership::force_claim(db, &session.id, &who)?;
                // Best-effort nudge (PRD §15): if the previous owner is on
                // this host, ask its process to shut down too. Whether or
                // not the signal lands, that process's own ownership
                // watchdog will notice the DB no longer names it as owner
                // and will exit gracefully on its own.
                if owner.host == who.host {
                    unsafe {
                        libc::kill(owner.pid, libc::SIGTERM);
                    }
                }
            }
            TakeoverChoice::ContinueInPreviousTerminal => return Ok(AttachOutcome::DeclinedTakeover),
            TakeoverChoice::Cancel => return Err(SessionError::AttachCancelled(name.to_string())),
        },
    }

    let session = sessions::get_by_id(db, &session.id)?.expect("session exists: we just touched it");
    let reason = recorder::run(db, &session, logging, is_resume)?;
    Ok(AttachOutcome::Ran(reason))
}
