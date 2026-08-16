use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use termnote_core::time::format_local;
use termnote_core::{resolve_logging, LoggingOverride, SessionOwner};
use termnote_session::{AttachOutcome, SelfIdentity, TakeoverChoice};
use termnote_storage::{bookmarks, events, notes, search as tn_search, sessions, SharedConn};

use crate::cli::{BookmarkAction, ConfigAction, ExportFormat, LoggingArgs, SettingsAction};
use crate::{config as cfgfile, paths};

/// Load the global config once per invocation.
fn load_global() -> Result<termnote_core::GlobalConfig> {
    cfgfile::load(&paths::config_path()?)
}

fn resolved_logging(global: &termnote_core::GlobalConfig, session: &termnote_core::SessionSettingsOverride, cli: LoggingOverride) -> termnote_core::LoggingSettings {
    resolve_logging(global.logging, &session.logging, &cli)
}

fn active_session_for_this_terminal(db: &SharedConn) -> Result<termnote_core::Session> {
    // Primary path: `termnote note`/`bookmark` run as a command inside a
    // termnote-managed shell inherit TERMNOTE_SESSION_ID from it (PRD
    // §103-104). This is exact and doesn't depend on tty device naming.
    if let Ok(id) = std::env::var("TERMNOTE_SESSION_ID") {
        if let Some(session) = sessions::get_by_id(db, &id)? {
            return Ok(session);
        }
    }
    // Fallback, mainly useful for tooling that talks to termnote without
    // going through its own managed shell: match on the calling terminal's
    // own tty against a session's ownership record.
    let tty = SelfIdentity::detect().terminal;
    sessions::get_by_active_terminal(db, &tty)?.ok_or_else(|| {
        anyhow::anyhow!(
            "no termnote session is active in this terminal.\n\
             Run `termnote attach <name>` (or `termnote new <name>`) first, then \
             `termnote note` / `termnote bookmark` from inside that session."
        )
    })
}

fn prompt_takeover(owner: &SessionOwner) -> TakeoverChoice {
    println!("Session is currently active elsewhere.\n");
    println!("Active owner:");
    println!("  Host:     {}", owner.host);
    println!("  PID:      {}", owner.pid);
    println!("  Terminal: {}", owner.terminal);
    println!("  Last heartbeat: {}\n", format_local(owner.heartbeat_at));
    println!("What would you like to do?");
    println!("  1. Continue here (ends the session in the other terminal)");
    println!("  2. Continue in the previous terminal (do nothing here)");
    println!("  3. Cancel");
    loop {
        print!("Choice [1/2/3]: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return TakeoverChoice::Cancel;
        }
        match line.trim() {
            "1" => return TakeoverChoice::ContinueHere,
            "2" => return TakeoverChoice::ContinueInPreviousTerminal,
            "3" | "" => return TakeoverChoice::Cancel,
            _ => continue,
        }
    }
}

fn on_recovered(owner: &SessionOwner) {
    println!(
        "Session \"{}\"'s previous owner (pid {}) appears to have terminated unexpectedly \
         (last heartbeat {}). Recovering session...\n",
        owner.host,
        owner.pid,
        format_local(owner.heartbeat_at)
    );
}

pub fn cmd_new(db: &SharedConn, name: &str, logging: LoggingArgs) -> Result<()> {
    let global = load_global()?;
    let effective = resolve_logging(global.logging, &Default::default(), &logging.to_override());
    match termnote_session::new_session(db, name, effective) {
        Ok(_reason) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("{e}")),
    }
}

pub fn cmd_attach(db: &SharedConn, name: &str, logging: LoggingArgs) -> Result<()> {
    let global = load_global()?;
    let session = sessions::require_by_name(db, name)?;
    let effective = resolved_logging(&global, &session.settings, logging.to_override());

    let outcome = termnote_session::attach_session(db, name, effective, prompt_takeover, on_recovered)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    match outcome {
        AttachOutcome::Ran(_) => Ok(()),
        AttachOutcome::DeclinedTakeover => {
            println!("Continuing in the previous terminal; nothing to do here.");
            Ok(())
        }
    }
}

