use serde::{Deserialize, Serialize};

use crate::errors::{CoreError, CoreResult};
use crate::settings::SessionSettingsOverride;

/// Lifecycle states a session can be in (PRD §10).
///
/// ```text
/// NEW -> ACTIVE -> {DETACHED, ARCHIVED}
/// DETACHED -> ACTIVE
/// ARCHIVED -> ACTIVE (via restore)
/// any -> DELETED
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    New,
    Active,
    Detached,
    Archived,
    Deleted,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::New => "NEW",
            SessionStatus::Active => "ACTIVE",
            SessionStatus::Detached => "DETACHED",
            SessionStatus::Archived => "ARCHIVED",
            SessionStatus::Deleted => "DELETED",
        }
    }

    pub fn parse(s: &str) -> CoreResult<Self> {
        Ok(match s {
            "NEW" => SessionStatus::New,
            "ACTIVE" => SessionStatus::Active,
            "DETACHED" => SessionStatus::Detached,
            "ARCHIVED" => SessionStatus::Archived,
            "DELETED" => SessionStatus::Deleted,
            other => return Err(CoreError::InvalidSessionStatus(other.to_string())),
        })
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Ownership lock describing which terminal currently "owns" (may write
/// into) a session (PRD §13).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOwner {
    pub pid: i32,
    pub host: String,
    pub terminal: String,
    pub heartbeat_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: SessionStatus,
    pub owner: Option<SessionOwner>,
    pub shell: Option<String>,
    pub cwd: Option<String>,
    pub settings: SessionSettingsOverride,
}

impl Session {
    pub fn is_archived(&self) -> bool {
        matches!(self.status, SessionStatus::Archived)
    }

    pub fn is_owned(&self) -> bool {
        self.owner.is_some()
    }
}

/// Validate a user-supplied session name (PRD §97): 1-100 characters,
/// no filesystem dependence since names live purely in SQLite.
pub fn validate_session_name(name: &str) -> CoreResult<()> {
    let len = name.chars().count();
    if len == 0 || len > 100 {
        return Err(CoreError::InvalidSessionName(name.to_string()));
    }
    Ok(())
}
