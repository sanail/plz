use std::fmt;
use std::path::{Path, PathBuf};

/// The shell family. It decides both the command syntax we ask the model for
/// and how we later run the chosen command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
    Nushell,
    PowerShell,
    Cmd,
    /// POSIX-compatible but not bash: sh, dash, ash
    Posix,
    Other,
}

impl ShellKind {
    /// Identify a shell from its executable name.
    ///
    /// The name arrives from several places (process name, `$SHELL`,
    /// `%COMSPEC%`), so strip the path and any `.exe` before comparing.
    pub fn from_program_name(raw: &str) -> Option<Self> {
        let name = raw
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(raw)
            .trim()
            .to_ascii_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);

        match name {
            "zsh" => Some(Self::Zsh),
            "bash" => Some(Self::Bash),
            "fish" => Some(Self::Fish),
            "nu" | "nushell" => Some(Self::Nushell),
            "pwsh" | "powershell" => Some(Self::PowerShell),
            "cmd" => Some(Self::Cmd),
            "sh" | "dash" | "ash" | "ksh" => Some(Self::Posix),
            "elvish" | "xonsh" | "tcsh" | "csh" => Some(Self::Other),
            _ => None,
        }
    }

    /// Human-readable name for the prompt.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
            Self::Nushell => "nushell",
            Self::PowerShell => "PowerShell",
            Self::Cmd => "cmd.exe",
            Self::Posix => "sh",
            Self::Other => "unknown shell",
        }
    }
}

/// A detected shell along with any environment qualifier.
#[derive(Debug, Clone)]
pub struct Shell {
    pub kind: ShellKind,
    /// The name the shell was found under (`zsh`, `pwsh`, ...)
    pub name: String,
    /// A qualifier such as "MSYS2/Git Bash"; it changes paths and quoting
    pub environment: Option<String>,
    /// The absolute path of the shell binary, when we know it. Used to run the
    /// very shell plz was launched from instead of whatever the PATH resolves
    /// to: on Windows several bashes (Cygwin, Git Bash, MSYS2) can share a
    /// PATH and they are not interchangeable.
    pub path: Option<PathBuf>,
}

impl Shell {
    fn new(kind: ShellKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            environment: None,
            path: None,
        }
    }

    fn unknown() -> Self {
        Self {
            kind: ShellKind::Other,
            name: "unknown".into(),
            environment: None,
            path: None,
        }
    }
}

impl fmt::Display for Shell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A recognised shell shows its normalized name; an unknown one shows
        // the name it was found under, since that is at least a hint for the
        // model — "unknown shell" tells it nothing.
        match self.kind {
            ShellKind::Other => write!(f, "{}", self.name)?,
            kind => write!(f, "{}", kind.label())?,
        }
        if let Some(env) = &self.environment {
            write!(f, " ({env})")?;
        }
        Ok(())
    }
}

/// Everything we tell the model about where it is running.
#[derive(Debug, Clone)]
pub struct Context {
    pub os: String,
    pub os_version: Option<String>,
    pub arch: String,
    pub shell: Shell,
    pub cwd: Option<String>,
}

impl Context {
    /// Gather the context of the current run.
    ///
    /// `send_cwd` comes from the config: a working directory path can carry
    /// project and client names, so sending it can be turned off.
    pub fn detect(send_cwd: bool) -> Self {
        Self {
            os: os_name().to_string(),
            os_version: os_version(),
            arch: std::env::consts::ARCH.to_string(),
            shell: detect_shell(),
            cwd: if send_cwd {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string())
            } else {
                None
            },
        }
    }
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

/// The OS version without repeating the OS name.
///
/// `long_os_version()` returns strings like "macOS 26.5.2 Tahoe"; next to the
/// `os` field that would read "macOS macOS 26.5.2 Tahoe" in both the output
/// and the prompt.
fn os_version() -> Option<String> {
    let raw = sysinfo::System::long_os_version().or_else(sysinfo::System::os_version)?;
    Some(strip_os_prefix(&raw, os_name()))
}

