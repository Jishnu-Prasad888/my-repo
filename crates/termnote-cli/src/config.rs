//! Load/save the global TOML config file (PRD §27, §80).

use std::path::Path;

use termnote_core::GlobalConfig;

pub fn load(path: &Path) -> anyhow::Result<GlobalConfig> {
    if !path.exists() {
        return Ok(GlobalConfig::default());
    }
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text).unwrap_or_else(|e| {
        eprintln!("warning: failed to parse {} ({e}); using defaults", path.display());
        GlobalConfig::default()
    }))
}

pub fn save(path: &Path, cfg: &GlobalConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(cfg)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Apply `key=value` (e.g. `logging.output=false`, `editor.command=nvim`)
/// to a config in place. Returns an error message on an unknown key or a
/// value that doesn't parse for that key's type.
pub fn set_key(cfg: &mut GlobalConfig, key: &str, value: &str) -> Result<(), String> {
    let parse_bool = |v: &str| -> Result<bool, String> {
        v.parse::<bool>().map_err(|_| format!("expected true/false, got {v:?}"))
    };
    match key {
        "logging.commands" => cfg.logging.commands = parse_bool(value)?,
        "logging.output" => cfg.logging.output = parse_bool(value)?,
        "logging.timestamps" => cfg.logging.timestamps = parse_bool(value)?,
        "logging.duration" => cfg.logging.duration = parse_bool(value)?,
        "logging.exit_codes" => cfg.logging.exit_codes = parse_bool(value)?,
        "logging.working_directory" => cfg.logging.working_directory = parse_bool(value)?,
        "logging.hostname" => cfg.logging.hostname = parse_bool(value)?,
        "logging.environment_metadata" => cfg.logging.environment_metadata = parse_bool(value)?,
        "editor.command" => cfg.editor.command = Some(value.to_string()),
        "ui.theme" => cfg.ui.theme = value.to_string(),
        other => {
            return Err(format!(
                "unknown config key {other:?}. Valid keys: logging.commands, logging.output, \
                 logging.timestamps, logging.duration, logging.exit_codes, \
                 logging.working_directory, logging.hostname, logging.environment_metadata, \
                 editor.command, ui.theme"
            ))
        }
    }
    Ok(())
}

/// Same as [`set_key`] but for a session's sparse override layer.
pub fn set_session_key(settings: &mut termnote_core::SessionSettingsOverride, key: &str, value: &str) -> Result<(), String> {
    let parse_bool = |v: &str| -> Result<bool, String> {
        v.parse::<bool>().map_err(|_| format!("expected true/false, got {v:?}"))
    };
    match key {
        "logging.commands" => settings.logging.commands = Some(parse_bool(value)?),
        "logging.output" => settings.logging.output = Some(parse_bool(value)?),
        "logging.timestamps" => settings.logging.timestamps = Some(parse_bool(value)?),
        "logging.duration" => settings.logging.duration = Some(parse_bool(value)?),
        "logging.exit_codes" => settings.logging.exit_codes = Some(parse_bool(value)?),
        "logging.working_directory" => settings.logging.working_directory = Some(parse_bool(value)?),
        "logging.hostname" => settings.logging.hostname = Some(parse_bool(value)?),
        "logging.environment_metadata" => settings.logging.environment_metadata = Some(parse_bool(value)?),
        "editor" => settings.editor = Some(value.to_string()),
        other => return Err(format!("unknown session setting key {other:?}")),
    }
    Ok(())
}
