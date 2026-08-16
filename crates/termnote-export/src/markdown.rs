//! Markdown export (PRD §52): a standalone, human-readable rendering of a
//! session's timeline that reads sensibly without `termnote` installed.

use std::collections::HashMap;
use std::fmt::Write as _;

use termnote_core::{
    time::{format_duration_ns, format_local, format_local_time},
    BookmarkPayload, CommandPayload, Event, EventType, LoggingSettings, NotePayload,
    SessionLifecyclePayload,
};
use termnote_storage::{events, sessions, SharedConn};

use crate::ansi::strip_ansi;
use crate::error::ExportResult;

pub fn export_markdown(db: &SharedConn, session_name: &str, display: LoggingSettings) -> ExportResult<String> {
    let session = sessions::require_by_name(db, session_name)?;
    let all_events = events::list_all(db, &session.id)?;

    // Group OUTPUT events by the command they belong to, so each command's
    // output renders directly beneath it, matching PRD §52's example shape.
    let mut output_by_command: HashMap<i64, String> = HashMap::new();
    let mut orphan_output = String::new();
    for e in &all_events {
        if e.event_type != EventType::Output {
            continue;
        }
        let Ok(payload) = e.payload_as::<termnote_core::OutputPayload>() else { continue };
        let text = strip_ansi(&payload.text);
        match payload.command_event_id {
            Some(cmd_id) => output_by_command.entry(cmd_id).or_default().push_str(&text),
            None => orphan_output.push_str(&text),
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "# {}", session.name);
    let _ = writeln!(out);
    let _ = writeln!(out, "Created: {}", format_local(session.created_at));
    let _ = writeln!(out);
    let _ = writeln!(out, "---");

    if !orphan_output.trim().is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "```text");
        let _ = writeln!(out, "{}", orphan_output.trim_end());
        let _ = writeln!(out, "```");
    }

    for event in &all_events {
        render_event(&mut out, event, &output_by_command, display);
    }

    Ok(out)
}

fn render_event(
    out: &mut String,
    event: &Event,
    output_by_command: &HashMap<i64, String>,
    display: LoggingSettings,
) {
    let ts = event
        .timestamp_start
        .filter(|_| display.timestamps)
        .map(format_local_time);

    match event.event_type {
        EventType::Command => {
            let Ok(payload) = event.payload_as::<CommandPayload>() else { return };
            let _ = writeln!(out);
            match &ts {
                Some(t) => {
                    let _ = writeln!(out, "## {t} — Command");
                }
                None => {
                    let _ = writeln!(out, "## Command");
                }
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "```bash");
            let _ = writeln!(out, "{}", payload.command);
            let _ = writeln!(out, "```");

            if let Some(id) = event.id {
                if let Some(output) = output_by_command.get(&id) {
                    if !output.trim().is_empty() {
                        let _ = writeln!(out);
                        let _ = writeln!(out, "### Output");
                        let _ = writeln!(out);
                        let _ = writeln!(out, "```text");
                        let _ = writeln!(out, "{}", output.trim_end());
                        let _ = writeln!(out, "```");
                    }
                }
            }

            let _ = writeln!(out);
            if display.duration {
                if let Some(d) = event.duration_ns {
                    let _ = writeln!(out, "**Duration:** {}", format_duration_ns(d));
                }
            }
            if display.exit_codes {
                match payload.exit_code {
                    Some(code) => {
                        let _ = writeln!(out, "**Exit code:** {code}");
                    }
                    None => {
                        let _ = writeln!(out, "**Exit code:** unknown");
                    }
                }
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "---");
        }
        EventType::Note => {
            let Ok(payload) = event.payload_as::<NotePayload>() else { return };
            let _ = writeln!(out);
            match &ts {
                Some(t) => {
                    let _ = writeln!(out, "## {t} — Note");
                }
                None => {
                    let _ = writeln!(out, "## Note");
                }
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", payload.markdown.trim_end());
            let _ = writeln!(out);
            let _ = writeln!(out, "---");
        }
        EventType::Bookmark => {
            let Ok(payload) = event.payload_as::<BookmarkPayload>() else { return };
            let _ = writeln!(out);
            match &ts {
                Some(t) => {
                    let _ = writeln!(out, "## {t} — Bookmark");
                }
                None => {
                    let _ = writeln!(out, "## Bookmark");
                }
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "### {}", payload.name.as_deref().unwrap_or("(unnamed)"));
            let _ = writeln!(out);
            let _ = writeln!(out, "---");
        }
        EventType::SessionAttach => {
            let Ok(payload) = event.payload_as::<SessionLifecyclePayload>() else { return };
            let _ = writeln!(out);
            let host = payload.host.as_deref().unwrap_or("unknown host");
            match &ts {
                Some(t) => {
                    let _ = writeln!(out, "*(session attached at {t} on {host})*");
                }
                None => {
                    let _ = writeln!(out, "*(session attached on {host})*");
                }
            }
        }
        EventType::SessionDetach => {
            let _ = writeln!(out);
            match &ts {
                Some(t) => {
                    let _ = writeln!(out, "*(session detached at {t})*");
                }
                None => {
                    let _ = writeln!(out, "*(session detached)*");
                }
            }
        }
        // SESSION_START is implied by the document header; OUTPUT events
        // are rendered inline with their owning command above.
        EventType::SessionStart | EventType::Output | EventType::SessionEnd | EventType::SettingChange => {}
    }
}
