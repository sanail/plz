//! Small console prompts: a line, a secret without echo, a y/N confirmation.

use std::io::{self, IsTerminal, Write};

use anyhow::{anyhow, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use rust_i18n::t;

use crate::i18n::{self, Lang};

/// Read a line; on empty input return `default` when one is given.
pub fn read_line(prompt: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(d) if !d.is_empty() => print!("{prompt} [{d}]: "),
        _ => print!("{prompt}: "),
    }
    io::stdout().flush()?;

    let mut buf = String::new();
    if io::stdin().read_line(&mut buf)? == 0 {
        return Err(anyhow!("{}", t!("errors.input_aborted")));
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
            KeyCode::Esc => return Err(anyhow!("{}", t!("errors.input_cancelled"))),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Err(anyhow!("{}", t!("errors.input_aborted")));
            }
            KeyCode::Char(c) => secret.push(c),
            _ => {}
        }
    }
}

/// Ask for confirmation. The default is "no": this is what guards risky
/// commands, and a mistyped key must not launch one.
pub fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [{}]: ", t!("input.confirm_hint"));
    io::stdout().flush()?;

    let mut buf = String::new();
    if io::stdin().read_line(&mut buf)? == 0 {
        return Ok(false);
    }
    Ok(is_affirmative(&buf, i18n::current()))
}

/// Whether an answer means yes.
///
/// The English `y`/`yes` is accepted in every language on top of the local
/// spelling: the keyboard layout in front of the user is not always the one
/// their language implies, and `y` means nothing else in any of them. Unaccented
/// forms are listed too, because `sí` is routinely typed `si`.
fn is_affirmative(answer: &str, lang: Lang) -> bool {
    // to_lowercase rather than to_ascii_lowercase: `Д` is not ASCII, and the
    // ASCII form would leave it uppercase and reject a perfectly good yes.
    let answer = answer.trim().to_lowercase();
    let local: &[&str] = match lang {
        Lang::En => &[],
        Lang::Ru => &["д", "да"],
        Lang::Es => &["s", "si", "sí"],
        Lang::Fr => &["o", "oui"],
        Lang::De => &["j", "ja"],
    };
    matches!(answer.as_str(), "y" | "yes") || local.contains(&answer.as_str())
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
            _ => println!("{}", t!("input.enter_a_number", max = options.len())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::locale_guard;

    #[test]
    fn the_english_yes_is_accepted_in_every_language() {
        // The layout in front of the user is not always the one their language
        // implies, and `y` is not a word in any of the others.
        for lang in Lang::ALL {
            assert!(is_affirmative("y", lang), "{lang:?}");
            assert!(is_affirmative(" YES \n", lang), "{lang:?}");
        }
    }

    #[test]
    fn a_localized_yes_is_accepted_in_its_own_language() {
        // The uppercase Cyrillic form is the one to_ascii_lowercase used to
        // leave alone and reject; `si` is `sí` typed without the accent.
        assert!(is_affirmative("Д", Lang::Ru));
        assert!(is_affirmative("да", Lang::Ru));
        assert!(is_affirmative("si", Lang::Es));
        assert!(is_affirmative("SÍ", Lang::Es));
        assert!(is_affirmative("oui", Lang::Fr));
        assert!(is_affirmative("Ja", Lang::De));
    }

    #[test]
    fn a_yes_from_another_language_is_not_accepted() {
        // Otherwise `o` — French for yes — would confirm for a Spanish user,
        // who may well have typed it meaning nothing of the sort.
        assert!(!is_affirmative("oui", Lang::Es));
        assert!(!is_affirmative("да", Lang::De));
        assert!(!is_affirmative("ja", Lang::Ru));
    }

    #[test]
    fn everything_else_is_a_no() {
        for lang in Lang::ALL {
            for answer in ["", "  ", "n", "no", "нет", "non", "nein", "\n", "yep"] {
                assert!(!is_affirmative(answer, lang), "{lang:?}/{answer}");
            }
        }
    }

    #[test]
    fn the_hint_offers_a_letter_the_prompt_actually_accepts() {
        // Showing [д/Н] while accepting only `y` is the regression this guards:
        // the hint and the matcher live in different files.
        let _guard = locale_guard();
        for lang in Lang::ALL {
            rust_i18n::set_locale(lang.code());
            let hint = t!("input.confirm_hint");
            let yes = hint.split('/').next().unwrap();
            assert!(is_affirmative(yes, lang), "{lang:?}: {hint}");
        }
    }
}