fn strip_os_prefix(version: &str, os: &str) -> String {
    let trimmed = version.trim();
    // `get` rather than a plain slice: on a localized Windows the version can
    // start with a multi-byte character, and slicing across its bytes panics.
    if let Some(head) = trimmed.get(..os.len()) {
        if head.eq_ignore_ascii_case(os) {
            let rest = trimmed[os.len()..].trim();
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    trimmed.to_string()
}

/// Detect the shell plz was launched from.
///
/// Sources are tried from most to least reliable:
///
/// 1. `PLZ_SHELL_INTEGRATION` — the wrapper states its own type.
/// 2. Walking up the process tree: `$SHELL` names the *login* shell rather
///    than the one in use, and `$ZSH_VERSION`/`$BASH_VERSION` are not exported
///    to child processes, so we cannot see them.
/// 3. `$SHELL` / `%COMSPEC%` as a last resort.
pub fn detect_shell() -> Shell {
    if let Some(shell) = shell_from_integration() {
        return with_windows_environment(shell);
    }
    if let Some(shell) = shell_from_process_tree() {
        return with_windows_environment(shell);
    }
    with_windows_environment(shell_from_env().unwrap_or_else(Shell::unknown))
}

fn shell_from_integration() -> Option<Shell> {
    let declared = std::env::var("PLZ_SHELL_INTEGRATION").ok()?;
    let kind = ShellKind::from_program_name(&declared)?;
    Some(Shell::new(kind, declared))
}

fn shell_from_process_tree() -> Option<Shell> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());

    let mut pid = Pid::from_u32(std::process::id());
    // Bounded depth: the shell is normally 1-3 steps up, and a cycle in the
    // process tree (possible in theory through PID reuse) must not hang plz.
    for _ in 0..12 {
        let process = system.process(pid)?;
        let parent = process.parent()?;
        let parent_process = system.process(parent)?;
        let name = parent_process.name().to_string_lossy().to_string();
        if let Some(kind) = ShellKind::from_program_name(&name) {
            let mut shell = Shell::new(kind, name);
            shell.path = shell_exe_path(&mut system, parent, kind);
            return Some(shell);
        }
        pid = parent;
    }
    None
}

/// The absolute path of a shell process, when we can trust it.
///
/// Asked for a single pid rather than in the walk above: filling in the exe
/// path costs an extra system call per process, and the walk itself needs only
/// names. A path we cannot vouch for is reported as `None` — the caller then
/// falls back to a PATH lookup, which is how plz always worked.
fn shell_exe_path(
    system: &mut sysinfo::System,
    pid: sysinfo::Pid,
    kind: ShellKind,
) -> Option<PathBuf> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};

    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let path = system.process(pid)?.exe()?;
    (path_matches_kind(path, kind) && path.is_file()).then(|| path.to_path_buf())
}

/// Whether an executable can stand in for the shell we recognised by name.
///
/// The process name and the executable need not agree. A multi-call binary
/// such as busybox runs under the name `sh` but lives at `/bin/busybox`, and it
/// picks its applet from `argv[0]` — so `/bin/busybox -c ...` fails where
/// `sh -c ...` works. Bash running as `sh` is the same trap in reverse: we told
/// the model "sh", and launching `/bin/bash` directly would quietly accept
/// syntax the real shell rejects. In both cases the plain name is the safe one.
fn path_matches_kind(path: &Path, kind: ShellKind) -> bool {
    ShellKind::from_program_name(&path.to_string_lossy()) == Some(kind)
}

fn shell_from_env() -> Option<Shell> {
    // These sources never set `Shell::path`: under Cygwin and MSYS2 `$SHELL`
    // holds a POSIX path (`/bin/bash`) that a native Windows process cannot
    // execute, so only the process tree gives one we can hand to the OS.
    if let Ok(shell) = std::env::var("SHELL") {
        if let Some(kind) = ShellKind::from_program_name(&shell) {
            return Some(Shell::new(kind, shell));
        }
    }
    if let Ok(comspec) = std::env::var("COMSPEC") {
        if let Some(kind) = ShellKind::from_program_name(&comspec) {
            return Some(Shell::new(kind, comspec));
        }
    }
    None
}

