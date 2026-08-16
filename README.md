# termnote

A terminal-native engineering notebook. `termnote` sits between your
terminal emulator and your shell, turning a session into a persistent,
searchable timeline of commands, output, notes, and bookmarks — one that
survives terminal closures, SSH drops, and reboots.

```text
Terminal Emulator
        │
        ▼
     termnote  ──── records ───▶  SQLite (WAL)
        │
        ▼
      shell (bash / zsh / fish / anything)
```

Full product spec: see the original PRD this implements. This README covers
what's built, how it works, and how to run it.

## Status

This is a working MVP covering the PRD's §106 MVP definition and the bulk of
its acceptance criteria (§110). It's real, compiling, tested Rust — not a
sketch. See [Implementation notes & known limitations](#implementation-notes--known-limitations)
for the honest list of what's simplified versus a hypothetical v2.

## Quick start

```bash
cargo build --release
./target/release/termnote new my-first-session
# ... use the shell normally ...
# in another terminal:
./target/release/termnote list
./target/release/termnote attach my-first-session
```

Bare `termnote` (no arguments) opens a session-picker TUI.

## Building

Requires a Rust toolchain and, on Linux, `libsqlite3` (dynamically linked).

```bash
cargo build --release
```

The binary is a single file: `target/release/termnote`. No runtime
dependencies beyond `libsqlite3` and whatever shell you point it at.

> **Note on this sandbox's toolchain:** this environment ships Rust 1.75 via
> apt with no network access to crates.io's *latest* releases (many of which
> now require a newer compiler). The workspace is built against Ubuntu's
> `librust-*-dev` packages as a local vendored registry — see
> `.cargo/config.toml`. On a normal machine with an up-to-date Rust
> toolchain and full crates.io access, you can delete `.cargo/config.toml`
> and it'll resolve fresh versions from crates.io instead; the code doesn't
> depend on anything Debian-specific.

## Testing

```bash
cargo test --workspace          # 24 unit tests across all crates
python3 scripts/e2e_smoke_test.py       # full PTY integration test
python3 scripts/e2e_reattach_test.py    # detach/reattach + note/bookmark test
```

The two Python scripts are true end-to-end tests: they fork a real PTY (via
`pty.fork()`, the same mechanism a terminal emulator uses), drive the actual
`termnote` binary through it by writing raw bytes to the master side exactly
like a user typing, and then assert on what landed in the SQLite database.
They're what caught two real bugs during development (a stale `is_resume`
check, and a tty-vs-tty mismatch for `termnote note`/`bookmark` — see git
history / comments for the fixes), which is exactly why they're checked in
rather than thrown away.

## Command reference

```text
termnote                          open the session picker
termnote new <name>               create a session and start recording
termnote attach <name>            attach (or reattach) to a session
termnote list [--all]             list sessions (--all includes archived)
termnote timeline <name>          read-only browser over a session's history
termnote note [--editor CMD]      open $EDITOR, save as a NOTE at the current position
termnote bookmark [label]         bookmark the current position
termnote bookmark list            list bookmarks in the active session
termnote bookmark show <n>        show the Nth bookmark
termnote -b [label]               shorthand for `termnote bookmark [label]`
termnote archive <name>           archive (no longer attachable, still searchable)
termnote restore <name>           un-archive
termnote delete <name> [--force]  permanently delete (asks you to type the name back)
termnote rename <old> <new>
termnote export <name> -f markdown|csv [-o path]
termnote search <query> [--session name]
termnote config show|set <key> <value>       global settings, e.g. logging.output
termnote settings <name> show|set <key> <val> per-session override
```

`termnote note` and `termnote bookmark` are meant to be run **as commands
inside a session you're already attached to** — that's how they know which
session to write to (via a `TERMNOTE_SESSION_ID` environment variable set on
the shell termnote spawns for you), matching the PRD's "at any point, just
run `termnote note`" workflow (§103-104).

## Architecture

```text
termnote/
├── crates/
│   ├── termnote-core      domain types only: events, sessions, settings.
│   │                      No IO, no SQL, no PTY code. Everything else
│   │                      depends on this; it depends on nothing internal.
│   ├── termnote-storage   SQLite (WAL + FTS5) schema, migrations, and every
│   │                      SQL statement in the app. Owns the single-
│   │                      terminal-ownership compare-and-swap lock.
│   ├── termnote-pty       Direct PTY layer (openpty/fork/exec/resize/
│   │                      foreground-pgrp) built on `nix`, not a PTY crate
│   │                      — see that crate's module docs for why.
│   ├── termnote-session   The recorder: ties PTY + storage together into
│   │                      "run a shell, build an event timeline." Owns
│   │                      command-boundary detection, ownership/heartbeat,
│   │                      and the optional shell-integration hook.
│   ├── termnote-editor    Launches $EDITOR/$VISUAL for notes.
│   ├── termnote-export    Markdown and CSV rendering of a session.
│   ├── termnote-tui       ratatui session picker + read-only timeline view.
│   └── termnote-cli       clap CLI; the `termnote` binary lives here.
├── scripts/               end-to-end PTY integration tests (see above).
└── migrations/            (embedded in termnote-storage via include_str!)
```