pub fn cmd_list(db: &SharedConn, all: bool) -> Result<()> {
    let list = sessions::list(db, all)?;
    if list.is_empty() {
        println!("No sessions yet. Create one with `termnote new <name>`.");
        return Ok(());
    }
    println!("{:<30} {:<10} {:<20}", "NAME", "STATUS", "UPDATED");
    for s in list {
        println!("{:<30} {:<10} {:<20}", s.name, s.status, format_local(s.updated_at));
    }
    Ok(())
}

pub fn cmd_timeline(db: &SharedConn, name: &str) -> Result<()> {
    termnote_tui::run_timeline(db, name)
}

pub fn cmd_note(db: &SharedConn, editor_override: Option<&str>) -> Result<()> {
    let session = active_session_for_this_terminal(db)?;
    let global = load_global()?;
    let editor_cmd = termnote_editor::resolve_editor(
        editor_override,
        session.settings.editor.as_deref().or(global.editor.command.as_deref()),
    );
    match termnote_editor::edit_markdown(&editor_cmd, "")? {
        Some(markdown) => {
            notes::create_note(db, &session.id, &markdown)?;
            println!("Note saved.");
        }
        None => println!("No note saved."),
    }
    Ok(())
}

pub fn cmd_bookmark(db: &SharedConn, name: Option<String>, action: Option<BookmarkAction>) -> Result<()> {
    let session = active_session_for_this_terminal(db)?;
    match action {
        Some(BookmarkAction::List) => {
            let list = bookmarks::list_bookmarks(db, &session.id)?;
            if list.is_empty() {
                println!("No bookmarks yet.");
            }
            for (i, b) in list.iter().enumerate() {
                println!("  [{}] {}", i + 1, b.name.as_deref().unwrap_or("(unnamed)"));
            }
        }
        Some(BookmarkAction::Show { index }) => {
            let b = bookmarks::nth_bookmark(db, &session.id, index)?;
            let event = events::require(db, b.target_event_id)?;
            println!("Bookmark #{index}: {}", b.name.as_deref().unwrap_or("(unnamed)"));
            println!("{event:#?}");
        }
        None => {
            let target = events::latest_event_id(db, &session.id)?
                .ok_or_else(|| anyhow::anyhow!("nothing has happened in this session yet to bookmark"))?;
            let label = name.filter(|s| !s.is_empty());
            bookmarks::create_bookmark(db, &session.id, target, label.as_deref())?;
            println!("Bookmark created.");
        }
    }
    Ok(())
}

pub fn cmd_archive(db: &SharedConn, name: &str) -> Result<()> {
    sessions::archive(db, name)?;
    println!("Archived \"{name}\".");
    Ok(())
}

pub fn cmd_restore(db: &SharedConn, name: &str) -> Result<()> {
    sessions::restore(db, name)?;
    println!("Restored \"{name}\".");
    Ok(())
}

pub fn cmd_delete(db: &SharedConn, name: &str, force: bool) -> Result<()> {
    let preview = sessions::delete_preview(db, name)?;
    if !force {
        println!("Delete session \"{name}\"?\n");
        println!("This will permanently delete:");
        println!("  {} events", preview.events);
        println!("  {} notes", preview.notes);
        println!("  {} bookmarks", preview.bookmarks);
        println!("  ~{} bytes of output\n", preview.output_bytes);
        print!("Type the session name to confirm: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if line.trim() != name {
            println!("Names didn't match; not deleting.");
            return Ok(());
        }
    }
    sessions::delete(db, name)?;
    println!("Deleted \"{name}\".");
    Ok(())
}

pub fn cmd_rename(db: &SharedConn, old_name: &str, new_name: &str) -> Result<()> {
    sessions::rename(db, old_name, new_name)?;
    println!("Renamed \"{old_name}\" to \"{new_name}\".");
    Ok(())
}

