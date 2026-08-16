//! Launch the user's editor to compose a note (PRD §30-31).
//!
//! Precedence (highest to lowest, PRD §31): CLI override > session editor >
//! `$VISUAL` > `$EDITOR` > `vim`.

use std::io::{Read, Write};
use std::process::Command;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EditorError {
    #[error("failed to create temporary file: {0}")]
    TempFile(#[source] std::io::Error),
    #[error("failed to launch editor {0:?}: {1}")]
    Launch(String, #[source] std::io::Error),
    #[error("editor {0:?} exited with a non-zero status")]
    NonZeroExit(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type EditorResult<T> = Result<T, EditorError>;

/// Resolve which editor command to run, following the precedence chain.
pub fn resolve_editor(cli: Option<&str>, session: Option<&str>) -> String {
    cli.map(str::to_string)
        .or_else(|| session.map(str::to_string))
        .or_else(|| std::env::var("VISUAL").ok().filter(|s| !s.trim().is_empty()))
        .or_else(|| std::env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "vim".to_string())
}

/// Open `editor_cmd` on a scratch file, wait for it to exit, and return the
/// file's contents. Returns `Ok(None)` if the user left the document empty
/// (PRD §33) -- in that case no note event should be recorded.
///
/// The editor command is split on whitespace so configurations like
/// `"code --wait"` work; the scratch file path is appended as the final
/// argument.
pub fn edit_markdown(editor_cmd: &str, initial: &str) -> EditorResult<Option<String>> {
    let mut file = tempfile::Builder::new()
        .prefix("termnote-note-")
        .suffix(".md")
        .tempfile()
        .map_err(EditorError::TempFile)?;
    file.write_all(initial.as_bytes())?;
    file.flush()?;
    let path = file.path().to_path_buf();

    let mut parts = editor_cmd.split_whitespace();
    let program = parts
        .next()
        .map(str::to_string)
        .unwrap_or_else(|| "vim".to_string());
    let extra_args: Vec<String> = parts.map(str::to_string).collect();

    let status = Command::new(&program)
        .args(&extra_args)
        .arg(&path)
        .status()
        .map_err(|e| EditorError::Launch(editor_cmd.to_string(), e))?;

    if !status.success() {
        return Err(EditorError::NonZeroExit(editor_cmd.to_string()));
    }

    let mut contents = String::new();
    std::fs::File::open(&path)?.read_to_string(&mut contents)?;
    // `file` (the NamedTempFile) is dropped here, deleting the scratch file
    // (PRD §30: "the temporary file is deleted after successful persistence").

    if contents.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(contents))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_order() {
        assert_eq!(resolve_editor(Some("nvim"), Some("emacs")), "nvim");
        assert_eq!(resolve_editor(None, Some("emacs")), "emacs");
    }

    #[test]
    fn edit_with_a_noninteractive_stand_in_editor() {
        use std::os::unix::fs::PermissionsExt;
        // `edit_markdown` splits the editor command on plain whitespace (no
        // shell quoting), so exercise it with a tiny script file rather than
        // an inline shell one-liner containing quotes.
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("editor.sh");
        std::fs::write(&script_path, "#!/bin/sh\necho replaced-content > \"$1\"\n").unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        let result = edit_markdown(script_path.to_str().unwrap(), "original").unwrap();
        assert_eq!(result.as_deref(), Some("replaced-content\n"));
    }

    #[test]
    fn empty_result_yields_none() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("editor.sh");
        std::fs::write(&script_path, "#!/bin/sh\ntrue\n").unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        let result = edit_markdown(script_path.to_str().unwrap(), "").unwrap();
        assert_eq!(result, None);
    }
}
