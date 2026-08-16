//! The event model. Everything that happens inside a session is an
//! append-only `Event` (PRD §19, §76). Structured, type-specific data lives
//! in `payload` as JSON so the schema can evolve without migrations for
//! every new field.

use serde::{Deserialize, Serialize};

use crate::errors::{CoreError, CoreResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    SessionStart,
    SessionAttach,
    SessionDetach,
    SessionEnd,
    Command,
    Output,
    Note,
    Bookmark,
    SettingChange,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::SessionStart => "SESSION_START",
            EventType::SessionAttach => "SESSION_ATTACH",
            EventType::SessionDetach => "SESSION_DETACH",
            EventType::SessionEnd => "SESSION_END",
            EventType::Command => "COMMAND",
            EventType::Output => "OUTPUT",
            EventType::Note => "NOTE",
            EventType::Bookmark => "BOOKMARK",
            EventType::SettingChange => "SETTING_CHANGE",
        }
    }

    pub fn parse(s: &str) -> CoreResult<Self> {
        Ok(match s {
            "SESSION_START" => EventType::SessionStart,
            "SESSION_ATTACH" => EventType::SessionAttach,
            "SESSION_DETACH" => EventType::SessionDetach,
            "SESSION_END" => EventType::SessionEnd,
            "COMMAND" => EventType::Command,
            "OUTPUT" => EventType::Output,
            "NOTE" => EventType::Note,
            "BOOKMARK" => EventType::Bookmark,
            "SETTING_CHANGE" => EventType::SettingChange,
            other => return Err(CoreError::InvalidEventType(other.to_string())),
        })
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Structured payload for a `COMMAND` event (PRD §21, §48).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandPayload {
    pub command: String,
    pub exit_code: Option<i32>,
    pub cwd: Option<String>,
    pub hostname: Option<String>,
    pub shell: Option<String>,
    pub terminal_cols: Option<u16>,
    pub terminal_rows: Option<u16>,
    /// True once an end boundary (foreground-process-group return, or a
    /// shell-integration report) has been observed for this command.
    pub closed: bool,
    /// Best-effort indicator of how the end boundary / exit code was
    /// determined, purely informational (`"pgrp"`, `"shell-hook"`,
    /// `"next-command"`, `"session-end"`).
    pub resolution: Option<String>,
}

/// Structured payload for an `OUTPUT` event (PRD §22-23).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputPayload {
    /// The `id` of the `COMMAND` event this output chunk belongs to, if any
    /// command was open when it was captured.
    pub command_event_id: Option<i64>,
    /// Raw bytes, lossily decoded to UTF-8 (invalid sequences replaced).
    /// ANSI/control sequences are preserved verbatim per PRD §22.
    pub text: String,
    pub byte_len: usize,
    pub stream: OutputStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputStream {
    /// PTY semantics merge stdout/stderr in the common case (PRD §23).
    Merged,
}

/// Structured payload for a `NOTE` event (PRD §30, §32).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotePayload {
    pub markdown: String,
}

/// Structured payload for a `BOOKMARK` event (PRD §34-36). A bookmark is a
/// pointer, not a copy: `target_event_id` references the event it marks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkPayload {
    pub name: Option<String>,
    pub target_event_id: i64,
}

/// Structured payload for session lifecycle events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionLifecyclePayload {
    pub host: Option<String>,
    pub pid: Option<i32>,
    pub terminal: Option<String>,
    pub note: Option<String>,
}

/// Structured payload for a `SETTING_CHANGE` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingChangePayload {
    pub key: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
}

/// A single row in the append-only event log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Option<i64>,
    pub session_id: String,
    pub sequence: i64,
    pub event_type: EventType,
    pub timestamp_start: Option<i64>,
    pub timestamp_end: Option<i64>,
    pub duration_ns: Option<i64>,
    pub payload: serde_json::Value,
}

impl Event {
    pub fn payload_as<T: for<'de> Deserialize<'de>>(&self) -> CoreResult<T> {
        Ok(serde_json::from_value(self.payload.clone())?)
    }
}
