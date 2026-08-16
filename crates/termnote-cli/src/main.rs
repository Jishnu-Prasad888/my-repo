mod cli;
mod commands;
mod config;
mod paths;

use clap::Parser;

use cli::{Cli, Command};

fn main() {
    init_tracing();

    if let Err(e) = run() {
        eprintln!("termnote: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let db_path = paths::default_db_path()?;
    let db = termnote_storage::open(&db_path)?;

    match cli.command {
        Some(Command::New { name, logging }) => commands::cmd_new(&db, &name, logging),
        Some(Command::Attach { name, logging }) => commands::cmd_attach(&db, &name, logging),
        Some(Command::Detach) => commands::cmd_detach(&db),
        Some(Command::List { all }) => commands::cmd_list(&db, all),
        Some(Command::Timeline { name }) => commands::cmd_timeline(&db, &name),
        Some(Command::Note { editor }) => commands::cmd_note(&db, editor.as_deref()),
        Some(Command::Bookmark { name, action }) => commands::cmd_bookmark(&db, name, action),
        Some(Command::Archive { name }) => commands::cmd_archive(&db, &name),
        Some(Command::Restore { name }) => commands::cmd_restore(&db, &name),
        Some(Command::Delete { name, force }) => commands::cmd_delete(&db, &name, force),
        Some(Command::Rename { old_name, new_name }) => commands::cmd_rename(&db, &old_name, &new_name),
        Some(Command::Export { name, format, output }) => commands::cmd_export(&db, &name, format, output),
        Some(Command::Search { query, session, limit }) => {
            commands::cmd_search(&db, &query, session.as_deref(), limit)
        }
        Some(Command::Config { action }) => commands::cmd_config(action),
        Some(Command::Settings { name, action }) => commands::cmd_settings(&db, &name, action),
        None => match cli.bookmark {
            Some(label) => commands::cmd_bare_bookmark(&db, label),
            None => commands::cmd_session_manager(&db),
        },
    }
}

/// A tiny, dependency-free stand-in for `tracing-subscriber`'s `env-filter`
/// feature, which isn't available in this build (see Cargo.toml comment).
/// Reads `RUST_LOG` as a single global level; good enough for termnote's
/// own diagnostic logging, which never includes recorded terminal content
/// (PRD §93).
fn init_tracing() {
    use tracing::level_filters::LevelFilter;

    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| match s.to_lowercase().as_str() {
            "trace" => Some(LevelFilter::TRACE),
            "debug" => Some(LevelFilter::DEBUG),
            "info" => Some(LevelFilter::INFO),
            "warn" | "warning" => Some(LevelFilter::WARN),
            "error" => Some(LevelFilter::ERROR),
            "off" | "none" => Some(LevelFilter::OFF),
            _ => None,
        })
        .unwrap_or(LevelFilter::WARN);

    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .try_init();
}