pub fn cmd_export(db: &SharedConn, name: &str, format: ExportFormat, output: Option<String>) -> Result<()> {
    let global = load_global()?;
    let session = sessions::require_by_name(db, name)?;
    let display = resolved_logging(&global, &session.settings, LoggingOverride::default());

    let (contents, default_ext) = match format {
        ExportFormat::Markdown => (termnote_export::export_markdown(db, name, display)?, "md"),
        ExportFormat::Csv => (termnote_export::export_csv(db, name)?, "csv"),
    };

    let path = match output {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(format!("{name}.{default_ext}")),
    };
    std::fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
    println!("Exported \"{name}\" to {}", path.display());
    Ok(())
}

pub fn cmd_search(db: &SharedConn, query: &str, session: Option<&str>, limit: i64) -> Result<()> {
    let session_id = match session {
        Some(name) => Some(sessions::require_by_name(db, name)?.id),
        None => None,
    };
    let hits = tn_search::search(db, query, session_id.as_deref(), limit)?;
    if hits.is_empty() {
        println!("No results for {query:?}.");
        return Ok(());
    }
    for hit in hits {
        println!("[{}] {} — {}", hit.session_name, hit.event_type, hit.snippet);
    }
    Ok(())
}

pub fn cmd_config(action: Option<ConfigAction>) -> Result<()> {
    let path = paths::config_path()?;
    let mut cfg = cfgfile::load(&path)?;
    match action {
        None | Some(ConfigAction::Show) => {
            println!("# {}", path.display());
            println!("{}", toml::to_string_pretty(&cfg)?);
        }
        Some(ConfigAction::Set { key, value }) => {
            cfgfile::set_key(&mut cfg, &key, &value).map_err(|e| anyhow::anyhow!(e))?;
            cfgfile::save(&path, &cfg)?;
            println!("Set {key} = {value}");
        }
    }
    Ok(())
}

pub fn cmd_settings(db: &SharedConn, name: &str, action: SettingsAction) -> Result<()> {
    let session = sessions::require_by_name(db, name)?;
    match action {
        SettingsAction::Show => {
            println!("{}", toml::to_string_pretty(&session.settings)?);
        }
        SettingsAction::Set { key, value } => {
            let mut settings = session.settings;
            cfgfile::set_session_key(&mut settings, &key, &value).map_err(|e| anyhow::anyhow!(e))?;
            sessions::save_settings(db, &session.id, &settings)?;
            println!("Set {key} = {value} for session \"{name}\".");
        }
    }
    Ok(())
}

pub fn cmd_bare_bookmark(db: &SharedConn, label: String) -> Result<()> {
    cmd_bookmark(db, if label.is_empty() { None } else { Some(label) }, None)
}

pub fn cmd_session_manager(db: &SharedConn) -> Result<()> {
    // The session manager runs the TUI once, then hands off to `new`/`attach`
    // (which take over the terminal for the duration of the session). Each
    // match arm returns, so there is deliberately no loop here.
    match termnote_tui::run_session_manager(db)? {
        termnote_tui::SessionManagerAction::Quit => Ok(()),
        termnote_tui::SessionManagerAction::New(name) => {
            if let Err(e) = cmd_new(db, &name, LoggingArgs::default()) {
                eprintln!("Error: {e}");
            }
            Ok(())
        }
        termnote_tui::SessionManagerAction::Attach(name) => {
            if let Err(e) = cmd_attach(db, &name, LoggingArgs::default()) {
                eprintln!("Error: {e}");
            }
            Ok(())
        }
    }
}

// Small helper kept local to this module: `bail!` is imported for future
// handlers that want early, formatted returns without constructing anyhow
// errors by hand.
#[allow(dead_code)]
fn _unused(x: bool) -> Result<()> {
    if !x {
        bail!("unreachable");
    }
    Ok(())
}
