//! Writing the wrapper into the shell's startup file (`plz hook <shell> --install`).
//!
//! The startup file gets one line — `eval "$(plz hook zsh)"` and its
//! equivalents — so the wrapper is regenerated at every shell start instead of
//! sitting there as a stale copy.
//!
//! Nothing here touches disk or the registry without an explicit `y`. That
//! matters most on Windows, where a working wrapper also needs the execution
//! policy relaxed, and changing a security setting behind the user's back is
//! not something a command-line helper gets to do.

use anyhow::{anyhow, Context as _, Result};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use rust_i18n::t;

use crate::cli::Shell;
use crate::context::{self, ShellKind};
use crate::input;
use crate::integration;
use crate::winpath;

/// The policy plz offers to set: local scripts run, downloaded ones still need
/// a signature. It is per-user and needs no administrator rights.
const WANTED_POLICY: &str = "RemoteSigned";

pub fn run(shell: Shell, assume_yes: bool) -> Result<()> {
    let line = integration::startup_line(shell)
        .ok_or_else(|| anyhow!("{}", integration::install_hint(shell)))?;

    // Resolved before anything is printed: for PowerShell this asks the shell
    // itself, and a failure there has nothing to do with consent.
    let target = startup_file(shell)?;

    let added = add_line(&target, line, assume_yes)?;

    if shell == Shell::Powershell {
        // Pointless to nag about the policy when the line was never written.
        if added.file_holds_the_line() {
            ensure_execution_policy(&target, assume_yes)?;
        }
    }
    Ok(())
}

/// What `add_line` did, which decides whether the follow-up steps make sense.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Appended,
    AlreadyPresent,
    Declined,
}

impl Outcome {
    fn file_holds_the_line(self) -> bool {
        matches!(self, Self::Appended | Self::AlreadyPresent)
    }
}

/// Append the line after asking. A refusal leaves the file untouched — and, if
/// it did not exist, uncreated.
fn add_line(target: &Path, line: &str, assume_yes: bool) -> Result<Outcome> {
    let existing = match std::fs::read_to_string(target) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err)
                .with_context(|| t!("errors.could_not_read", path = target.display()).to_string())
        }
    };

    if existing.as_deref().is_some_and(|text| contains(text, line)) {
        println!(
            "{}",
            t!("install.already_installed", path = target.display())
        );
        return Ok(Outcome::AlreadyPresent);
    }

    println!("{}", t!("install.file", path = target.display()));
    println!("{}", t!("install.line", line = line));
    if !assume_yes && !input::confirm(&t!("install.add_it"))? {
        println!("{}", t!("install.nothing_written"));
        return Ok(Outcome::Declined);
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| t!("install.could_not_create", path = parent.display()).to_string())?;
    }

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(target)
        .with_context(|| t!("install.could_not_open", path = target.display()).to_string())?;
    // UTF-8 without a BOM. The line is pure ASCII, so every shell that reads
    // the file — including PowerShell 5.1, which treats a BOM-less file as
    // ANSI — sees the same bytes.
    file.write_all(appendix(existing.as_deref().unwrap_or(""), line).as_bytes())
        .with_context(|| t!("install.could_not_write_to", path = target.display()).to_string())?;

    println!("{}", t!("install.added", path = target.display()));
    println!("{}", t!("install.open_a_new_shell"));
    Ok(Outcome::Appended)
}

/// Whether the startup file already has the line, as its own statement.
///
/// A commented-out copy does not count, and neither does the line appearing
/// inside a longer one — both would leave the wrapper uninstalled.
fn contains(contents: &str, line: &str) -> bool {
    contents.lines().any(|candidate| candidate.trim() == line)
}

/// The text to append, with the newline the existing content is missing.
fn appendix(existing: &str, line: &str) -> String {
    if existing.is_empty() || existing.ends_with('\n') {
        format!("{line}\n")
    } else {
        format!("\n{line}\n")
    }
}

/// Where the line goes for each shell.
fn startup_file(shell: Shell) -> Result<PathBuf> {
    match shell {
        Shell::Zsh => {
            // ZDOTDIR moves the whole zsh configuration; honouring it is the
            // difference between installing and writing to a file zsh never reads.
            match env_dir("ZDOTDIR") {
                Some(dir) => Ok(dir.join(".zshrc")),
                None => Ok(home()?.join(".zshrc")),
            }
        }
        Shell::Bash => Ok(home()?.join(".bashrc")),
        Shell::Fish => {
            let config = match env_dir("XDG_CONFIG_HOME") {
                Some(dir) => dir,
                None => home()?.join(".config"),
            };
            // conf.d is sourced automatically, so plz gets a file of its own
            // instead of editing the user's config.fish.
            Ok(config.join("fish").join("conf.d").join("plz.fish"))
        }
        Shell::Powershell => powershell_profile(),
        Shell::Cmd => Err(anyhow!("{}", integration::install_hint(shell))),
    }
}

