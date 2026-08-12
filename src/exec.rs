//! Running the chosen command.
//!
//! There are two paths. With the shell wrapper installed (`plz hook <shell>`)
//! we write the command to the file named by `PLZ_OUTPUT_FILE`, and the wrapper
//! runs it in the current shell — so `cd`, `export` and venv activation work.
//! Without the wrapper we spawn a child process: that works out of the box but
//! cannot change the parent shell's state, which is how processes work in the OS.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context as _, Result};

use crate::context::{Shell, ShellKind};

/// What the wrapper should do with the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Run it in the current shell
    Run,
    /// Insert it into the prompt buffer without running it
    Buffer,
}

impl Verb {
    fn as_str(&self) -> &'static str {
        match self {
            Verb::Run => "run",
            Verb::Buffer => "buffer",
        }
    }
}

/// Path to the protocol file when the wrapper is active.
pub fn output_file() -> Option<PathBuf> {
    std::env::var_os("PLZ_OUTPUT_FILE")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Whether the shell wrapper is installed.
pub fn integration_active() -> bool {
    output_file().is_some()
}

/// Whether the shell can insert a command into its prompt buffer.
///
/// zsh has `print -z`, fish has `commandline -r`, PowerShell has
/// `PSConsoleReadLine::Insert`. bash has no real equivalent, and neither does
/// cmd.exe, so there `Tab` degrades into copying to the clipboard.
pub fn supports_buffer(shell: ShellKind) -> bool {
    matches!(
        shell,
        ShellKind::Zsh | ShellKind::Fish | ShellKind::PowerShell
    )
}

/// Hand the command to the wrapper through the protocol file.
///
/// Format: the verb on the first line, the command on the rest.
/// A file rather than stdout, because the selection UI writes escape sequences
/// to the terminal; had they reached `eval`, the shell would run garbage.
pub fn hand_off(verb: Verb, command: &str) -> Result<()> {
    let path =
        output_file().context("the shell wrapper is not active: PLZ_OUTPUT_FILE is unset")?;
    let payload = format!("{}\n{}", verb.as_str(), command.trim());
    fs::write(&path, payload).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

/// Parse the contents of the protocol file.
///
/// In production the shell wrapper does the parsing; this function lets tests
/// check the format from the same side the wrapper sees it from.
#[cfg(test)]
pub fn parse_protocol(payload: &str) -> Option<(Verb, String)> {
    let mut lines = payload.splitn(2, '\n');
    let verb = match lines.next()?.trim() {
        "run" => Verb::Run,
        "buffer" => Verb::Buffer,
        _ => return None,
    };
    let command = lines.next().unwrap_or("").trim().to_string();
    if command.is_empty() {
        return None;
    }
    Some((verb, command))
}

/// Run the command in a child shell, inheriting stdin/stdout/stderr.
///
/// Returns the command's exit code so that `plz` exits with the same one:
/// otherwise `plz "..." && next-step` would behave incorrectly.
pub fn run_in_child_shell(shell: &Shell, command: &str) -> Result<i32> {
    let (program, args) = child_shell_invocation(shell.kind);

    let mut cmd = Command::new(program);
    cmd.args(args).arg(command);

    let status = cmd
        .status()
        .with_context(|| format!("could not launch `{program}`"))?;

    // A signal (Ctrl+C inside the command) yields no exit code; report the
    // conventional 130, which is what shells themselves do.
    Ok(status.code().unwrap_or(130))
}

/// How to run a one-off command in a given shell.
fn child_shell_invocation(kind: ShellKind) -> (&'static str, &'static [&'static str]) {
    match kind {
        ShellKind::Zsh => ("zsh", &["-c"]),
        ShellKind::Bash => ("bash", &["-c"]),
        ShellKind::Fish => ("fish", &["-c"]),
        ShellKind::Nushell => ("nu", &["-c"]),
        // -NoProfile: a user profile may print banners and change the
        // environment, and we want a predictable one-off run.
        ShellKind::PowerShell => ("powershell", &["-NoProfile", "-Command"]),
        ShellKind::Cmd => ("cmd", &["/C"]),
        ShellKind::Posix | ShellKind::Other => {
            if cfg!(windows) {
                ("cmd", &["/C"])
            } else {
                ("sh", &["-c"])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::env_guard;

    #[test]
    fn protocol_round_trips() {
        let payload = "run\ngit status --short";
        let (verb, command) = parse_protocol(payload).unwrap();
        assert_eq!(verb, Verb::Run);
        assert_eq!(command, "git status --short");
    }

    #[test]
    fn protocol_preserves_multiline_commands() {
        let payload = "buffer\ncd /tmp\nls -la";
        let (verb, command) = parse_protocol(payload).unwrap();
        assert_eq!(verb, Verb::Buffer);
        assert_eq!(command, "cd /tmp\nls -la");
    }

    #[test]
    fn empty_payload_means_the_user_cancelled() {
        assert!(parse_protocol("").is_none());
        assert!(parse_protocol("run\n").is_none());
        assert!(parse_protocol("run\n   ").is_none());
    }

    #[test]
    fn unknown_verb_is_rejected() {
        // The wrapper must do nothing with an unknown verb.
        assert!(parse_protocol("destroy\nrm -rf /").is_none());
    }

    #[test]
    fn hand_off_writes_a_parseable_file() {
        let _guard = env_guard();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out");
        std::env::set_var("PLZ_OUTPUT_FILE", &path);

        hand_off(Verb::Run, "  echo hi  ").unwrap();
        let written = fs::read_to_string(&path).unwrap();
        let (verb, command) = parse_protocol(&written).unwrap();
        assert_eq!(verb, Verb::Run);
        assert_eq!(command, "echo hi");

        std::env::remove_var("PLZ_OUTPUT_FILE");
    }

    #[test]
    fn hand_off_fails_clearly_without_integration() {
        let _guard = env_guard();
        std::env::remove_var("PLZ_OUTPUT_FILE");
        let err = hand_off(Verb::Run, "ls").unwrap_err().to_string();
        assert!(err.contains("PLZ_OUTPUT_FILE"));
    }

    #[test]
    fn buffer_support_matches_shell_capabilities() {
        assert!(supports_buffer(ShellKind::Zsh));
        assert!(supports_buffer(ShellKind::Fish));
        assert!(supports_buffer(ShellKind::PowerShell));
        // bash has no `print -z` equivalent, and neither does cmd.exe.
        assert!(!supports_buffer(ShellKind::Bash));
        assert!(!supports_buffer(ShellKind::Cmd));
    }

    #[test]
    fn invocations_are_correct_per_shell() {
        assert_eq!(child_shell_invocation(ShellKind::Zsh), ("zsh", &["-c"][..]));
        assert_eq!(child_shell_invocation(ShellKind::Cmd), ("cmd", &["/C"][..]));
        assert_eq!(
            child_shell_invocation(ShellKind::PowerShell),
            ("powershell", &["-NoProfile", "-Command"][..])
        );
    }

    #[test]
    #[cfg(unix)]
    fn child_shell_runs_the_command_and_returns_its_code() {
        let shell = Shell {
            kind: ShellKind::Posix,
            name: "sh".into(),
            environment: None,
        };
        assert_eq!(run_in_child_shell(&shell, "exit 0").unwrap(), 0);
        assert_eq!(run_in_child_shell(&shell, "exit 7").unwrap(), 7);
    }
}
