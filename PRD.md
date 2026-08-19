# PRD — `termnote`

**Product:** `termnote`
**Type:** Terminal session recorder, journal, and command timeline
**Implementation:** Rust + Ratatui
**Primary interface:** Terminal UI (TUI) + CLI
**Storage:** SQLite with WAL
**Target:** Linux, distro-agnostic; shell-agnostic; terminal-emulator-agnostic
**License:** MIT

# 1. Product Overview

`termnote` is a terminal-native application that turns a terminal session into a persistent, searchable engineering notebook.

It records:

* Commands executed
* Command output
* Command start time
* Command end time
* Command duration
* Exit code
* Working directory
* Environment/session metadata
* User-created notes
* Bookmarks
* Session lifecycle events

The user can organize terminal activity into **sessions**.

A session can remain open across terminal closures, SSH reconnects, shell restarts, or system restarts.

The user can later reopen the session and continue exactly where they left off.

The application must not depend on:

* Bash
* Zsh
* Fish
* Any particular shell
* GNOME Terminal
* Konsole
* Kitty
* Alacritty
* WezTerm
* Any particular terminal emulator

The application should operate at the **PTY level**.

# 2. Problem

Normal terminal history is insufficient for serious engineering work.

A normal shell history might tell you:

```text
kubectl get pods -A
helm upgrade ...
kubectl logs ...
```

but it does not reliably preserve:

* What the command output looked like
* When the command was executed
* How long it took
* Whether it succeeded
* What the user was trying to accomplish
* Why the user executed the command
* Where in a debugging process the command occurred
* Which commands were related to the same task
* Notes made during the investigation

Terminal recording tools such as `script` and asciinema solve parts of the problem, but primarily treat the terminal as a recording.

`termnote` instead treats the terminal as a **structured timeline**.

# 3. Product Vision

The core mental model is:

> **A terminal session is an append-only engineering notebook.**

Example:

```text
SESSION: K3s OpenChoreo Debugging

09:31:02  COMMAND
kubectl get pods -A

09:31:03  OUTPUT
...

09:32:18  COMMAND
kubectl logs -n openchoreo ...

09:32:19  OUTPUT
...

09:33:40  BOOKMARK
Investigating OpenBao failure

09:34:01  COMMAND
kubectl describe pod ...

09:35:10  NOTE

The pod is healthy but the SecretStore
is failing to authenticate.

09:37:51  COMMAND
kubectl get secretstore -A

...
```

The entire thing becomes a persistent artifact that can later be searched, exported, or reviewed.

---

# 4. Goals

## 4.1 Primary goals

### G1 — Record terminal activity

Capture every command executed through the `termnote` terminal.

Capture:

* Command
* Output
* Start timestamp
* End timestamp
* Duration
* Exit status
* Working directory

---

### G2 — Persistent sessions

Users can create:

```bash
termnote session create k3s-debug
```

and continue using the same session later.

---

### G3 — Session lifecycle management

Sessions must support:

* Create
* Open
* Attach
* Detach
* Continue
* Archive
* Restore
* Delete
* Rename
* List
* Search

---

### G4 — Cross-terminal continuation

A session can only actively run in **one terminal at a time**.

If the user attempts:

```bash
termnote attach k3s-debug
```

while it is already active elsewhere:

```text
Session "k3s-debug" is currently active.

Active terminal:
    PID: 3812
    Host: workstation
    Started: 03:12:44
    Last activity: 03:28:11

What would you like to do?

> Continue here
  Continue in previous terminal
  Cancel
```

---

### G5 — Markdown notes

Users can insert notes into the timeline.

Running:

```bash
termnote note
```

must open the user's configured editor, normally Vim.

User writes:

```markdown
# OpenBao Investigation

The SecretStore appears healthy.

The failure seems to originate from the authentication
configuration rather than OpenBao itself.

## Next

- Inspect ClusterSecretStore
- Check service account
```

User executes:

```vim
:wq
```

The note is persisted at the exact point in the session timeline where it was created.

---

### G6 — Bookmarks

The user can mark the current position:

```bash
termnote -b
```

or:

```bash
termnote bookmark
```

A bookmark acts as a pointer into the session timeline.

Example:

```text
BOOKMARK
──────────────
OpenChoreo installation failure

Position:
Command #143

kubectl logs -n openchoreo ...
```

Bookmarks should optionally support labels.

Example :

```text
termnote -b "Starting OOM error debug"
```

---

### G7 — Configurable logging

Logging features can be enabled or disabled:

* Globally
* Per session

Examples:

```text
Record command:       ON
Record output:        ON
Record timestamps:    ON
Record duration:      ON
Record exit code:     ON
Record cwd:           ON
```

Session-level settings override global settings.

---

### G8 — Export

Entire sessions must be exportable to:

* Markdown
* CSV

```text
termnnote export -s "Pod Crash Debug" -o csv -n "K3s Debug session 2-3-4"
```

the above command should export the session Pod Crash Debug in csv format into the file `K3s Debug Session 2-3-4.csv`

---

### G9 — Crash resilience

The application must minimize data loss.

SQLite WAL should be used where practical.

Commands and events should be persisted incrementally rather than waiting for the terminal session to close.

---

# 5. Non-Goals

Version 1 does **not** attempt to:

* Become a full terminal emulator
* Replace Bash/Zsh/Fish
* Implement shell parsing itself
* Provide remote command execution
* Provide cloud synchronization
* Provide multi-user collaboration
* Capture arbitrary terminals outside the PTY managed by `termnote`
* Modify shell configuration files automatically
* Require a particular shell