/// The home directory of the shell we are installing into.
///
/// `HOME` comes first, and on Windows that is not a matter of taste: the
/// `directories` crate asks the OS there (`FOLDERID_Profile`, i.e.
/// `C:\Users\name`) and ignores `HOME` by design, while a Cygwin bash reads
/// `/home/name/.bashrc`. Writing to the file the shell never opens looks like a
/// successful install and does nothing at all.
fn home() -> Result<PathBuf> {
    if let Some(home) = env_dir("HOME") {
        return Ok(home);
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .ok_or_else(|| anyhow!("{}", t!("install.no_home_directory")))
}

/// A directory named by an environment variable, in a form we can write to.
///
/// Under Cygwin these hold POSIX paths that reach a native process untranslated,
/// so `/home/name` would be resolved against the current drive as `C:\home\name`;
/// `winpath::to_native` asks `cygpath` for the directory the shell actually meant.
fn env_dir(var: &str) -> Option<PathBuf> {
    let value = std::env::var_os(var).filter(|value| !value.is_empty())?;
    match winpath::to_native(&value.to_string_lossy()) {
        Some(native) => Some(native),
        None => Some(PathBuf::from(value)),
    }
}

/// Which PowerShell to talk to.
///
/// Windows PowerShell 5.1 and PowerShell 7 keep separate `$PROFILE` paths *and*
/// separate execution-policy registry keys, so guessing means fixing the one
/// the user is not sitting in. The shell plz was launched from is the answer
/// whenever there is one.
///
/// Cached: answering it walks the process table, and both the profile lookup
/// and the policy check need it.
fn powershell_binary() -> &'static str {
    static BINARY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BINARY.get_or_init(|| {
        let detected = context::detect_shell();
        if detected.kind == ShellKind::PowerShell {
            return detected.name;
        }
        if Command::new("pwsh").arg("-Help").output().is_ok() {
            return "pwsh".to_string();
        }
        "powershell".to_string()
    })
}

/// Ask PowerShell where its profile is, rather than reconstructing the path.
fn powershell_profile() -> Result<PathBuf> {
    let binary = powershell_binary();
    let hint = integration::install_hint(Shell::Powershell);
    let path = powershell_query(binary, "$PROFILE").map_err(|err| {
        anyhow!(
            "{}",
            t!("install.run_from_powershell", err = err, hint = hint)
        )
    })?;
    if path.is_empty() {
        return Err(anyhow!(
            "{}",
            t!("install.no_profile_path", binary = binary, hint = hint)
        ));
    }
    Ok(PathBuf::from(path))
}

