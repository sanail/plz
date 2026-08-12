//! Full-screen interactive mode: running `plz` with no arguments.
//!
//! The request to the model runs on a background thread while the UI keeps
//! redrawing — otherwise the window would look frozen for the seconds it waits.

use std::io::{self, IsTerminal};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{execute, terminal};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::context::Context;
use crate::provider::Provider;
use crate::suggestion::Suggestion;
use crate::ui::Outcome;

/// How often the spinner is redrawn while waiting for a reply.
const TICK: Duration = Duration::from_millis(120);
const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

/// What the interactive mode produced.
pub struct Session {
    pub outcome: Outcome,
    pub suggestions: Vec<Suggestion>,
}

/// What is currently on screen.
enum Screen {
    /// Typing the query
    Editing,
    /// Request sent, waiting for a reply
    Waiting {
        started: Instant,
        rx: Receiver<Result<Vec<Suggestion>>>,
    },
    /// Suggestions arrived
    Choosing { selected: usize },
    /// The request failed; show the error without leaving the mode
    Failed { message: String },
}

struct App {
    query: String,
    screen: Screen,
    suggestions: Vec<Suggestion>,
    spinner: usize,
    buffer_supported: bool,
    count: usize,
}

/// Launch the interactive mode.
pub fn run(
    provider: Arc<dyn Provider + Send + Sync>,
    ctx: Context,
    count: usize,
    buffer_supported: bool,
) -> Result<Session> {
    if !io::stderr().is_terminal() {
        anyhow::bail!(
            "interactive mode needs a terminal.\n\
             Pass the task as an argument instead: plz \"describe your task\""
        );
    }

    // Drawn to stderr: stdout stays clean for `--json` and pipes.
    let mut stderr = io::stderr();
    terminal::enable_raw_mode()?;
    execute!(stderr, terminal::EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App {
        query: String::new(),
        screen: Screen::Editing,
        suggestions: Vec::new(),
        spinner: 0,
        buffer_supported,
        count,
    };

    let result = event_loop(&mut terminal, &mut app, provider, ctx);

    // Restore the terminal whatever happens, including a panic while drawing:
    // otherwise the user is stranded in the alternate screen with no echo.
    let _ = terminal::disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), terminal::LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result.map(|outcome| Session {
        outcome,
        suggestions: app.suggestions,
    })
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    app: &mut App,
    provider: Arc<dyn Provider + Send + Sync>,
    ctx: Context,
) -> Result<Outcome> {
    loop {
        terminal.draw(|frame| draw(frame, app))?;

        // While waiting, poll the channel and keep the spinner turning.
        if let Screen::Waiting { rx, .. } = &app.screen {
            match rx.try_recv() {
                Ok(Ok(suggestions)) if suggestions.is_empty() => {
                    app.screen = Screen::Failed {
                        message: "The model returned no suggestions.".into(),
                    };
                }
                Ok(Ok(suggestions)) => {
                    app.suggestions = suggestions;
                    app.screen = Screen::Choosing { selected: 0 };
                }
                Ok(Err(err)) => {
                    app.screen = Screen::Failed {
                        message: format!("{err:#}"),
                    };
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    app.screen = Screen::Failed {
                        message: "the background request was interrupted".into(),
                    };
                }
            }
        }

        if !event::poll(TICK)? {
            app.spinner = app.spinner.wrapping_add(1);
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match &mut app.screen {
            Screen::Editing => match key.code {
                KeyCode::Esc => return Ok(Outcome::Cancel),
                KeyCode::Char('c' | 'd') if ctrl => return Ok(Outcome::Cancel),
                KeyCode::Enter if !app.query.trim().is_empty() => {
                    app.screen = spawn_request(&provider, &ctx, &app.query, app.count);
                }
                KeyCode::Backspace => {
                    app.query.pop();
                }
                // Only bare characters are text: without the guard an unbound
                // chord like Ctrl+X would silently type an "x" into the query.
                KeyCode::Char(c) if !ctrl => app.query.push(c),
                _ => {}
            },

            Screen::Waiting { .. } => {
                // While waiting, only cancellation is accepted.
                if matches!(key.code, KeyCode::Esc)
                    || (ctrl && matches!(key.code, KeyCode::Char('c' | 'd')))
                {
                    return Ok(Outcome::Cancel);
                }
            }

            Screen::Failed { .. } => match key.code {
                KeyCode::Esc => return Ok(Outcome::Cancel),
                KeyCode::Char('c' | 'd') if ctrl => return Ok(Outcome::Cancel),
                // Any other key goes back to editing the query.
                _ => app.screen = Screen::Editing,
            },

            Screen::Choosing { selected } => {
                let total = app.suggestions.len();
                match key.code {
                    // The ctrl guards must stay above the plain 'c' arm below,
                    // which is the one that copies.
                    KeyCode::Char('c' | 'd') if ctrl => return Ok(Outcome::Cancel),
                    KeyCode::Char('n') if ctrl => {
                        app.query.clear();
                        app.suggestions.clear();
                        app.screen = Screen::Editing;
                    }
                    KeyCode::Char('r') if ctrl => {
                        app.screen = spawn_request(&provider, &ctx, &app.query, app.count);
                    }
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(Outcome::Cancel),
                    KeyCode::Char('c') => return Ok(Outcome::Copy(*selected)),
                    KeyCode::Enter => return Ok(Outcome::Run(*selected)),
                    KeyCode::Tab | KeyCode::Char('e') => {
                        return Ok(if app.buffer_supported {
                            Outcome::Buffer(*selected)
                        } else {
                            Outcome::Copy(*selected)
                        });
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        *selected = selected.checked_sub(1).unwrap_or(total - 1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        *selected = (*selected + 1) % total;
                    }
                    KeyCode::Char(c @ '1'..='9') => {
                        let index = c as usize - '1' as usize;
                        if index < total {
                            return Ok(Outcome::Run(index));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Send the request off to a background thread.
///
/// Blocking HTTP on a separate thread rather than async: a single task for the
/// whole process lifetime does not justify pulling in a tokio runtime.
fn spawn_request(
    provider: &Arc<dyn Provider + Send + Sync>,
    ctx: &Context,
    query: &str,
    count: usize,
) -> Screen {
    let (tx, rx) = mpsc::channel();
    let provider = Arc::clone(provider);
    let ctx = ctx.clone();
    let query = query.trim().to_string();

    std::thread::spawn(move || {
        // A send error just means the user already quit; ignore it.
        let _ = tx.send(provider.suggest(&ctx, &query, count));
    });

    Screen::Waiting {
        started: Instant::now(),
        rx,
    }
}

fn draw(frame: &mut Frame, app: &App) {
    let areas = Layout::vertical([
        Constraint::Length(3), // query input
        Constraint::Min(3),    // suggestion list or message
        Constraint::Length(1), // key hints
    ])
    .split(frame.area());

    draw_query(frame, app, areas[0]);
    draw_body(frame, app, areas[1]);
    draw_hints(frame, app, areas[2]);
}

fn draw_query(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let input = Paragraph::new(app.query.as_str()).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" What do you need to do? "),
    );
    frame.render_widget(input, area);

    // The cursor is placed only while editing; otherwise it would blink over
    // the list.
    if matches!(app.screen, Screen::Editing) {
        // The width comes from the terminal and in degenerate cases (a freshly
        // opened pty, a window a couple of columns wide) can be zero, so
        // subtract with saturation rather than plain minus.
        let last_column = area.x + area.width.saturating_sub(2);
        let cursor_x = area.x.saturating_add(1) + app.query.chars().count() as u16;
        frame.set_cursor_position((cursor_x.min(last_column), area.y + 1));
    }
}

fn draw_body(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    match &app.screen {
        Screen::Editing => {
            let hint = Paragraph::new("Describe your task in plain language and press Enter.")
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(hint, area);
        }

        Screen::Waiting { started, .. } => {
            let spinner = SPINNER[app.spinner % SPINNER.len()];
            let seconds = started.elapsed().as_secs();
            let text = if seconds >= 3 {
                format!("{spinner} Thinking… ({seconds}s)")
            } else {
                format!("{spinner} Thinking…")
            };
            let waiting = Paragraph::new(text)
                .style(Style::default().fg(Color::Yellow))
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(waiting, area);
        }

        Screen::Failed { message } => {
            let error = Paragraph::new(message.as_str())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(Borders::ALL).title(" Error "));
            frame.render_widget(error, area);
        }

        Screen::Choosing { selected } => {
            let items: Vec<ListItem> = app
                .suggestions
                .iter()
                .enumerate()
                .map(|(i, suggestion)| {
                    let chosen = i == *selected;
                    let marker = if chosen { "❯" } else { " " };
                    let command_style = if chosen {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let mut lines = vec![Line::from(vec![
                        Span::raw(format!("{marker} {}  ", i + 1)),
                        Span::styled(suggestion.one_line(), command_style),
                    ])];
                    if !suggestion.explanation.is_empty() {
                        lines.push(Line::from(Span::styled(
                            format!("     {}", suggestion.explanation),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                    ListItem::new(lines)
                })
                .collect();

            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Suggestions "),
            );
            frame.render_widget(list, area);
        }
    }
}

fn draw_hints(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let text = match &app.screen {
        Screen::Editing => "Enter send · Esc quit".to_string(),
        Screen::Waiting { .. } => "Esc cancel".to_string(),
        Screen::Failed { .. } => "Any key to edit the query · Esc quit".to_string(),
        Screen::Choosing { .. } => {
            let tab = if app.buffer_supported {
                "Tab to edit"
            } else {
                "Tab to copy"
            };
            format!(
                "1-9/↑↓ select · Enter run · {tab} · c copy · \
                 Ctrl+R retry · Ctrl+N new query · Esc quit"
            )
        }
    };

    let hints = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, area);
}