---

# 6. Core Architecture

The application consists of five major components.

```text
                   ┌─────────────────────┐
                   │      termnote       │
                   │                     │
                   │      CLI / TUI      │
                   └──────────┬──────────┘
                              │
                    ┌─────────▼─────────┐
                    │   Session Manager │
                    └─────────┬─────────┘
                              │
              ┌───────────────┼───────────────┐
              │               │               │
      ┌───────▼──────┐ ┌──────▼───────┐ ┌─────▼──────┐
      │ PTY Manager  │ │ Event Engine │ │   SQLite   │
      │              │ │              │ │    WAL     │
      └───────┬──────┘ └──────────────┘ └────────────┘
              │
       ┌──────▼───────┐
       │ Shell Process│
       │ bash/zsh/... │
       └──────────────┘
```

---

# 7. PTY Architecture

This is one of the most important requirements.

`termnote` must not depend on shell-specific hooks such as:

```bash
PROMPT_COMMAND
precmd
preexec
```

because that would violate shell agnosticism.

Instead:

```text
Terminal emulator
       │
       │ stdin/stdout
       ▼
    termnote
       │
       │ PTY
       ▼
    shell
       │
       ▼
 commands
```

`termnote` creates a pseudo-terminal.

The shell runs inside that PTY.

The application observes the PTY's input/output stream.

This allows:

```text
termnote
   │
   ├── bash
   ├── zsh
   ├── fish
   ├── sh
   └── other shells
```

without shell-specific integrations.

---

# 8. Important Technical Constraint — Command Detection

There is a fundamental limitation.

At the PTY level, the application can reliably see:

```text
terminal input
terminal output
```

but not necessarily a semantic boundary saying:

> "The user has finished entering command X."

Therefore the implementation should use a layered strategy.

### Primary mechanism

Track terminal input and output and detect command execution boundaries using PTY interaction.

### Optional shell integration

Shell-specific integrations may later be provided as an **optional enhancement**, but the core application must function without them.

For example:

```text
termnote
 ├── core PTY recording
 └── optional shell integrations
      ├── bash
      ├── zsh
      ├── fish
      └── ...
```

The product must never require these integrations.

---

# 9. Session Model

A session is the primary organizational unit.

Example:

```text
Sessions
│
├── k3s-debug
├── openchoreo
├── linux-learning
├── networking
└── rpi-cluster
```

Each session has:

```text
ID
Name
Created At
Updated At
Status
Host
Active PID
Archived
Settings
```

---

# 10. Session States

A session can have the following states:

```text
NEW
ACTIVE
DETACHED
ARCHIVED
DELETED
```

Conceptually:

```text
             ┌────────────┐
             │    NEW     │
             └─────┬──────┘
                   │
                   ▼
             ┌────────────┐
             │   ACTIVE   │
             └─────┬──────┘
                   │
             ┌─────┴──────┐
             │            │
             ▼            ▼
        DETACHED       ARCHIVED
             │            │
             │            │
             ▼            │
          ACTIVE ◄─────────┘
```

---

# 11. Session Creation

CLI:

```bash
termnote session create k3s-debug
```

or:

```bash
termnote new k3s-debug
```

The application should then start the session.

---

# 12. Session Attachment

Attach:

```bash
termnote attach k3s-debug
```

If detached:

```text
Attaching session:

k3s-debug

Last activity:
2026-08-12 03:18:31

Last command:
kubectl get pods -A

Continue?
[Y/n]
```

---

# 13. Single Terminal Ownership

A session must have an ownership lock.

Example database state:

```text
session_id
active_pid
active_host
active_terminal_id
active_since
heartbeat
```

The active terminal periodically updates a heartbeat.

Example:

```text
session: k3s-debug
owner:
    host = workstation
    pid = 3911
    terminal = ttypts/4
    heartbeat = 03:36:02
```

---

# 14. Starting an Already Active Session

Suppose Terminal A owns:

```text
k3s-debug
```

Terminal B executes:

```bash
termnote attach k3s-debug
```

Display:

```text
Session is currently active.

k3s-debug

Active owner:
  Host: workstation
  PID: 3911
  Terminal: /dev/pts/4
  Since: 03:12:04

Options:

  1. Continue here
  2. Continue in previous terminal
  3. Cancel
```

---

# 15. "Continue Here"

This transfers ownership.

Process:

```text
Terminal B
   │
   ├── request takeover
   │
   ▼
Database
   │
   ├── mark Terminal A as revoked
   │
   ▼
Terminal A
   │
   └── receives termination signal
```

Terminal A should gracefully terminate its `termnote` instance.

The session then becomes owned by Terminal B.

The user's shell should be terminated along with the old PTY.

This prevents two terminals from simultaneously writing to the same live shell.

---

# 16. "Continue in Previous Terminal"

The new invocation exits.

The original terminal continues owning the session.

---

# 17. Terminal Closure

If the user closes their terminal unexpectedly:

```text
terminal
   ↓
termnote
   ↓
shell
```

the application must detect the PTY/terminal closure.

The session becomes:

```text
DETACHED
```

The recorded data remains intact.

Later:

```bash
termnote attach k3s-debug
```

can resume it.

---

# 18. Shell Continuation

A major requirement is that reopening a session should not merely display history.

It should actually restore the shell environment as far as reasonably possible.

Example:

```text
Terminal closed

          ↓

termnote attach k3s-debug

          ↓

shell resumes
```

The implementation should preserve at minimum:

