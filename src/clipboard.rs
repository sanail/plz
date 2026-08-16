//! Copying the command to the clipboard.
//!
//! Two independent mechanisms, because neither covers every case:
//!
//! * `arboard` — the system clipboard. Works well on macOS and Windows, where
//!   the OS owns the contents.
//! * **OSC 52** — an escape sequence asking the terminal itself to put the text
//!   on the clipboard. Works where no system clipboard is visible: over SSH,
//!   in a container, in a headless environment.
//!
//! On Linux the order is reversed: under X11/Wayland the owning process holds
//! the contents, so the clipboard would empty when `plz` exits unless a
//! clipboard manager picked it up. There we try OSC 52 first, since it outlives
//! the process.

use std::io::{IsTerminal, Write};

use anyhow::{anyhow, Result};
use rust_i18n::t;

/// How the text reached the clipboard. Shown to the user, because it decides
/// where the command will actually be available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    System,
    Osc52,
}

impl Method {
    pub fn describe(&self) -> String {
        match self {
            Method::System => t!("ui.copied_to_clipboard").to_string(),
            Method::Osc52 => t!("ui.copied_via_terminal").to_string(),
        }
    }
}

/// Put the text on the clipboard.
pub fn copy(text: &str) -> Result<Method> {
    let mut errors = Vec::new();

    for attempt in attempt_order() {
        match attempt {
            Method::System => match copy_via_system(text) {
                Ok(()) => return Ok(Method::System),
                Err(err) => errors.push(format!("system clipboard: {err}")),
            },
            Method::Osc52 => match copy_via_osc52(text) {
                Ok(()) => return Ok(Method::Osc52),
                Err(err) => errors.push(format!("OSC 52: {err}")),
            },
        }
    }

    // The mechanism labels inside `details` stay English: they name a system
    // clipboard API and an escape sequence, not something to read as prose.
    Err(anyhow!(
        "{}",
        t!("errors.clipboard_failed", details = errors.join("; "))
    ))
}

fn attempt_order() -> [Method; 2] {
    if cfg!(target_os = "linux") {
        [Method::Osc52, Method::System]
    } else {
        [Method::System, Method::Osc52]
    }
}

fn copy_via_system(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    clipboard.set_text(text.to_string())?;
    Ok(())
}

/// Send OSC 52 to the terminal.
///
/// We write to stderr rather than stdout: stdout may be redirected to a file,
/// where the escape sequence would simply land instead of reaching the
/// terminal.
fn copy_via_osc52(text: &str) -> Result<()> {
    let mut out = std::io::stderr();
    if !out.is_terminal() {
        return Err(anyhow!("{}", t!("errors.stderr_not_a_terminal")));
    }
    // `c` is the main system clipboard (CLIPBOARD), not the selection.
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()))?;
    out.flush()?;
    Ok(())
}

/// Base64 without a dependency: needed exactly once, and only for encoding.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        encoded.push(ALPHABET[(triple >> 18 & 0x3f) as usize] as char);
        encoded.push(ALPHABET[(triple >> 12 & 0x3f) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6 & 0x3f) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_shell_commands() {
        assert_eq!(base64(b"ls -la"), "bHMgLWxh");
        assert_eq!(
            base64("ps aux | awk '{print $2}'".as_bytes()),
            "cHMgYXV4IHwgYXdrICd7cHJpbnQgJDJ9Jw=="
        );
    }

    #[test]
    fn base64_handles_non_ascii() {
        // Explanations and paths can be non-ASCII; what matters is that we
        // encode UTF-8 bytes.
        assert_eq!(base64("café".as_bytes()), "Y2Fmw6k=");
    }

    #[test]
    fn linux_prefers_osc52() {
        // On Linux the system clipboard dies with the process, so the option
        // that outlives the exit has to come first.
        let order = attempt_order();
        if cfg!(target_os = "linux") {
            assert_eq!(order[0], Method::Osc52);
        } else {
            assert_eq!(order[0], Method::System);
        }
    }

    #[test]
    fn methods_describe_themselves_differently() {
        assert_ne!(Method::System.describe(), Method::Osc52.describe());
    }

    #[test]
    fn a_description_is_prose_rather_than_a_catalogue_key() {
        // A key that is missing or misspelled comes back from `t!` as the key
        // itself, which reads as gibberish but breaks nothing else.
        for method in [Method::System, Method::Osc52] {
            let text = method.describe();
            assert!(!text.starts_with("ui."), "{method:?}: {text}");
        }
    }
}
