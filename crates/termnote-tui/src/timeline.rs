//! Read-only timeline browser (PRD §41, §77): scroll through a session's
//! recorded history and search it. Attaching live is a separate, non-TUI
//! code path (see `app.rs` module docs) since a live PTY passthrough and a
//! ratatui screen can't share the terminal at once.

use std::io::Stdout;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;

use termnote_core::time::{format_duration_ns, format_local};
use termnote_core::{BookmarkPayload, CommandPayload, Event as TnEvent, EventType, NotePayload};
use termnote_storage::{events, sessions, SharedConn};

type Term = Terminal<CrosstermBackend<Stdout>>;

fn setup() -> anyhow::Result<Term> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn teardown(mut term: Term) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

pub fn run_timeline(db: &SharedConn, session_name: &str) -> anyhow::Result<()> {
    let session = sessions::require_by_name(db, session_name)?;
    let all_events = events::list_all(db, &session.id)?;
    let mut term = setup()?;
    let result = event_loop(&session.name, &all_events, &mut term);
    teardown(term)?;
    result
}

enum Mode {
    Browse,
    Search { buffer: String },
}

fn event_loop(session_name: &str, all_events: &[TnEvent], term: &mut Term) -> anyhow::Result<()> {
    let lines: Vec<(String, i64)> = all_events.iter().filter_map(|e| summarize(e).map(|s| (s, e.id.unwrap_or(0)))).collect();
    let mut visible: Vec<usize> = (0..lines.len()).collect();
    let mut state = ListState::default();
    if !visible.is_empty() {
        state.select(Some(0));
    }
    let mut mode = Mode::Browse;
    let mut query = String::new();

    loop {
        term.draw(|f| draw(f, session_name, &lines, &visible, &mut state, &mode))?;

        let ev = event::read()?;
        let Event::Key(key) = ev else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match &mut mode {
            Mode::Browse => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Down | KeyCode::Char('j') => move_selection(&mut state, visible.len(), 1),
                KeyCode::Up | KeyCode::Char('k') => move_selection(&mut state, visible.len(), -1),
                KeyCode::Char('g') => {
                    if !visible.is_empty() {
                        state.select(Some(0));
                    }
                }
                KeyCode::Char('G') => {
                    if !visible.is_empty() {
                        state.select(Some(visible.len() - 1));
                    }
                }
                KeyCode::Char('/') => mode = Mode::Search { buffer: String::new() },
                KeyCode::Char('n') if !query.is_empty() => {
                    visible = filter(&lines, &query);
                    if !visible.is_empty() {
                        state.select(Some(0));
                    }
                }
                _ => {}
            },
            Mode::Search { buffer } => match key.code {
                KeyCode::Enter => {
                    query = buffer.clone();
                    visible = filter(&lines, &query);
                    if visible.is_empty() {
                        visible = (0..lines.len()).collect();
                    }
                    state.select(if visible.is_empty() { None } else { Some(0) });
                    mode = Mode::Browse;
                }
                KeyCode::Esc => mode = Mode::Browse,
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Char(c) => buffer.push(c),
                _ => {}
            },
        }
    }
}

fn filter(lines: &[(String, i64)], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    lines
        .iter()
        .enumerate()
        .filter(|(_, (text, _))| text.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

fn move_selection(state: &mut ListState, len: usize, delta: i32) {
    if len == 0 {
        return;
    }
    let current = state.selected().unwrap_or(0) as i32;
    let next = (current + delta).rem_euclid(len as i32);
    state.select(Some(next as usize));
}

fn summarize(event: &TnEvent) -> Option<String> {
    let ts = event.timestamp_start.map(format_local).unwrap_or_default();
    match event.event_type {
        EventType::Command => {
            let p: CommandPayload = event.payload_as().ok()?;
            let dur = event.duration_ns.map(format_duration_ns).unwrap_or_else(|| "?".into());
            let exit = p.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
            Some(format!("{ts}  COMMAND   {}  (exit {exit}, {dur})", p.command))
        }
        EventType::Note => {
            let p: NotePayload = event.payload_as().ok()?;
            let first_line = p.markdown.lines().next().unwrap_or("").trim_start_matches('#').trim();
            Some(format!("{ts}  NOTE      {first_line}"))
        }
        EventType::Bookmark => {
            let p: BookmarkPayload = event.payload_as().ok()?;
            Some(format!("{ts}  BOOKMARK  {}", p.name.as_deref().unwrap_or("(unnamed)")))
        }
        EventType::SessionStart => Some(format!("{ts}  ── session started ──")),
        EventType::SessionAttach => Some(format!("{ts}  ── session attached ──")),
        EventType::SessionDetach => Some(format!("{ts}  ── session detached ──")),
        _ => None,
    }
}

fn draw(
    f: &mut ratatui::Frame<CrosstermBackend<Stdout>>,
    session_name: &str,
    lines: &[(String, i64)],
    visible: &[usize],
    state: &mut ListState,
    mode: &Mode,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(f.size());

    let items: Vec<ListItem> = visible
        .iter()
        .map(|&i| {
            let (text, _) = &lines[i];
            let color = if text.contains("COMMAND") {
                Color::White
            } else if text.contains("NOTE") {
                Color::Cyan
            } else if text.contains("BOOKMARK") {
                Color::Yellow
            } else {
                Color::DarkGray
            };
            ListItem::new(Line::from(Span::styled(text.clone(), Style::default().fg(color))))
        })
        .collect();

    let title = format!(" {session_name} — timeline ({} events) ", lines.len());
    let list_widget = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list_widget, chunks[0], state);

    let help_text = match mode {
        Mode::Browse => "↑↓/jk Navigate   g/G Top/Bottom   / Search   q Quit".to_string(),
        Mode::Search { buffer } => format!("Search: {buffer}_  (Enter to run, Esc to cancel)"),
    };
    let help = Paragraph::new(help_text).block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[1]);
}