* Working directory
* Session environment variables where practical
* Shell type
* Terminal dimensions
* Session metadata

Exact restoration of arbitrary shell state is inherently shell-dependent and should not be promised.

---

# 19. Events

Everything inside a session is an event.

Core event types:

```text
COMMAND
OUTPUT
NOTE
BOOKMARK
SESSION_START
SESSION_ATTACH
SESSION_DETACH
SESSION_END
SETTING_CHANGE
```

Potential future events:

```text
FILE_CHANGE
DIRECTORY_CHANGE
ERROR
MARKER
ANNOTATION
```

---

# 20. Event Timeline

Example:

```text
03:12:01 SESSION_START

03:12:04 COMMAND
kubectl get pods -A

03:12:05 OUTPUT

03:12:05 COMMAND_END
exit=0
duration=823ms

03:13:21 COMMAND
kubectl logs ...

03:13:22 COMMAND_END
exit=1
duration=1.2s

03:14:01 BOOKMARK
postgres-debug

03:14:30 NOTE
Markdown document...

03:16:40 COMMAND
kubectl restart ...

03:17:01 SESSION_DETACH
```

---

# 21. Command Metadata

Every command record should support:

```text
id
session_id
timestamp_start
timestamp_end
duration_ms
command
exit_code
working_directory
terminal_size
hostname
shell
```

Optional:

```text
user
environment_hash
git_branch
git_commit
```

The latter should be configurable.

---

# 22. Output Recording

Output should be associated with commands.

Example:

```text
COMMAND
kubectl get pods

OUTPUT
NAME          READY   STATUS
postgres-0    1/1     Running
api-7df       1/1     Running
```

The output must preserve terminal control sequences where necessary.

This matters because commands may output:

* ANSI colors
* Cursor movement
* Progress bars
* Interactive UI output
* Unicode
* stderr

---

# 23. stdout vs stderr

Where possible, preserve:

```text
stdout
stderr
```

However, because PTY semantics merge streams in many cases, the system should not promise perfect separation when running through a PTY.

The raw PTY stream should remain authoritative.

---

# 24. Timing

Each command should support:

```text
start_time
end_time
duration
```

Example:

```text
03:41:22.142
kubectl get pods

Duration: 431ms
Exit: 0
```

Timing precision:

**Recommended:** nanosecond-capable monotonic clock internally.

Persist:

```text
duration_ns
```

and display:

```text
431 ms
```

---

# 25. Timestamp Configuration

Users should be able to disable timestamp logging.

Global:

```toml
[logging]
timestamps = true
```

Session:

```toml
[session.logging]
timestamps = false
```

If disabled, the raw command event should not contain user-visible timestamp information, while internal database metadata required for integrity may still exist.

---

# 26. Configurable Features

Every logging feature should be independently configurable.

Example:

```text
Command logging       ON
Output logging        ON
Timestamps            ON
Duration              ON
Exit codes             ON
Working directory     ON
Hostname              OFF
Environment metadata  OFF
```

---

# 27. Global Configuration

Example:

```text
~/.config/termnote/config.toml
```

Example:

```toml
[logging]
commands = true
output = true
timestamps = true
duration = true
exit_codes = true
working_directory = true
hostname = false

[editor]
command = "vim"

[storage]
database = "~/.local/share/termnote/termnote.db"

[ui]
theme = "default"
```

---

# 28. Session Configuration

Session settings override global settings.

Example:

```text
session:
    k3s-debug

logging:
    commands = true
    output = true
    timestamps = true
    duration = true
    exit_codes = true
    working_directory = true
```

These settings persist with the session.

---

# 29. Configuration Precedence

Highest priority:

```text
CLI argument
     ↓
Session setting
     ↓
Global setting
     ↓
Application default
```

Example:

```bash
termnote --timestamps=false
```

overrides the session configuration for that invocation.

---

# 30. Notes

Notes are first-class events.

Command:

```bash
termnote note
```

The configured editor opens.

Default:

```text
vim
```

The user writes Markdown.

Example:

```markdown
# Investigation

The API pod cannot reach PostgreSQL.

## Observations

- PostgreSQL is running.
- DNS resolution works.
- Port 5432 is reachable from another pod.

## Hypothesis

The application is using the wrong service name.
```

User executes:

```vim
:wq
```

The application captures the resulting file contents.

The temporary file is deleted after successful persistence.

---

# 31. Editor Configuration

Global:

```toml
[editor]
command = "vim"
```

Possible:

```toml
command = "nvim"
```

or:

```toml
command = "emacs"
```

The editor command should be launched using the user's environment.

No editor should be hardcoded.

Fallback:

```text
$VISUAL
$EDITOR
vim
```

Recommended precedence:

```text
CLI
↓
session editor
↓
$VISUAL
↓
$EDITOR
↓
vim
```

---

# 32. Note Positioning

A note must be inserted into the timeline at the exact event position where the user invoked it.

Example:

```text
Command 91
Command 92
Command 93
NOTE
Command 94
Command 95
```

The note must not simply appear at the end of the session later.

---

# 33. Empty Notes

If the editor exits with an empty document:

```text
No note saved.
```

No event is created.

---

# 34. Bookmarks

Command:

```bash
termnote bookmark
```

or:

```bash
termnote -b
```

creates:

```text
BOOKMARK
```

at the current event position.

---

# 35. Bookmark Naming

Interactive form:

```text
Bookmark name:
```

User enters:

```text
OpenBao authentication failure
```

Alternatively:

```bash
termnote -b "OpenBao authentication failure"
```

The exact CLI syntax should support both.

