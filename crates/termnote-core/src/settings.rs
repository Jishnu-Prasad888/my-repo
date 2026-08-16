//! Configurable logging features and precedence resolution.
//!
//! Precedence (highest to lowest), per PRD §29:
//!   CLI argument > session setting > global setting > application default.

use serde::{Deserialize, Serialize};

/// Fully-resolved logging feature toggles that the recorder consults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingSettings {
    pub commands: bool,
    pub output: bool,
    pub timestamps: bool,
    pub duration: bool,
    pub exit_codes: bool,
    pub working_directory: bool,
    pub hostname: bool,
    pub environment_metadata: bool,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            commands: true,
            output: true,
            timestamps: true,
            duration: true,
            exit_codes: true,
            working_directory: true,
            hostname: false,
            environment_metadata: false,
        }
    }
}

/// A sparse override layer (session config or a one-off CLI flag). `None`
/// means "inherit from the layer below".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingOverride {
    pub commands: Option<bool>,
    pub output: Option<bool>,
    pub timestamps: Option<bool>,
    pub duration: Option<bool>,
    pub exit_codes: Option<bool>,
    pub working_directory: Option<bool>,
    pub hostname: Option<bool>,
    pub environment_metadata: Option<bool>,
}

impl LoggingOverride {
    pub fn is_empty(&self) -> bool {
        self.commands.is_none()
            && self.output.is_none()
            && self.timestamps.is_none()
            && self.duration.is_none()
            && self.exit_codes.is_none()
            && self.working_directory.is_none()
            && self.hostname.is_none()
            && self.environment_metadata.is_none()
    }
}

impl LoggingSettings {
    /// Apply a sparse override on top of `self`, returning the merged result.
    pub fn apply_override(mut self, o: &LoggingOverride) -> Self {
        macro_rules! ov {
            ($field:ident) => {
                if let Some(v) = o.$field {
                    self.$field = v;
                }
            };
        }
        ov!(commands);
        ov!(output);
        ov!(timestamps);
        ov!(duration);
        ov!(exit_codes);
        ov!(working_directory);
        ov!(hostname);
        ov!(environment_metadata);
        self
    }
}

/// Resolve the effective logging settings for one invocation, following the
/// precedence chain described in PRD §29.
pub fn resolve_logging(
    global: LoggingSettings,
    session: &LoggingOverride,
    cli: &LoggingOverride,
) -> LoggingSettings {
    global.apply_override(session).apply_override(cli)
}

/// Per-session overrides persisted alongside the session row (PRD §28).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSettingsOverride {
    pub logging: LoggingOverride,
    /// Session-level editor override, beats global but loses to `$VISUAL`
    /// only if explicitly unset -- see `termnote_editor` crate for the full
    /// precedence chain (PRD §31).
    pub editor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    pub command: Option<String>,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self { command: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub database: Option<String>,
    pub max_output_size_bytes: Option<u64>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            database: None,
            max_output_size_bytes: Some(1024 * 1024 * 1024), // 1 GiB, PRD §57
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "default".to_string(),
        }
    }
}

/// The full contents of `~/.config/termnote/config.toml` (PRD §27).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalConfig {
    pub logging: LoggingSettings,
    pub editor: EditorConfig,
    pub storage: StorageConfig,
    pub ui: UiConfig,
}
