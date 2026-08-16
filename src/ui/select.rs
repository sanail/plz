//! Inline suggestion picker: the list is drawn right under the prompt, with no
//! alternate screen, so the output does not jump around once it exits.

use std::io::{self, IsTerminal, Write};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, style, terminal, QueueableCommand};
use rust_i18n::t;

use crate::suggestion::Suggestion;
use crate::ui::Outcome;

/// Lines taken by the footer under the list: the explanation and the key hints.
const FOOTER_LINES: u16 = 2;

/// Show the list and wait for a choice.
///
/// `buffer_supported` only changes the Tab key's caption: in shells without
/// prompt-buffer insertion it copies the command to the clipboard instead.
///
/// Without a terminal on stderr there is nobody to ask, so the list is printed
/// and the result is `Outcome::Listed` rather than a cancellation.
pub fn select(suggestions: &[Suggestion], buffer_supported: bool) -> Result<Outcome> {
    if suggestions.is_empty() {
        return Ok(Outcome::Cancel);
    }

    // The UI is drawn to stderr: stdout may be redirected to a file or read by
    // a script, where escape sequences have no business being.
    let mut out = io::stderr();
    if !out.is_terminal() {
        // With no terminal there is nothing to pick with: print and leave.
        // Not a cancellation — `plz "task" > file` did exactly what was asked,
        // and reporting 130 there would break the next step of any pipeline.
        print_plain(suggestions);
        return Ok(Outcome::Listed);
    }

    terminal::enable_raw_mode()?;
    let result = interact(&mut out, suggestions, buffer_supported);
    // Leave raw mode whatever happens, including a drawing error: otherwise
    // the terminal is left without echo or line editing.
    let _ = terminal::disable_raw_mode();

    // Clear the UI behind us so the terminal history keeps only the command
    // itself and its output.
    let _ = clear_ui(&mut out, suggestions.len());
    let _ = out.flush();

    result
}

fn interact(
    out: &mut impl Write,
    suggestions: &[Suggestion],
    buffer_supported: bool,
) -> Result<Outcome> {
    let mut selected = 0usize;
    let mut first_draw = true;

    loop {
        draw(out, suggestions, selected, buffer_supported, first_draw)?;
        first_draw = false;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            // Ctrl+C keeps its usual meaning; the guard must stay above the
            // plain 'c' arm below, which is the one that copies.
            KeyCode::Char('c' | 'd') if ctrl => return Ok(Outcome::Cancel),
            KeyCode::Esc | KeyCode::Char('q') => return Ok(Outcome::Cancel),

            KeyCode::Char('c') => return Ok(Outcome::Copy(selected)),
            KeyCode::Enter => return Ok(Outcome::Run(selected)),
            KeyCode::Tab | KeyCode::Char('e') => {
                return Ok(if buffer_supported {
                    Outcome::Buffer(selected)
                } else {
                    Outcome::Copy(selected)
                });
            }

            KeyCode::Up | KeyCode::Char('k') => {
                selected = selected.checked_sub(1).unwrap_or(suggestions.len() - 1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                selected = (selected + 1) % suggestions.len();
            }

            // A digit picks and runs immediately — the fast path the whole
            // tool exists for.
            KeyCode::Char(c @ '1'..='9') => {
                let index = c as usize - '1' as usize;
                if index < suggestions.len() {
                    return Ok(Outcome::Run(index));
                }
            }
            _ => {}
        }
    }
}

fn draw(
    out: &mut impl Write,
    suggestions: &[Suggestion],
    selected: usize,
    buffer_supported: bool,
    first_draw: bool,
) -> Result<()> {
    let total = suggestions.len() as u16 + FOOTER_LINES;
    if !first_draw {
        out.queue(cursor::MoveToColumn(0))?;
        out.queue(cursor::MoveUp(total))?;
    }
    out.queue(cursor::Hide)?;
    out.queue(terminal::Clear(terminal::ClearType::FromCursorDown))?;

    let width = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);

    for (i, suggestion) in suggestions.iter().enumerate() {
        let marker = if i == selected { "❯" } else { " " };
        let command = truncate(&suggestion.one_line(), width.saturating_sub(6));
        let line = format!("{marker} {}  {}", i + 1, command);

        if i == selected {
            out.queue(style::SetAttribute(style::Attribute::Bold))?;
            write!(out, "{line}")?;
            out.queue(style::SetAttribute(style::Attribute::Reset))?;
        } else {
            write!(out, "{line}")?;
        }
        write!(out, "\r\n")?;
    }

    let explanation = suggestions
        .get(selected)
        .map(|s| s.explanation.as_str())
        .unwrap_or("");
    out.queue(style::SetForegroundColor(style::Color::DarkGrey))?;
    write!(
        out,
        "  {}\r\n",
        truncate(explanation, width.saturating_sub(3))
    )?;
    write!(
        out,
        "  {}\r\n",
        truncate(&hints(buffer_supported), width.saturating_sub(3))
    )?;
    out.queue(style::ResetColor)?;
    out.queue(cursor::Show)?;

    out.flush()?;
    Ok(())
}