---

# 36. Bookmark Semantics

A bookmark is a pointer, not a copy.

Example:

```text
Bookmark
    ↓
Event ID 184
    ↓
COMMAND
kubectl logs openbao-0
```

If events are exported, the bookmark should resolve to that event.

---

# 37. Bookmark Navigation

Future TUI functionality:

```text
Bookmarks

  [1] OpenBao authentication failure
  [2] Working ingress
  [3] Final fix
```

Selecting one jumps directly to that location.

CLI:

```bash
termnote bookmark list
```

and:

```bash
termnote bookmark show 2
```

---

# 38. Sessions CLI

Core commands:

```bash
termnote
termnote new <name>
termnote attach <name>
termnote list
termnote archive <name>
termnote restore <name>
termnote delete <name>
termnote rename <old> <new>
termnote note
termnote bookmark
termnote export
```

Aliases may be provided.

---

# 39. Proposed CLI

```text
termnote new <session>
termnote attach <session>
termnote list
termnote status
termnote note
termnote bookmark
termnote archive <session>
termnote restore <session>
termnote delete <session>
termnote export <session>
termnote config
```

---

# 40. Default `termnote` Behavior

Running:

```bash
termnote
```

should open the session manager TUI.

Example:

```text
┌────────────────────────────────────────────────────────┐
│ TERMNOTE                                                │
├────────────────────────────────────────────────────────┤
│                                                        │
│ Sessions                                               │
│                                                        │
│ > k3s-debug               ACTIVE                       │
│   openchoreo              DETACHED                     │
│   rpi-networking          ARCHIVED                     │
│   linux-learning          DETACHED                     │
│                                                        │
│                                                        │
│ n New   a Attach   e Edit   d Delete   q Quit          │
└────────────────────────────────────────────────────────┘
```

---

# 41. Session TUI

Opening a session:

```text
┌────────────────────────────────────────────────────────────┐
│ k3s-debug                                     ACTIVE        │
├────────────────────────────────────────────────────────────┤
│ 03:12:31 COMMAND                                           │
│ kubectl get pods -A                                        │
│                                                            │
│ 03:12:32 OUTPUT                                            │
│ NAME                 READY    STATUS                       │
│ postgres-0           1/1      Running                      │
│                                                            │
│ 03:14:12 BOOKMARK                                         │
│ PostgreSQL debugging                                      │
│                                                            │
│ 03:15:01 NOTE                                              │
│ # Investigation                                           │
│ The service cannot connect...                             │
│                                                            │
├────────────────────────────────────────────────────────────┤
│ n Note  b Bookmark  / Search  ↑↓ Navigate  q Quit          │
└────────────────────────────────────────────────────────────┘
```

---

# 42. Session Search

Search must work across:

* Commands
* Output
* Notes
* Bookmarks

Example:

```bash
termnote search "exec format error"
```

Result:

```text
2026-08-10 02:14:21
Session: harbor-debug

kubectl logs harbor-database-0

OUTPUT:
exec /bin/sh: exec format error
```

---

# 43. Full-Text Search

SQLite FTS5 should be considered.

Potential searchable fields:

```text
commands
outputs
notes
bookmark names
```

This makes searches like:

```bash
termnote search "postgres"
```

fast even with years of history.

---

# 44. Storage

Recommended:

```text
SQLite
```

Database:

```text
~/.local/share/termnote/termnote.db
```

Use:

```sql
PRAGMA journal_mode = WAL;
```

---

# 45. Why WAL

WAL provides:

* Better concurrent read behavior
* Better crash resilience
* Incremental persistence
* Good fit for append-heavy event storage

The application should also configure appropriate:

```sql
PRAGMA synchronous=NORMAL;
```

as the default performance/safety tradeoff, with an optional safer mode.

---

# 46. Database Schema

Core:

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    status TEXT NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0,
    active_pid INTEGER,
    active_host TEXT,
    active_terminal TEXT,
    heartbeat_at INTEGER,
    shell TEXT,
    cwd TEXT,
    settings TEXT
);
```

---

# 47. Events Table

```sql
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    type TEXT NOT NULL,
    timestamp_start INTEGER,
    timestamp_end INTEGER,
    duration_ns INTEGER,
    payload TEXT,
    FOREIGN KEY(session_id) REFERENCES sessions(id)
);
```

`payload` can initially use JSON.

---

# 48. Commands

Option A: commands are events.

Option B: separate normalized table.

For V1, use events with structured payloads.

Example:

```json
{
  "command": "kubectl get pods -A",
  "exit_code": 0,
  "cwd": "/home/user/k3s"
}
```

---

# 49. Notes Table

Notes should eventually become their own table:

```sql
CREATE TABLE notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    event_id INTEGER NOT NULL,
    markdown TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

---

# 50. Bookmarks Table

```sql
CREATE TABLE bookmarks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    event_id INTEGER NOT NULL,
    name TEXT,
    created_at INTEGER NOT NULL
);
```

---

# 51. Settings

Global settings:

```text
config.toml
```

Session settings:

```text
SQLite
```

This avoids scattering session-specific configuration across files.

---

# 52. Export — Markdown

Command:

```bash
termnote export k3s-debug --format markdown
```

Example:

````markdown
# k3s-debug

Created: 2026-08-12 03:12:01

---

## 03:12:04 — Command