### How command boundaries are detected without shell hooks

This is the trickiest part of the PRD (§8), and worth explaining because
it's not obvious from the code alone.

**The core mechanism (works with any shell, zero configuration):**
`termnote` spawns the shell as its own session leader
(`setsid()` + `TIOCSCTTY`), so its process-group id equals its own pid. A
dedicated thread polls `tcgetpgrp()` on the PTY master every ~60ms — this is
the kernel's own bookkeeping of "which process group currently owns the
foreground of this terminal," the same mechanism every job-control-aware
shell relies on. When it changes away from the shell's pgid, a command
started; when it changes back, the command ended. Combined with a shadow
copy of stdin (only captured while the shell is in the foreground — so
keystrokes going to `vim` or `ssh` are never mistaken for a new command),
this gives accurate start/end timestamps, duration, and (via `/proc/<pid>/cwd`)
working directory, for *any* shell, with no dotfiles touched.

What this can't give you: an **exit code**. The kernel has no concept of
"exit status" for a process termnote didn't fork itself; only the shell that
actually `wait()`ed on the job knows that, and shell builtins (`cd`,
`export`, ...) never change the foreground pgrp at all, so the pgrp signal
alone can't even see them.

**The optional enhancement (bash/zsh/fish, still zero configuration from the
user's point of view):** when launching one of those three shells, termnote
transparently adds a tiny `precmd`/`PROMPT_COMMAND`/`fish_postexec` hook —
generated on the fly, loaded via `--rcfile`/`$ZDOTDIR`/`--init-command`, so
your actual `.bashrc`/`.zshrc`/`config.fish` still loads normally, unedited.
The hook reports `$?`/`$status` and `$PWD` over a FIFO after every prompt.
This is what makes builtins show up as instantaneous commands with exact
exit codes, and gives external commands exact codes too, instead of just
"we know it ran and how long it took." Unsupported/unrecognized shells still
get full command-boundary detection via the pgrp path; they just show
`exit_code: null`.

This two-layer design is a direct reading of PRD §8's "primary mechanism +
optional enhancement" split — the difference from a literal reading is that
the "optional" layer requires no user action (no `termnote shell install`
step, no edited dotfiles) since it's just an implementation detail of how
termnote launches the shell process for that session.

### Single-terminal ownership

Implemented as a real compare-and-swap `UPDATE ... WHERE active_pid IS NULL
OR heartbeat_at < ?stale_threshold` (see `termnote-storage::sessions`), so
it's correct even when raced by a second `termnote` process on the same
database — not just "check then act" from the CLI layer. "Continue here"
sends `SIGTERM` to the previous owner if it's on the same host, and every
running instance also watches its own ownership row in the background and
exits gracefully the moment it notices someone else took over, whether or
not the signal was delivered (e.g., the previous owner was on a different
host).

## Implementation notes & known limitations

Being upfront about what's simplified in this pass, matching the PRD's own
"MVP now, V1.1/V2 later" phasing (§107-108):

- **Command-text capture** is a shadow copy of raw input bytes (with
  backspace handling), not a readline/PTY-output parse. Using shell history
  expansion or arrow-key history recall to run a command will still be
  detected as *a* command (via the pgrp signal) but its captured text may be
  empty or stale. Documented rather than hidden.
- **Output storage** is one event per PTY read chunk (no batching/
  coalescing), and there's no enforcement of `storage.max_output_size` yet
  (§57 lists this as a "future option," not required for MVP).
- **Timeline UI** is read-only browsing; live-attaching happens through the
  normal raw-PTY code path (same one `termnote attach` uses on the command
  line), not inside ratatui — you can't render a live PTY passthrough and a
  TUI widget tree in the same terminal at once, so the session picker hands
  off to the CLI's attach flow rather than trying to.
- **Session-level settings** (§28) are fully wired into the data model and
  the recorder (`resolve_logging` correctly layers CLI > session > global >
  default), with a `termnote settings <name> set ...` CLI command; there's
  no interactive TUI settings *screen* (§79) yet.
- **Secret redaction** (§59) is explicitly out of scope for v1, per the
  PRD's own instruction not to attempt aggressive redaction.
- **Export flag names** were standardized to `--format/-f` and `--output/-o`
  with a positional session name; the PRD's §8 illustrative example used a
  different, inconsistent scheme (`-s`/`-o`/`-n`) that doesn't match its own
  §52-53/§105 examples. This README documents the deviation rather than
  silently picking one.