fn hints(buffer_supported: bool) -> String {
    let tab = if buffer_supported {
        t!("tui.tab_to_edit")
    } else {
        t!("tui.tab_to_copy")
    };
    t!("tui.hints_picker", tab = tab).to_string()
}

/// Erase the UI, leaving the cursor at the start of the line.
fn clear_ui(out: &mut impl Write, count: usize) -> Result<()> {
    let total = count as u16 + FOOTER_LINES;
    out.queue(cursor::MoveToColumn(0))?;
    out.queue(cursor::MoveUp(total))?;
    out.queue(terminal::Clear(terminal::ClearType::FromCursorDown))?;
    out.queue(cursor::Show)?;
    out.flush()?;
    Ok(())
}

/// Non-interactive printing, for pipes and a non-terminal stderr.
fn print_plain(suggestions: &[Suggestion]) {
    for (i, s) in suggestions.iter().enumerate() {
        println!("{}. {}", i + 1, s.command);
        if !s.explanation.is_empty() {
            println!("   {}", s.explanation);
        }
    }
}

/// Truncate a string to the terminal's visible width.
///
/// Counted in characters, not bytes: non-ASCII explanations would otherwise be
/// cut far too early, and slicing on bytes panics on a character boundary.
fn truncate(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    use crate::testutil::locale_guard;

    #[test]
    fn truncate_keeps_short_text_intact() {
        assert_eq!(truncate("ls -la", 20), "ls -la");
    }

    #[test]
    fn truncate_shortens_long_text_with_an_ellipsis() {
        let result = truncate("git log --oneline --graph --decorate --all", 12);
        assert_eq!(result.chars().count(), 12);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        // These characters take more than one byte each; slicing on bytes
        // would both shorten the string too much and panic mid-character.
        let text = "afficher l'espace disque utilisé";
        let result = truncate(text, 10);
        assert_eq!(result.chars().count(), 10);
    }

    #[test]
    fn truncate_handles_zero_width() {
        assert_eq!(truncate("anything at all", 0), "");
    }

    #[test]
    fn hints_mention_the_actual_tab_behaviour() {
        assert!(hints(true).contains("Tab to edit"));
        assert!(hints(false).contains("Tab to copy"));
    }

    #[test]
    fn the_hints_fit_beside_a_truncated_footer() {
        // The footer is cut to the terminal width, so a hint that outgrows an
        // 80-column terminal loses its last keys without saying so. Three
        // columns go to the indent and the ellipsis.
        let _guard = locale_guard();
        for lang in Lang::ALL {
            rust_i18n::set_locale(lang.code());
            for buffered in [true, false] {
                let hint = hints(buffered);
                let width = hint.chars().count();
                assert!(width <= 77, "{lang:?} ({buffered}): {width} chars, {hint}");
            }
        }
    }

    #[test]
    fn hints_advertise_the_key_that_actually_copies() {
        // Ctrl+C cancels; copying is a plain 'c'. A hint saying otherwise
        // would send people to the one key that discards their choice.
        assert!(hints(true).contains("c copy"));
        assert!(!hints(true).contains("Ctrl+C"));
    }

    #[test]
    fn empty_list_cancels_instead_of_hanging() {
        // Otherwise select() would sit in event::read() with an empty screen.
        // Cancel rather than Listed: nothing was printed, so nothing was
        // delivered either, and callers must not treat it as a success.
        assert_eq!(select(&[], true).unwrap(), Outcome::Cancel);
    }

    #[test]
    fn a_non_interactive_stderr_lists_instead_of_cancelling() {
        // `cargo test` captures stderr, so this is the no-terminal path. Under
        // --nocapture in a real terminal it would block on a keypress instead,
        // which is not something a test should do.
        if io::stderr().is_terminal() {
            return;
        }
        let suggestions = [Suggestion::new("ls -la", "list files")];
        assert_eq!(select(&suggestions, true).unwrap(), Outcome::Listed);
    }
}