```bash
kubectl get pods -A
````

### Output

```text
NAME              READY   STATUS
postgres-0        1/1     Running
api-0             1/1     Running
```

**Duration:** 823 ms
**Exit code:** 0

---

## 03:14:01 — Bookmark

### PostgreSQL debugging

---

## 03:14:30 — Note

# Investigation

The PostgreSQL service is healthy.

The problem appears to be networking.

---

````

The Markdown export should be human-readable without requiring `termnote`.

---

# 53. Export — CSV

Command:

```bash
termnote export k3s-debug --format csv
````

CSV columns:

```text
session_id
sequence
event_id
event_type
timestamp_start
timestamp_end
duration_ms
command
output
exit_code
cwd
note
bookmark
```

Each event becomes a row.

---

# 54. Export Rules

Exports should support:

```bash
--output file.md
```

or:

```bash
> file.md
```

The exported data must contain enough information to be useful independently of `termnote`.

---

# 55. Archive

Archive:

```bash
termnote archive k3s-debug
```

Archived sessions:

* Cannot be attached
* Remain searchable
* Remain exportable
* Remain readable

Restore:

```bash
termnote restore k3s-debug
```

---

# 56. Delete

Deletion must require confirmation.

```text
Delete session "k3s-debug"?

This will permanently delete:

  3,412 events
  184 MB of output
  17 notes
  12 bookmarks

Type the session name to confirm:
```

Optional:

```bash
--force
```

---

# 57. Output Size Management

Long-running sessions could produce huge amounts of output.

Therefore configuration should support:

```toml
[storage]
max_output_size = "1GB"
```

Potential future options:

* Compression
* Output truncation
* Per-command output limits
* Automatic archival

Do not silently discard data.

---

# 58. Security & Privacy

Terminal sessions may contain extremely sensitive information.

Examples:

```text
API keys
passwords
tokens
SSH commands
private URLs
database credentials
```

Therefore:

### Default

All data is local.

No network communication.

No telemetry.

No cloud service.

---

# 59. Secret Redaction

V1 should **not attempt aggressive automatic secret detection**, because incorrect redaction could destroy useful terminal history.

Instead, provide:

```bash
termnote redact
```

as a future feature.

Potential future detection:

```text
AWS_SECRET_ACCESS_KEY
Authorization: Bearer ...
password=...
```

but this should not be part of the initial implementation.

---

# 60. Database Security

The SQLite database should respect normal filesystem permissions.

Recommended:

```text
0700 ~/.local/share/termnote
0600 termnote.db
```

No server should be exposed.

---

# 61. Crash Recovery

If the process crashes:

```text
termnote
   ↓
crash
```

the next invocation should detect:

```text
Session "k3s-debug" appears to have terminated unexpectedly.

Last heartbeat:
03:31:41

Recover session?
[Y/n]
```

The session becomes detached.

Existing events remain intact.

---

# 62. Heartbeat

The active instance should periodically update:

```text
heartbeat_at
```

Recommended interval:

```text
1–5 seconds
```

This allows stale session detection.

---

# 63. Stale Ownership

If:

```text
current_time - heartbeat > threshold
```

the session can be considered stale.

Example:

```text
Session appears to have been abandoned.

Options:

> Recover session
  Cancel
```

Never automatically steal a live session merely because of a temporary delay.

---

# 64. Terminal Resize

Because `termnote` sits between terminal and shell, it must correctly forward terminal resize events.

When terminal changes from:

```text
120 × 40
```

to:

```text
180 × 50
```

the child PTY must receive the new dimensions.

This is essential for:

* vim
* htop
* tmux
* interactive programs
* full-screen TUIs

---

# 65. Interactive Programs

This is a critical requirement.

The system must support commands such as:

```bash
vim
top
htop
less
ssh
tmux
python
sudo
```

without breaking the terminal.

`termnote` must therefore behave like a real PTY proxy rather than simply piping stdin/stdout.

---

# 66. Nested Terminal Programs

Running:

```bash
tmux
```

inside `termnote` should work.

Running:

```bash
ssh server
```

should work.

However, command-level semantic tracking inside nested remote shells is not guaranteed.

The raw terminal stream should still be recorded.

---

# 67. Shell Agnosticism

The core system must work with:

```text
bash
zsh
fish
dash
sh
ksh
nushell
```

where the shell can operate correctly inside the PTY.

Shell detection should be based on:

```text
$SHELL
```

or configured shell path, but should never require a particular shell.

---

# 68. Terminal Emulator Agnosticism

The application must work from:

```text
GNOME Terminal
Konsole
Kitty
Alacritty
WezTerm
xterm
foot
TTY
SSH
```

provided the terminal exposes a normal PTY.

---

# 69. Linux Distribution Agnosticism

No distro-specific APIs.

Avoid dependencies on:

```text
systemd
apt
dnf
pacman
snap
flatpak
```

The binary should run independently.

Target:

```text
Linux x86_64
Linux aarch64
```

Initially.

---

# 70. Rust Architecture

Recommended crate structure:

```text
termnote/
│
├── crates/
│   ├── termnote-core/
│   ├── termnote-pty/
│   ├── termnote-storage/
│   ├── termnote-session/
│   ├── termnote-editor/
│   ├── termnote-export/
│   ├── termnote-tui/
│   └── termnote-cli/
│
├── Cargo.toml
└── README.md
```

---

# 71. Suggested Rust Libraries

### PTY

Investigate:

```text
portable-pty
```

or equivalent Rust PTY implementation.

### SQLite

```text
rusqlite
```

with bundled SQLite where appropriate.

### TUI

```text
ratatui
```

### Terminal handling

```text
crossterm
```

### Serialization

```text
serde
serde_json
```

### CLI

```text
clap
```