/// Flag an MSYS2/Cygwin environment.
///
/// On Windows such a bash looks like any other bash, but it lives in a
/// POSIX-like filesystem (`/c/Users/...`) and does not understand Windows
/// paths in arguments — the model has to be told this explicitly.
fn with_windows_environment(mut shell: Shell) -> Shell {
    if let Ok(msystem) = std::env::var("MSYSTEM") {
        if !msystem.trim().is_empty() {
            shell.environment = Some(format!("MSYS2/Git Bash, MSYSTEM={msystem}"));
            return shell;
        }
    }
    if std::env::var_os("CYGWIN").is_some() {
        shell.environment = Some("Cygwin".into());
    }
    shell
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_plain_names() {
        assert_eq!(ShellKind::from_program_name("zsh"), Some(ShellKind::Zsh));
        assert_eq!(ShellKind::from_program_name("fish"), Some(ShellKind::Fish));
        assert_eq!(ShellKind::from_program_name("dash"), Some(ShellKind::Posix));
    }

    #[test]
    fn strips_unix_paths() {
        assert_eq!(
            ShellKind::from_program_name("/bin/zsh"),
            Some(ShellKind::Zsh)
        );
        assert_eq!(
            ShellKind::from_program_name("/usr/local/bin/bash"),
            Some(ShellKind::Bash)
        );
    }

    #[test]
    fn strips_windows_paths_and_exe_suffix() {
        assert_eq!(
            ShellKind::from_program_name(r"C:\Windows\System32\cmd.exe"),
            Some(ShellKind::Cmd)
        );
        assert_eq!(
            ShellKind::from_program_name("PowerShell.EXE"),
            Some(ShellKind::PowerShell)
        );
        assert_eq!(
            ShellKind::from_program_name("pwsh.exe"),
            Some(ShellKind::PowerShell)
        );
    }

    #[test]
    fn unknown_programs_are_not_shells() {
        // Otherwise the process-tree walk would stop at the very first parent.
        assert_eq!(ShellKind::from_program_name("cargo"), None);
        assert_eq!(ShellKind::from_program_name("/usr/bin/tmux"), None);
        assert_eq!(ShellKind::from_program_name(""), None);
    }

    #[test]
    fn os_version_does_not_repeat_the_os_name() {
        assert_eq!(
            strip_os_prefix("macOS 26.5.2 Tahoe", "macOS"),
            "26.5.2 Tahoe"
        );
        assert_eq!(strip_os_prefix("Windows 11 Pro", "Windows"), "11 Pro");
    }

    #[test]
    fn os_version_without_a_prefix_is_left_alone() {
        assert_eq!(strip_os_prefix("6.8.0-generic", "Linux"), "6.8.0-generic");
        // A string that is only the OS name must not collapse to empty.
        assert_eq!(strip_os_prefix("macOS", "macOS"), "macOS");
    }

    #[test]
    fn a_non_ascii_version_does_not_panic() {
        // A localized Windows can report the product name in its own script.
        // "Windows" is 7 bytes, and byte 7 falls inside a character here — a
        // plain slice would panic on every single run, `plz config show` too.
        assert_eq!(strip_os_prefix("Виндовс 11", "Windows"), "Виндовс 11");
        assert_eq!(strip_os_prefix("日本語版 11", "Windows"), "日本語版 11");
        // Shorter than the OS name: nothing to compare against.
        assert_eq!(strip_os_prefix("Ω", "Windows"), "Ω");
    }

    #[test]
    fn shell_display_includes_environment() {
        let mut shell = Shell::new(ShellKind::Bash, "bash");
        shell.environment = Some("Cygwin".into());
        assert_eq!(shell.to_string(), "bash (Cygwin)");
    }

    #[test]
    fn detection_yields_something_usable() {
        // Under cargo test the parent may not be a shell, so only check that
        // detection never panics and always returns something.
        let ctx = Context::detect(false);
        assert!(!ctx.os.is_empty());
        assert!(!ctx.arch.is_empty());
        assert!(ctx.cwd.is_none(), "send_cwd=false must not send the path");
    }

    #[test]
    fn a_multicall_binary_is_not_mistaken_for_the_shell_it_reports() {
        // Alpine's `sh` is busybox, which dispatches on argv[0]: launching
        // `/bin/busybox -c ...` fails where `sh -c ...` works.
        assert!(!path_matches_kind(
            Path::new("/bin/busybox"),
            ShellKind::Posix
        ));
        // Bash running under the name `sh` would accept syntax that the shell
        // we described to the model does not.
        assert!(!path_matches_kind(Path::new("/bin/bash"), ShellKind::Posix));
        // A path Linux marks as replaced mid-session is no longer executable.
        assert!(!path_matches_kind(
            Path::new("/usr/bin/zsh (deleted)"),
            ShellKind::Zsh
        ));
    }

    #[test]
    fn an_ordinary_shell_path_is_kept() {
        // Debian's `sh` is dash: a different name, but a real shell.
        assert!(path_matches_kind(
            Path::new("/usr/bin/dash"),
            ShellKind::Posix
        ));
        assert!(path_matches_kind(
            Path::new("/opt/homebrew/bin/fish"),
            ShellKind::Fish
        ));
        assert!(path_matches_kind(
            Path::new(r"C:\cygwin64\bin\bash.exe"),
            ShellKind::Bash
        ));
        // pwsh and powershell share a kind, and the same arguments.
        assert!(path_matches_kind(
            Path::new(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            ShellKind::PowerShell
        ));
    }

    #[test]
    fn a_detected_shell_path_is_executable() {
        // Whatever the test runner's parent turns out to be, a path we report
        // must be one we can actually hand to the OS — exec.rs launches it
        // directly instead of searching the PATH.
        if let Some(path) = detect_shell().path {
            assert!(path.is_file(), "{} is not a file", path.display());
            assert!(path.is_absolute(), "{} is not absolute", path.display());
        }
    }

    #[test]
    fn cwd_is_included_when_enabled() {
        let ctx = Context::detect(true);
        assert!(ctx.cwd.is_some());
    }
}
