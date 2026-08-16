//! The session manager screen (PRD §40, §78): the default view when running
//! bare `termnote`, listing sessions with quick actions. Attaching hands
//! back control to the caller rather than trying to run a live PTY inside
//! ratatui's own screen -- the CLI then performs the actual attach exactly
//! as `termnote attach <name>` would, so there's exactly one code path for
//! "run a live recording session."

use std::io::Stdout;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;

use termnote_core::{Session, SessionStatus};
use termnote_storage::{sessions, SharedConn};

pub enum SessionManagerAction {
    Attach(String),
    New(String),
    Quit,
}

enum Mode {
    List,
    Prompt { buffer: String },
    ConfirmDelete { name: String, summary: String },
    Message(String),
}

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

pub fn run_session_manager(db: &SharedConn) -> anyhow::Result<SessionManagerAction> {
    let mut term = setup()?;
    let result = event_loop(db, &mut term);
    teardown(term)?;
    result
}

fn event_loop(db: &SharedConn, term: &mut Term) -> anyhow::Result<SessionManagerAction> {
    let mut list_state = ListState::default();
    list_state.select(Some(0));
    let mut mode = Mode::List;
    let mut sessions_cache = sessions::list(db, true)?;
    if !sessions_cache.is_empty() {
        list_state.select(Some(0));
    }

    loop {
        term.draw(|f| draw(f, &sessions_cache, &mut list_state, &mode))?;

        let ev = event::read()?;
        let Event::Key(key) = ev else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match &mut mode {
            Mode::List => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(SessionManagerAction::Quit),
                KeyCode::Down | KeyCode::Char('j') => move_selection(&mut list_state, sessions_cache.len(), 1),
                KeyCode::Up | KeyCode::Char('k') => move_selection(&mut list_state, sessions_cache.len(), -1),
                KeyCode::Char('n') => mode = Mode::Prompt { buffer: String::new() },
                KeyCode::Enter | KeyCode::Char('a') => {
                    if let Some(s) = selected(&sessions_cache, &list_state) {
                        return Ok(SessionManagerAction::Attach(s.name.clone()));
                    }
                }
                KeyCode::Char('r') => {
                    if let Some(s) = selected(&sessions_cache, &list_state) {
                        let name = s.name.clone();
                        let result = if s.is_archived() {
                            sessions::restore(db, &name)
                        } else {
                            sessions::archive(db, &name)
                        };
                        match result {
                            Ok(()) => sessions_cache = sessions::list(db, true)?,
                            Err(e) => mode = Mode::Message(format!("Error: {e}")),
                        }
                    }
                }
                KeyCode::Char('d') => {
                    if let Some(s) = selected(&sessions_cache, &list_state) {
                        if let Ok(preview) = sessions::delete_preview(db, &s.name) {
                            let summary = format!(
                                "{} events, {} notes, {} bookmarks",
                                preview.events, preview.notes, preview.bookmarks
                            );
                            mode = Mode::ConfirmDelete { name: s.name.clone(), summary };
                        }
                    }
                }
                _ => {}
            },
            Mode::Prompt { buffer } => match key.code {
                KeyCode::Enter => {
                    if !buffer.trim().is_empty() {
                        return Ok(SessionManagerAction::New(buffer.trim().to_string()));
                    }
                }
                KeyCode::Esc => mode = Mode::List,
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Char(c) => buffer.push(c),
                _ => {}
            },
            Mode::ConfirmDelete { name, .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let name = name.clone();
                    match sessions::delete(db, &name) {
                        Ok(()) => {
                            sessions_cache = sessions::list(db, true)?;
                            list_state.select(if sessions_cache.is_empty() { None } else { Some(0) });
                            mode = Mode::List;
                        }
                        Err(e) => mode = Mode::Message(format!("Error: {e}")),
                    }
                }
                _ => mode = Mode::List,
            },
            Mode::Message(_) => mode = Mode::List,
        }
    }
}

fn move_selection(state: &mut ListState, len: usize, delta: i32) {
    if len == 0 {
        return;
    }
    let current = state.selected().unwrap_or(0) as i32;
    let next = (current + delta).rem_euclid(len as i32);
    state.select(Some(next as usize));
}

fn selected<'a>(list: &'a [Session], state: &ListState) -> Option<&'a Session> {
    state.selected().and_then(|i| list.get(i))
}

fn draw(f: &mut ratatui::Frame<CrosstermBackend<Stdout>>, list: &[Session], state: &mut ListState, mode: &Mode) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(f.size());

    let items: Vec<ListItem> = list
        .iter()
        .map(|s| {
            let (label, color) = status_label(s.status);
            let line = Line::from(vec![
                Span::raw(format!("{:<28}", s.name)),
                Span::styled(label, Style::default().fg(color).add_modifier(Modifier::BOLD)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list_widget = List::new(items)
        .block(Block::default().title(" termnote — sessions ").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    f.render_stateful_widget(list_widget, chunks[0], state);

    let help_text = match mode {
        Mode::List => "n New   Enter/a Attach   r Archive/Restore   d Delete   q Quit".to_string(),
        Mode::Prompt { buffer } => format!("New session name: {buffer}_  (Enter to confirm, Esc to cancel)"),
        Mode::ConfirmDelete { name, summary } => {
            format!("Delete \"{name}\"? This removes {summary}. [y/N]")
        }
        Mode::Message(m) => m.clone(),
    };
    let help = Paragraph::new(help_text).block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[1]);
}

fn status_label(status: SessionStatus) -> (&'static str, Color) {
    match status {
        SessionStatus::Active => ("ACTIVE", Color::Green),
        SessionStatus::Detached => ("DETACHED", Color::Yellow),
        SessionStatus::Archived => ("ARCHIVED", Color::DarkGray),
        SessionStatus::New => ("NEW", Color::Cyan),
        SessionStatus::Deleted => ("DELETED", Color::Red),
    }
}
