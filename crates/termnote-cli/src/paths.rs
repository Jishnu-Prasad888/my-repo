//! Filesystem locations (PRD §27, §44): `~/.config/termnote/config.toml`
//! and `~/.local/share/termnote/termnote.db` by default, both overridable.

use std::path::PathBuf;

use directories::ProjectDirs;

fn project_dirs() -> anyhow::Result<ProjectDirs> {
    ProjectDirs::from("", "", "termnote")
        .ok_or_else(|| anyhow::anyhow!("could not determine a home directory for this user"))
}

pub fn config_path() -> anyhow::Result<PathBuf> {
    if let Ok(p) = std::env::var("TERMNOTE_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    Ok(project_dirs()?.config_dir().join("config.toml"))
}

pub fn default_db_path() -> anyhow::Result<PathBuf> {
    Ok(project_dirs()?.data_dir().join("termnote.db"))
}