### Time

```text
chrono
```

plus:

```text
std::time::Instant
```

for duration measurement.

### Errors

```text
thiserror
anyhow
```

---

# 72. Async Runtime

Avoid introducing async everywhere unnecessarily.

The PTY layer may use threads/channels where appropriate.

A reasonable architecture:

```text
PTY reader thread
       │
       ▼
event channel
       │
       ▼
event processor
       │
       ├── SQLite
       ├── TUI
       └── session manager
```

This keeps terminal I/O responsive.

---

# 73. Event Pipeline

```text
PTY
 │
 ▼
Raw bytes
 │
 ▼
Terminal/event parser
 │
 ▼
Event detector
 │
 ▼
Session Event
 │
 ├──────────────► SQLite
 │
 └──────────────► TUI
```

Raw PTY data should optionally be retained.

---

# 74. Important Design Decision

The application should distinguish between:

```text
Raw terminal stream
```

and:

```text
Semantic events
```

Example:

```text
Raw stream:
ANSI codes
cursor movements
characters
backspaces
output

Semantic layer:
COMMAND
COMMAND_OUTPUT
NOTE
BOOKMARK
```

This makes the system more robust.

---

# 75. Event IDs

Every event receives a monotonically increasing session-local sequence number.

Example:

```text
1
2
3
4
5
...
```

This gives a stable timeline.

Bookmarks point to sequence/event IDs.

---

# 76. Append-Only Philosophy

The event log should be fundamentally append-only.

Avoid modifying historical command events.

Corrections should be represented as new events.

Example:

```text
NOTE
I initially believed DNS was broken.

NOTE
Correction: DNS was working; the issue was the service port.
```

This preserves the engineering history.

---

# 77. TUI Navigation

Recommended keybindings:

```text
↑ / k       Previous event
↓ / j       Next event
Ctrl-u      Page up
Ctrl-d      Page down
g           Beginning
G           End
/           Search
n           Next search result
b           Bookmark
N           New note
e           Export
s           Session settings
q           Quit
```

---

# 78. Session Manager Keybindings

```text
n       New session
Enter   Attach
d       Delete
a       Archive
r       Restore
e       Rename
/       Search
q       Quit
```

---

# 79. Session Settings UI

Example:

```text
Session Settings
────────────────────────

Logging

[x] Commands
[x] Output
[x] Timestamps
[x] Duration
[x] Exit codes
[x] Working directory
[ ] Hostname

Editor

Editor: vim

Storage

Output limit: 1 GB

[Save] [Cancel]
```

---

# 80. Global Settings UI

The same interface should exist globally.

CLI:

```bash
termnote config
```

or:

```bash
termnote config set logging.output false
```

---

# 81. Versioning

Database schema must have migrations.

Example:

```text
migrations/
├── 001_initial.sql
├── 002_fts.sql
└── 003_bookmarks.sql
```

Database:

```text
schema_version
```

---

# 82. Backups

Provide:

```bash
termnote backup
```

which creates:

```text
termnote-backup-2026-08-12.db
```

Because SQLite is used, backup should use SQLite's backup mechanism rather than simply copying a live WAL database.

---

# 83. Import

Not required for V1.

Potential future:

```bash
termnote import session.md
termnote import session.csv
```

---

# 84. Multi-Machine Sessions

Not required for V1.

Sessions are local to a machine.

Future possibility:

```text
termnote sync
```

but this should not influence the initial architecture.

---

# 85. Remote Sessions

SSH support should work naturally:

```bash
ssh server
```

inside `termnote`.

But the remote shell is not a separate `termnote` session unless `termnote` is installed and launched remotely.

---

# 86. Installation

The initial release should produce a single binary:

```text
termnote
```

No runtime dependency.

Potential installation methods:

```text
cargo install termnote
```

and eventually:

```text
.deb
.rpm
Arch package
Nix
Homebrew
```

---

# 87. Shell Integration

The base application must require **zero shell configuration**.

However, optional integrations can later improve command boundary detection.

For example:

```bash
termnote shell install bash
```

would install an optional integration.

But:

```bash
termnote
```

must work without it.

---

# 88. Performance Requirements

Startup:

```text
<100 ms
```

for normal systems where possible.

Terminal input latency:

```text
<10 ms
```

target added overhead.

Recording should not noticeably slow commands.

---

# 89. Storage Performance

The event writer should batch non-critical metadata where possible while ensuring command records aren't lost.

Suggested:

```text
Command event → immediate SQLite transaction
Output → buffered chunks
```

with periodic flushing.

For maximum durability mode:

```text
Every event → transaction → WAL
```

---

# 90. Large Output

Commands such as:

```bash
kubectl logs
journalctl
cat hugefile
```

can generate massive output.

The implementation must avoid:

```text
read entire output into RAM
```

Instead:

```text
PTY
 ↓
chunk
 ↓
SQLite writer
```

Output should be streamed.

---

# 91. Backpressure

If SQLite becomes slow:

```text
PTY → bounded buffer → storage
```

must be carefully designed so that the terminal does not become unusably laggy.

Potential strategy:

* Small in-memory event queue
* Background writer
* Raw output chunk persistence
* Configurable durability

---

# 92. Data Integrity

Every event should have:

```text
session_id
sequence
event_id
```

Optionally future versions can add hashes:

```text
previous_hash
event_hash
```

This could turn the log into a tamper-evident chain.

Not required for V1.

---

# 93. Logging Levels

Application logging should be separate from recorded terminal output.

Example:

```bash
RUST_LOG=debug termnote
```

