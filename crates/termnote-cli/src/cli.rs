use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "termnote",
    version,
    about = "A terminal session recorder, journal, and command timeline",
    long_about = "termnote turns a terminal session into a persistent, searchable \
                   engineering notebook. Run it with no arguments to open the session \
                   manager, or use a subcommand directly."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Create or add to a bookmark at the current position in the active
    /// session for this terminal (PRD §34-35). Equivalent to
    /// `termnote bookmark [LABEL]`.
    #[arg(short = 'b', long, num_args = 0..=1, default_missing_value = "")]
    pub bookmark: Option<String>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new session and start recording in it immediately.
    #[command(alias = "create")]
    New {
        name: String,
        #[command(flatten)]
        logging: LoggingArgs,
    },

    /// Attach to an existing session (creating a fresh shell if it was
    /// detached, PRD §12, §18).
    Attach {
        name: String,
        #[command(flatten)]
        logging: LoggingArgs,
    },

    /// Detach the active session in this terminal, ending its recording
    /// shell and transitioning the session to `DETACHED` (PRD §3, §17).
    /// Run as a command from inside a session you're attached to.
    Detach,

    /// List sessions.
    List {
        /// Include archived sessions.
        #[arg(long)]
        all: bool,
    },

    /// Open the read-only timeline browser for a session.
    Timeline { name: String },

    /// Insert a note at the current position in the active session running
    /// in this terminal (PRD §30).
    Note {
        /// Override the editor to use for this note only.
        #[arg(long)]
        editor: Option<String>,
    },

    /// Create a bookmark at the current position in the active session
    /// running in this terminal (PRD §34), or list/show existing ones.
    Bookmark {
        /// Optional label for a new bookmark.
        name: Option<String>,
        #[command(subcommand)]
        action: Option<BookmarkAction>,
    },

    /// Archive a session (PRD §55): no longer attachable, still searchable
    /// and exportable.
    Archive { name: String },

    /// Restore an archived session back to `DETACHED`.
    Restore { name: String },

    /// Permanently delete a session and all of its recorded history.
    Delete {
        name: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        force: bool,
    },

    /// Rename a session.
    Rename { old_name: String, new_name: String },

    /// Export a session to Markdown or CSV (PRD §52-54).
    Export {
        name: String,
        #[arg(short = 'f', long = "format", value_enum, default_value_t = ExportFormat::Markdown)]
        format: ExportFormat,
        /// Output file path. Defaults to `<session-name>.<ext>` in the
        /// current directory.
        #[arg(short = 'o', long)]
        output: Option<String>,
    },

    /// Full-text search across commands, output, notes, and bookmarks
    /// (PRD §42-43).
    Search {
        query: String,
        /// Restrict the search to one session.
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },

    /// View or change the global configuration (PRD §27, §80).
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },

    /// View or change a single session's settings overrides (PRD §28).
    Settings {
        name: String,
        #[command(subcommand)]
        action: SettingsAction,
    },
}

#[derive(Subcommand)]
pub enum BookmarkAction {
    /// List bookmarks in the active session for this terminal.
    List,
    /// Show the Nth bookmark (1-indexed, creation order).
    Show { index: usize },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Print the current effective global configuration.
    Show,
    /// Set a global configuration key, e.g. `logging.output false`.
    Set { key: String, value: String },
}

#[derive(Subcommand)]
pub enum SettingsAction {
    Show,
    Set { key: String, value: String },
}

#[derive(Copy, Clone, ValueEnum)]
pub enum ExportFormat {
    Markdown,
    Csv,
}

/// Per-invocation logging overrides (PRD §29: CLI beats session and global
/// settings). Only meaningful for `new`/`attach`, which are the only
/// commands that actually record anything.
#[derive(clap::Args, Default, Clone, Copy)]
pub struct LoggingArgs {
    #[arg(long)]
    pub commands: Option<bool>,
    #[arg(long)]
    pub output: Option<bool>,
    #[arg(long)]
    pub timestamps: Option<bool>,
    #[arg(long)]
    pub duration: Option<bool>,
    #[arg(long = "exit-codes")]
    pub exit_codes: Option<bool>,
    #[arg(long = "working-directory")]
    pub working_directory: Option<bool>,
    #[arg(long)]
    pub hostname: Option<bool>,
}

impl LoggingArgs {
    pub fn to_override(self) -> termnote_core::LoggingOverride {
        termnote_core::LoggingOverride {
            commands: self.commands,
            output: self.output,
            timestamps: self.timestamps,
            duration: self.duration,
            exit_codes: self.exit_codes,
            working_directory: self.working_directory,
            hostname: self.hostname,
            environment_metadata: None,
        }
    }
}