/// Run a one-liner in PowerShell and return its trimmed output.
///
/// -NoProfile because the profile is the thing being repaired, and
/// -NonInteractive so a misconfigured host cannot sit waiting for a keypress.
fn powershell_query(binary: &str, command: &str) -> Result<String> {
    let output = Command::new(binary)
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .output()
        .with_context(|| t!("install.could_not_launch", binary = binary).to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "{}",
            t!(
                "install.command_failed",
                binary = binary,
                command = command,
                stderr = stderr.trim()
            )
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Offer to lift the execution policy when it stops PowerShell loading the
/// profile at all — the default state on Windows client, where the line we just
/// wrote would otherwise never run.
fn ensure_execution_policy(profile: &Path, assume_yes: bool) -> Result<()> {
    let binary = powershell_binary();
    let policy = match powershell_query(binary, PERSISTENT_POLICY) {
        Ok(policy) => settled_policy(&policy),
        // Not being able to read the policy is not a failure to install: the
        // line is in the profile either way.
        Err(err) => {
            eprintln!("{}", t!("install.policy_unreadable", err = err));
            return Ok(());
        }
    };

    if !policy_blocks_profile(&policy) {
        return Ok(());
    }

    println!();
    println!(
        "{}",
        t!(
            "install.policy_blocks_profile",
            path = profile.display(),
            policy = policy
        )
    );
    println!(
        "{}",
        t!("install.policy_explanation", policy = WANTED_POLICY)
    );

    let prompt = t!("install.set_policy_now", policy = WANTED_POLICY);
    if !assume_yes && !input::confirm(&prompt)? {
        println!("{}", t!("install.policy_left_unchanged"));
        println!("    Set-ExecutionPolicy {WANTED_POLICY} -Scope CurrentUser");
        return Ok(());
    }

    powershell_query(
        binary,
        &format!("Set-ExecutionPolicy -ExecutionPolicy {WANTED_POLICY} -Scope CurrentUser -Force"),
    )?;

    // Read it back: a Group Policy at machine or user scope overrides
    // CurrentUser, and the command above reports success regardless.
    let now = settled_policy(&powershell_query(binary, PERSISTENT_POLICY)?);
    if policy_blocks_profile(&now) {
        return Err(anyhow!("{}", t!("install.policy_enforced", policy = now)));
    }
    println!("{}", t!("install.policy_now_set", policy = now));
    Ok(())
}

/// The policy a *new* PowerShell session will start under.
///
/// Plain `Get-ExecutionPolicy` answers for the session plz was launched from,
/// and the Process scope it reports is inherited by us as a child. A terminal
/// started with `-ExecutionPolicy Bypass` would therefore look fine while every
/// ordinary window stays Restricted — the profile line would keep not running.
/// Dropping the Process scope and taking the first setting that survives leaves
/// exactly what governs the windows the user opens tomorrow.
///
/// `Get-ExecutionPolicy -List` is already in precedence order, so "first
/// defined" is the answer. All of them undefined prints nothing.
const PERSISTENT_POLICY: &str = "(Get-ExecutionPolicy -List | \
     Where-Object { $_.Scope -ne 'Process' -and $_.ExecutionPolicy -ne 'Undefined' } | \
     Select-Object -First 1 -ExpandProperty ExecutionPolicy)";

/// Fill in the platform default when no scope sets a policy.
///
/// Windows clients fall back to Restricted; PowerShell on macOS and Linux does
/// not enforce policies at all and reports Unrestricted.
fn settled_policy(reported: &str) -> String {
    if reported.trim().is_empty() {
        let default = if cfg!(windows) {
            "Restricted"
        } else {
            "Unrestricted"
        };
        return default.to_string();
    }
    reported.trim().to_string()
}

/// Whether a policy stops PowerShell from running the profile.
///
/// Anything unrecognised counts as fine: a policy plz does not know is not a
/// reason to push a security change on the user.
fn policy_blocks_profile(policy: &str) -> bool {
    matches!(
        policy.trim().to_ascii_lowercase().as_str(),
        // AllSigned blocks the profile too — it is unsigned like any other
        // script. Undefined at the effective level means the Windows client
        // default, which is Restricted.
        "restricted" | "allsigned" | "undefined"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::env_guard;

    #[test]
    fn policies_that_stop_the_profile_loading() {
        for policy in ["Restricted", "AllSigned", "Undefined"] {
            assert!(policy_blocks_profile(policy), "{policy}");
        }
    }

    #[test]
    fn policies_that_let_the_profile_load() {
        for policy in ["RemoteSigned", "Unrestricted", "Bypass"] {
            assert!(!policy_blocks_profile(policy), "{policy}");
        }
    }

    #[test]
    fn no_scope_set_means_the_platform_default() {
        // Every scope Undefined is the out-of-the-box state on Windows, and it
        // is exactly the case that has to be caught.
        let fallback = settled_policy("");
        assert_eq!(
            policy_blocks_profile(&fallback),
            cfg!(windows),
            "{fallback}"
        );
        assert_eq!(settled_policy("  \r\n "), fallback);
    }

    #[test]
    fn a_scope_that_is_set_wins_over_the_default() {
        assert_eq!(settled_policy("RemoteSigned\r\n"), "RemoteSigned");
        assert_eq!(settled_policy("AllSigned"), "AllSigned");
    }

    #[test]
    fn the_policy_query_ignores_the_inherited_process_scope() {
        // A terminal launched with -ExecutionPolicy Bypass hands that scope to
        // plz as a child, and it says nothing about ordinary new windows.
        assert!(PERSISTENT_POLICY.contains("$_.Scope -ne 'Process'"));
        assert!(PERSISTENT_POLICY.contains("Select-Object -First 1"));
    }

    #[test]
    fn the_policy_reply_arrives_from_a_child_process() {
        // Trailing newline and host-dependent casing come with it.
        assert!(policy_blocks_profile("restricted\r\n"));
        assert!(!policy_blocks_profile("  REMOTESIGNED \n"));
        // Something we cannot classify is not grounds for a security prompt.
        assert!(!policy_blocks_profile(""));
        assert!(!policy_blocks_profile("SomethingNew"));
    }

    #[test]
    fn an_existing_line_is_recognised() {
        let line = integration::startup_line(Shell::Zsh).unwrap();
        assert!(contains(&format!("# comments\n{line}\n"), line));
        // Indented by an `if` block, but still a live statement.
        assert!(contains(&format!("if true; then\n  {line}\nfi\n"), line));
    }

    #[test]
    fn a_line_that_does_not_run_does_not_count() {
        let line = integration::startup_line(Shell::Zsh).unwrap();
        // Commented out: reinstalling has to add a working copy.
        assert!(!contains(&format!("# {line}\n"), line));
        // Part of a longer statement, so appending is still correct.
        assert!(!contains(&format!("{line} # trailing\n"), line));
        assert!(!contains("", line));
    }

    #[test]
    fn the_appended_text_never_joins_the_previous_line() {
        assert_eq!(appendix("", "L"), "L\n");
        assert_eq!(appendix("prev\n", "L"), "L\n");
        // No trailing newline: without one of ours the line would be glued on.
        assert_eq!(appendix("prev", "L"), "\nL\n");
    }

    #[test]
    fn declining_leaves_no_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        // Nested, so a stray create_dir_all would show up too.
        let target = dir.path().join("nested").join(".zshrc");

        // confirm() reads stdin, which is not a terminal under `cargo test`, so
        // it sees EOF and returns the default: no.
        let outcome = add_line(&target, "eval line", false).unwrap();
        assert_eq!(outcome, Outcome::Declined);
        assert!(!target.exists());
        assert!(!target.parent().unwrap().exists());
    }

    #[test]
    fn installing_twice_appends_once() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("conf.d").join("plz.fish");
        let line = integration::startup_line(Shell::Fish).unwrap();

        assert_eq!(add_line(&target, line, true).unwrap(), Outcome::Appended);
        assert_eq!(
            add_line(&target, line, true).unwrap(),
            Outcome::AlreadyPresent
        );

        let contents = std::fs::read_to_string(&target).unwrap();
        assert_eq!(contents.matches(line).count(), 1, "{contents:?}");
    }

    #[test]
    fn appending_preserves_what_was_there() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".bashrc");
        std::fs::write(&target, "export EDITOR=vim").unwrap();

        add_line(&target, "line", true).unwrap();

        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "export EDITOR=vim\nline\n"
        );
    }

    #[test]
    fn every_installable_shell_resolves_a_startup_file() {
        // PowerShell is left out: resolving it launches a shell, which is not
        // something a unit test should depend on.
        for shell in [Shell::Zsh, Shell::Bash, Shell::Fish] {
            let path = startup_file(shell).unwrap_or_else(|e| panic!("{shell:?}: {e}"));
            assert!(path.is_absolute(), "{shell:?}: {}", path.display());
        }
    }

    #[test]
    fn home_decides_where_the_startup_file_goes() {
        // On Windows `directories` answers with the profile the OS knows about
        // (`C:\Users\name`) and ignores HOME, but a Cygwin bash reads
        // `$HOME/.bashrc` and nothing else. HOME is the shell's own answer, so
        // it wins everywhere.
        let _guard = env_guard();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        std::env::remove_var("ZDOTDIR");

        assert_eq!(
            startup_file(Shell::Bash).unwrap(),
            dir.path().join(".bashrc")
        );
        assert_eq!(startup_file(Shell::Zsh).unwrap(), dir.path().join(".zshrc"));

        std::env::remove_var("HOME");
    }

    #[test]
    fn zdotdir_still_wins_for_zsh() {
        let _guard = env_guard();
        let home = tempfile::tempdir().unwrap();
        let zdotdir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());
        std::env::set_var("ZDOTDIR", zdotdir.path());

        assert_eq!(
            startup_file(Shell::Zsh).unwrap(),
            zdotdir.path().join(".zshrc")
        );

        std::env::remove_var("ZDOTDIR");
        std::env::remove_var("HOME");
    }

    #[test]
    fn cmd_cannot_be_installed_and_says_why() {
        let err = startup_file(Shell::Cmd).unwrap_err().to_string();
        assert!(err.contains("cmd.exe"), "{err}");
    }
}