Application diagnostics should never be inserted into the user's session history.

---

# 94. Error Handling

If storage fails:

```text
WARNING: Unable to persist session data.

Terminal recording continues in memory.

Retrying...
```

The terminal should not immediately die because SQLite is temporarily unavailable.

However, if the buffer becomes full, the user must be informed.

---

# 95. Graceful Shutdown

On:

```text
Ctrl-D
exit
terminal close
SIGTERM
```

the application should:

1. Stop accepting input
2. Flush pending events
3. Commit database transaction
4. Update session state
5. Release ownership
6. Close PTY
7. Exit

---

# 96. SIGKILL

`SIGKILL` cannot be handled.

Therefore:

* WAL
* frequent persistence
* heartbeat
* stale-owner recovery

are essential.

---

# 97. Session Naming

Names should allow:

```text
k3s-debug
openchoreo
rpi networking
```

Recommended normalization:

```text
1–100 characters
```

Avoid filesystem dependence because names live in SQLite.

---

# 98. Session Metadata

Future-friendly fields:

```text
description
tags
project
host
git_repo
git_branch
```

Example:

```text
Session: openchoreo

Tags:
k3s
kubernetes
openchoreo
debugging
```

Tags are not required for V1 but schema should not prevent them.

---

# 99. Session Dashboard

A future dashboard:

```text
SESSION: k3s-debug

Created:       Aug 12 03:12
Duration:      2h 17m
Commands:      381
Notes:         14
Bookmarks:     8
Failures:      31
Output:        142 MB

Last command:
kubectl get pods -A
```

---

# 100. Statistics

The system should eventually calculate:

```text
Total commands
Successful commands
Failed commands
Average duration
Longest command
Total session duration
Output generated
```

Example:

```text
Longest command:
helm upgrade --install openchoreo
Duration: 4m 31s
```

---

# 101. Session Resume UX

Ideal experience:

```bash
termnote
```

shows:

```text
Recent sessions

> k3s-debug       DETACHED   2m ago
  openchoreo      DETACHED   1h ago
  rpi-network     ARCHIVED   2d ago

Enter = attach
n = new
q = quit
```

---

# 102. Quick Start

There should be an easy path:

```bash
termnote new k3s
```

Then the user simply uses the terminal normally.

No additional commands are necessary to record commands.

---

# 103. Notes While Working

At any point:

```bash
termnote note
```

Vim opens.

After:

```vim
:wq
```

the terminal returns to exactly where it was.

The note is now part of the timeline.

---

# 104. Bookmark While Working

At any point:

```bash
termnote -b
```

The application records:

```text
BOOKMARK
Current position
```

No interruption beyond a small prompt if naming is requested.

---

# 105. Export Workflow

Example:

```bash
termnote export k3s-debug \
    --format markdown \
    --output k3s-debug.md
```

Then:

```bash
vim k3s-debug.md
```

The result can become:

* Blog material
* Debugging report
* Research notes
* Documentation
* Incident report

---

# 106. MVP Definition

The first usable release should contain:

### Terminal

* PTY management
* Shell launching
* stdin/stdout forwarding
* resize handling
* interactive programs

### Sessions

* Create
* Attach
* Detach
* Continue
* Delete
* Archive
* Restore
* Single-terminal ownership

### Recording

* Commands
* Output
* Timestamps
* Duration
* Exit code
* Working directory

### Notes

* Vim/editor integration
* Markdown
* Timeline positioning

### Bookmarks

* Create
* Name
* Navigate

### Storage

* SQLite
* WAL
* Crash recovery
* Heartbeat

### Export

* Markdown
* CSV

### UI

* Basic TUI
* Session list
* Timeline
* Search

---

# 107. V1.1

After MVP:

* FTS5 search
* Session tags
* Session statistics
* Better bookmark navigation
* Configurable themes
* Output compression
* Better command detection
* Optional Bash/Zsh/Fish integrations

---

# 108. V2

Potential advanced functionality:

```text
Git-like session branching
Session diff
Remote synchronization
Encrypted databases
Cloud sync
Secret detection
Session replay
Terminal recording playback
AI-generated summaries
Automatic incident reports
```

But none of these should complicate the MVP.

---

# 109. Example Complete Workflow

User starts:

```bash
termnote new k3s-openchoreo
```

They run:

```bash
kubectl get pods -A
```

`termnote` records:

```text
COMMAND
kubectl get pods -A

START
03:12:31.182

END
03:12:31.421

DURATION
239ms

EXIT
0
```

They discover an error.

They run:

```bash
termnote -b "OpenChoreo failure"
```

Then:

```bash
termnote note
```

Vim opens.

They write:

```markdown
# OpenChoreo Failure

The control plane is healthy.

The gateway is failing to obtain its certificate.

Need to inspect cert-manager.
```

They save:

```vim
:wq
```

Then continue:

```bash
kubectl get certificates -A
```

They close their laptop.

Later:

```bash
termnote
```

They select:

```text
k3s-openchoreo
```

The session resumes.

They run another command.

The timeline continues:

```text
03:12 COMMAND
03:13 COMMAND
03:14 BOOKMARK
03:15 NOTE
03:17 COMMAND
...
NEXT DAY
09:42 COMMAND
09:44 NOTE
...
```

It is one continuous engineering record.

---

# 110. Acceptance Criteria

The MVP is considered successful when all of the following work.

### AC1

```bash
termnote new test
```

opens a working shell.

### AC2

Commands execute normally.

### AC3

Interactive applications such as `vim`, `top`, and `ssh` work.

