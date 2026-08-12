//! Small console prompts: a line, a secret without echo, a y/N confirmation.

use std::io::{self, IsTerminal, Write};

use anyhow::{anyhow, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

/// Read a line; on empty input return `default` when one is given.
pub fn read_line(prompt: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(d) if !d.is_empty() => print!("{prompt} [{d}]: "),
        _ => print!("{prompt}: "),
    }
    io::stdout().flush()?;

    let mut buf = String::new();
    if io::stdin().read_line(&mut buf)? == 0 {
        return Err(anyhow!("input aborted"));
    }
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        return Ok(default.unwrap_or_default().to_string());
    }
    Ok(trimmed.to_string())
}

/// Read a secret without echoing it to the terminal.
///
/// Echo is suppressed through raw mode rather than termios directly, so the
/// code stays the same on Unix and Windows. When input is not a terminal
/// (a pipe, CI) we read a plain line — there is no echo to suppress.
pub fn read_secret(prompt: &str) -> Result<String> {
    print!("{prompt}: ");
    io::stdout().flush()?;

    if !io::stdin().is_terminal() {
        let mut buf = String::new();
        io::stdin().read_line(&mut buf)?;
        println!();
        return Ok(buf.trim().to_string());
    }

    terminal::enable_raw_mode()?;
    let result = read_secret_raw();
    // Always leave raw mode, or the terminal stays without echo and the user
    // has to type `reset` blind.
    let _ = terminal::disable_raw_mode();
    println!();
    result
}

fn read_secret_raw() -> Result<String> {
    let mut secret = String::new();
    loop {
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Enter => return Ok(secret.trim().to_string()),
            KeyCode::Backspace => {
                secret.pop();
            }
            KeyCode::Esc => return Err(anyhow!("input cancelled")),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Err(anyhow!("input aborted"));
            }
            KeyCode::Char(c) => secret.push(c),
            _ => {}
        }
    }
}

/// Ask for confirmation. The default is "no": this is what guards risky
/// commands, and a mistyped key must not launch one.
pub fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N]: ");
    io::stdout().flush()?;

    let mut buf = String::new();
    if io::stdin().read_line(&mut buf)? == 0 {
        return Ok(false);
    }
    Ok(matches!(
        buf.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Pick from a numbered list. Returns the index.
pub fn choose(prompt: &str, options: &[String], default_index: usize) -> Result<usize> {
    for (i, option) in options.iter().enumerate() {
        println!("  {}. {}", i + 1, option);
    }
    loop {
        let raw = read_line(prompt, Some(&(default_index + 1).to_string()))?;
        match raw.parse::<usize>() {
            Ok(n) if n >= 1 && n <= options.len() => return Ok(n - 1),
            _ => println!("Enter a number between 1 and {}.", options.len()),
        }
    }
}