### AC4

Commands and output are persisted.

### AC5

Duration is recorded.

### AC6

Exit codes are recorded.

### AC7

The terminal can be closed and the session later reattached.

### AC8

Only one terminal can own a session.

### AC9

Attempting to attach from another terminal presents the takeover/continue choice.

### AC10

"Continue here" terminates the old terminal's `termnote` instance and transfers ownership.

### AC11

Notes open in Vim.

### AC12

`:wq` saves the Markdown note.

### AC13

Notes appear at the correct timeline position.

### AC14

`termnote -b` creates a bookmark at the current position.

### AC15

Sessions can be archived and restored.

### AC16

Sessions can be deleted.

### AC17

Global logging configuration works.

### AC18

Session logging configuration overrides global configuration.

### AC19

Settings survive reopening the session.

### AC20

SQLite WAL is enabled.

### AC21

A process crash does not corrupt the database or destroy already committed events.

### AC22

Markdown export produces a readable standalone document.

### AC23

CSV export contains the recorded event information.

### AC24

The application works with at least:

```text
bash
zsh
fish
```

without requiring shell modifications.

### AC25

The application works in at least:

```text
GNOME Terminal
Kitty
Alacritty
```

without terminal-specific integrations.

### AC26

The application works on:

```text
x86_64 Linux
aarch64 Linux
```

---

# 111. Recommended Repository Structure

```text
termnote/
│
├── Cargo.toml
├── Cargo.lock
├── LICENSE
├── README.md
│
├── crates/
│   ├── core/
│   │   ├── events.rs
│   │   ├── session.rs
│   │   ├── settings.rs
│   │   └── errors.rs
│   │
│   ├── pty/
│   │   ├── manager.rs
│   │   ├── process.rs
│   │   ├── resize.rs
│   │   └── io.rs
│   │
│   ├── storage/
│   │   ├── database.rs
│   │   ├── migrations.rs
│   │   ├── sessions.rs
│   │   ├── events.rs
│   │   ├── notes.rs
│   │   └── bookmarks.rs
│   │
│   ├── session/
│   │   ├── manager.rs
│   │   ├── ownership.rs
│   │   ├── heartbeat.rs
│   │   └── recovery.rs
│   │
│   ├── editor/
│   │   └── editor.rs
│   │
│   ├── export/
│   │   ├── markdown.rs
│   │   └── csv.rs
│   │
│   ├── tui/
│   │   ├── app.rs
│   │   ├── sessions.rs
│   │   ├── timeline.rs
│   │   ├── search.rs
│   │   └── settings.rs
│   │
│   └── cli/
│       └── commands.rs
│
├── migrations/
│   ├── 001_initial.sql
│   ├── 002_fts.sql
│   └── 003_indexes.sql
│
├── docs/
│   ├── architecture.md
│   ├── storage.md
│   ├── pty.md
│   └── command-detection.md
│
└── tests/
    ├── session_tests.rs
    ├── storage_tests.rs
    ├── export_tests.rs
    └── ownership_tests.rs
```

---

# 112. One Important Architectural Recommendation

I would **not** make the first implementation a traditional terminal emulator.

Make it a **PTY supervisor/proxy**.

The distinction is important:

```text
                WRONG DIRECTION

termnote
  └── implement a terminal emulator
        ├── ANSI parser
        ├── cursor
        ├── rendering
        ├── keyboard handling
        └── shell
```

versus:

```text
                 RECOMMENDED

Terminal Emulator
        │
        ▼
     termnote
        │
        ├── record
        ├── persist
        ├── notes
        ├── bookmarks
        └── session management
        │
        ▼
      PTY
        │
        ▼
      shell
```

This dramatically reduces the amount of terminal-emulation code you need to write.

The existing terminal emulator remains responsible for rendering the terminal. `termnote` primarily manages the PTY and records its stream.

---

# 113. Core Design Principle

The most important design rule for the project should be:

> **Never make the user's terminal experience depend on the recording system.**

If SQLite slows down, the terminal shouldn't freeze.

If an export fails, the shell shouldn't die.

If a note editor crashes, the session should continue.

If the database becomes temporarily unavailable, the application should buffer and recover where possible.

If `termnote` itself crashes, previously committed events should remain safe.

The user should feel like they are simply using a normal terminal, with a persistent engineering notebook quietly attached to it.

---

# 114. Final Product Concept

The resulting experience should essentially be:

```text
                         TERMNOTE

             ┌─────────────────────────────┐
             │       TERMINAL SESSION       │
             │                             │
             │ $ kubectl get pods -A       │
             │                             │
             │ NAME      READY    STATUS   │
             │ api       1/1      Running  │
             │ db        1/1      Running  │
             │                             │
             └──────────────┬──────────────┘
                            │
                            ▼
                 ┌──────────────────────┐
                 │    EVENT JOURNAL     │
                 │                      │
                 │ COMMAND              │
                 │ OUTPUT               │
                 │ NOTE                 │
                 │ BOOKMARK             │
                 │ COMMAND              │
                 │ OUTPUT               │
                 │ ...                  │
                 └──────────┬───────────┘
                            │
                            ▼
                    ┌───────────────┐
                    │ SQLite + WAL  │
                    └───────────────┘
                            │
                 ┌──────────┼──────────┐
                 ▼          ▼          ▼
              Markdown     CSV       Search
```

In other words, **Asciinema treats a terminal as something to replay; `termnote` should treat a terminal session as a structured, persistent lab notebook.** That distinction should drive the architecture from the beginning.

