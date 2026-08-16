//! Heuristics for commands worth confirming before they run.
//!
//! These are heuristics, not a sandbox: the list is knowingly incomplete and
//! trivial to slip past. The point is to catch the classic case where someone
//! hits the wrong digit in the suggestion list, not to defend against a
//! malicious model.

use rust_i18n::t;

/// Why a command was flagged.
///
/// An identity rather than the sentence itself: the sentence is translated, and
/// callers compare and match on the rule, not on its wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    RecursiveDelete,
    DdOverwrite,
    Mkfs,
    BlockDevice,
    CurlPipeShell,
    ForkBomb,
    RecklessChmod,
    Shutdown,
    ForcePush,
    DestructiveSql,
    PowerShellRecursiveDelete,
    PowerShellFormat,
    CmdRecursiveDelete,
    DiskPartitioning,
}

impl Reason {
    /// The sentence the warning shows.
    pub fn text(self) -> String {
        let key = match self {
            Reason::RecursiveDelete => "safety.recursive_delete",
            Reason::DdOverwrite => "safety.dd_overwrite",
            Reason::Mkfs => "safety.mkfs",
            Reason::BlockDevice => "safety.block_device",
            Reason::CurlPipeShell => "safety.curl_pipe_shell",
            Reason::ForkBomb => "safety.fork_bomb",
            Reason::RecklessChmod => "safety.reckless_chmod",
            Reason::Shutdown => "safety.shutdown",
            Reason::ForcePush => "safety.force_push",
            Reason::DestructiveSql => "safety.destructive_sql",
            Reason::PowerShellRecursiveDelete => "safety.powershell_recursive_delete",
            Reason::PowerShellFormat => "safety.powershell_format",
            Reason::CmdRecursiveDelete => "safety.cmd_recursive_delete",
            Reason::DiskPartitioning => "safety.disk_partitioning",
        };
        t!(key).to_string()
    }
}

/// How risky a command looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Risk {
    Safe,
    /// A rule matched; which one decides what the warning says.
    Dangerous(Reason),
}

impl Risk {
    pub fn reason(&self) -> Option<Reason> {
        match self {
            Risk::Dangerous(reason) => Some(*reason),
            Risk::Safe => None,
        }
    }
}

/// A rule: which reason it reports, plus a predicate over the normalized
/// command.
type Rule = (Reason, fn(&str) -> bool);

const RULES: &[Rule] = &[
    (Reason::RecursiveDelete, is_dangerous_rm),
    (Reason::DdOverwrite, is_dangerous_dd),
    (Reason::Mkfs, is_mkfs),
    (Reason::BlockDevice, writes_to_block_device),
    (Reason::CurlPipeShell, is_curl_pipe_shell),
    (Reason::ForkBomb, is_fork_bomb),
    (Reason::RecklessChmod, is_reckless_chmod),
    (Reason::Shutdown, is_shutdown),
    (Reason::ForcePush, is_force_push),
    (Reason::DestructiveSql, is_destructive_sql),
    (
        Reason::PowerShellRecursiveDelete,
        is_powershell_recursive_delete,
    ),
    (Reason::PowerShellFormat, is_powershell_format),
    (Reason::CmdRecursiveDelete, is_cmd_recursive_delete),
    (Reason::DiskPartitioning, is_disk_partitioning),
];

/// Classify a command. The first matching rule wins.
pub fn classify(command: &str) -> Risk {
    let normalized = normalize(command);
    for (reason, matches) in RULES {
        if matches(&normalized) {
            return Risk::Dangerous(*reason);
        }
    }
    Risk::Safe
}

/// Normalize a command for matching: lowercase, whitespace collapsed.
fn normalize(command: &str) -> String {
    command
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split a compound command into its individual invocations.
///
/// Without this, `cd /tmp && rm -rf /` would slip past because the command
/// starts with a harmless `cd`.
fn segments(command: &str) -> Vec<&str> {
    command
        .split([';', '\n'])
        .flat_map(|s| s.split("&&"))
        .flat_map(|s| s.split("||"))
        .flat_map(|s| s.split('|'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// The program name without its path: `/bin/rm` becomes `rm`.
fn program(segment: &str) -> Option<&str> {
    let first = segment.split_whitespace().next()?;
    // sudo is transparent: what follows it decides the risk.
    Some(first.rsplit('/').next().unwrap_or(first))
}

/// A segment's tokens with leading `sudo`/`doas` and env assignments removed.
fn args(segment: &str) -> Vec<&str> {
    let mut tokens: Vec<&str> = segment.split_whitespace().collect();
    while let Some(first) = tokens.first() {
        let name = first.rsplit('/').next().unwrap_or(first);
        if name == "sudo" || name == "doas" || first.contains('=') {
            tokens.remove(0);
        } else {
            break;
        }
    }
    tokens
}

/// The program a segment runs: no leading `sudo`, no directory, no `.exe`.
fn command_name(segment: &str) -> Option<&str> {
    let first = args(segment).into_iter().next()?;
    let name = first.rsplit(['/', '\\']).next().unwrap_or(first);
    Some(name.strip_suffix(".exe").unwrap_or(name))
}

fn is_dangerous_rm(command: &str) -> bool {
    for segment in segments(command) {
        let tokens = args(segment);
        let Some(first) = tokens.first() else {
            continue;
        };
        if first.rsplit('/').next().unwrap_or(first) != "rm" {
            continue;
        }

        let recursive = tokens.iter().skip(1).any(|t| {
            *t == "--recursive" || (t.starts_with('-') && !t.starts_with("--") && t.contains('r'))
        });
        // `rm -f file` without recursion is ordinary work, not a risk.
        if !recursive {
            continue;
        }

        // `rm` is an alias of Remove-Item too, so a Windows path can turn up
        // here with unix-style flags.
        if tokens
            .iter()
            .skip(1)
            .filter(|t| !t.starts_with('-'))
            .any(|t| is_critical_path(t) || is_critical_windows_path(t))
        {
            return true;
        }
    }
    false
}

/// Paths where a recursive delete is almost certainly not what was meant.
fn is_critical_path(raw: &str) -> bool {
    let path = raw.trim_matches(['"', '\'']);

    // The home directory itself, but not something inside it.
    if matches!(path, "~" | "~/" | "~/*" | "$home" | "$home/" | "$home/*") {
        return true;
    }
    // Everything in the working directory.
    if matches!(path, "*" | "." | "./" | "./*" | ".." | "../" | "../*") {
        return true;
    }
    if !path.starts_with('/') {
        return false;
    }
    if path == "/" || path == "/*" {
        return true;
    }
    let trimmed = path.trim_end_matches('*').trim_end_matches('/');

    // Temp directories are scratch space: clearing subdirectories there is
    // routine, and warning about it would only train people to confirm blindly.
    // The temp directory itself (depth 1) is not covered by this exception.
    const SCRATCH_ROOTS: &[&str] = &["/tmp/", "/var/tmp/", "/private/tmp/", "/var/folders/"];
    if SCRATCH_ROOTS
        .iter()
        .any(|root| trimmed.len() > root.len() && trimmed.starts_with(root))
    {
        return false;
    }

    // Upper levels of the filesystem: /etc, /usr/bin, /var/lib and friends.
    let depth = trimmed.split('/').filter(|s| !s.is_empty()).count();
    depth <= 2
}

/// The same idea as `is_critical_path`, in Windows spelling.
///
/// Separate rather than folded in, because `C:\Users` and `/Users` are not the
/// same depth: the drive letter is a level of its own, so a shared depth rule
/// would either miss `C:\Windows` or flag every `C:\projects\app\build`.
fn is_critical_windows_path(raw: &str) -> bool {
    let path = raw.trim_matches(['"', '\'']).replace('\\', "/");

    // The profile directory itself, in every spelling Windows shells use.
    if matches!(
        path.as_str(),
        "~" | "~/" | "~/*" | "$home" | "$home/" | "$home/*" | "$env:userprofile"
    ) {
        return true;
    }
    // Everything in the working directory.
    if matches!(
        path.as_str(),
        "*" | "." | "./" | "./*" | ".." | "../" | "../*"
    ) {
        return true;
    }

    // A drive letter is what makes this an absolute path: "c:" or "c:/...".
    let Some((drive, rest)) = path.split_once(':') else {
        return false;
    };
    if drive.len() != 1 || !drive.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return false;
    }

    // The drive root, or one level under it: C:\, C:\Windows, C:\Users.
    let trimmed = rest.trim_end_matches('*').trim_end_matches('/');
    trimmed.split('/').filter(|s| !s.is_empty()).count() <= 1
}

fn is_dangerous_dd(command: &str) -> bool {
    for segment in segments(command) {
        if program(segment) != Some("dd") && args(segment).first() != Some(&"dd") {
            continue;
        }
        // Writing to a device is the risk; reading from one (`if=`) is harmless.
        if segment.contains("of=/dev/") || segment.contains("of=\\\\.\\") {
            return true;
        }
    }
    false
}

fn is_mkfs(command: &str) -> bool {
    segments(command).iter().any(|s| {
        let name = program(s).unwrap_or("");
        name.starts_with("mkfs") || name == "mkswap" || name == "newfs"
    })
}

fn writes_to_block_device(command: &str) -> bool {
    [
        "> /dev/sd",
        ">/dev/sd",
        "> /dev/nvme",
        ">/dev/nvme",
        "> /dev/disk",
        ">/dev/disk",
        "> /dev/hd",
        ">/dev/hd",
    ]
    .iter()
    .any(|pattern| command.contains(pattern))
}

fn is_curl_pipe_shell(command: &str) -> bool {
    let downloads = ["curl ", "wget ", "iwr ", "invoke-webrequest"]
        .iter()
        .any(|p| command.contains(p));
    if !downloads {
        return false;
    }
    // The download is piped into an interpreter: the classic `curl ... | sh`.
    segments(command).iter().skip(1).any(|s| {
        matches!(
            program(s).unwrap_or(""),
            "sh" | "bash"
                | "zsh"
                | "fish"
                | "python"
                | "python3"
                | "perl"
                | "ruby"
                | "node"
                | "iex"
        )
    }) || command.contains("| iex")
        || command.contains("|iex")
}

fn is_fork_bomb(command: &str) -> bool {
    // Whitespace inside the bomb varies freely, so compare without it.
    let dense: String = command.chars().filter(|c| !c.is_whitespace()).collect();
    dense.contains(":(){:|:&};:")
}

fn is_reckless_chmod(command: &str) -> bool {
    for segment in segments(command) {
        let tokens = args(segment);
        if tokens.first().copied() != Some("chmod") {
            continue;
        }
        let recursive = tokens
            .iter()
            .any(|t| matches!(*t, "-r" | "-R" | "--recursive"));
        let permissive = tokens
            .iter()
            .any(|t| matches!(*t, "777" | "a+rwx" | "0777"));
        if recursive && permissive && tokens.iter().skip(1).any(|t| is_critical_path(t)) {
            return true;
        }
    }
    false
}

fn is_shutdown(command: &str) -> bool {
    segments(command).iter().any(|s| {
        matches!(
            program(s).unwrap_or(""),
            "shutdown" | "reboot" | "halt" | "poweroff"
        ) || args(s).first().copied() == Some("shutdown")
            || args(s).first().copied() == Some("reboot")
            || args(s).first().copied() == Some("halt")
            || args(s).first().copied() == Some("poweroff")
    }) || command.contains("restart-computer")
        || command.contains("stop-computer")
}

fn is_force_push(command: &str) -> bool {
    for segment in segments(command) {
        let tokens = args(segment);
        if tokens.first().copied() != Some("git") {
            continue;
        }
        if !tokens.contains(&"push") {
            continue;
        }
        let forced = tokens
            .iter()
            .any(|t| *t == "--force" || *t == "-f" || t.starts_with("--force-with-lease"));
        if !forced {
            continue;
        }
        // Force-pushing a personal branch is routine; shared branches are not.
        if tokens
            .iter()
            .any(|t| matches!(*t, "main" | "master" | "develop" | "release" | "production"))
        {
            return true;
        }
        // With no branch named, push targets the current one, which may be main.
        let has_explicit_branch = tokens
            .iter()
            .skip(2)
            .filter(|t| !t.starts_with('-'))
            .count()
            >= 2;
        if !has_explicit_branch {
            return true;
        }
    }
    false
}

fn is_destructive_sql(command: &str) -> bool {
    const CLIENTS: &[&str] = &[
        "psql",
        "mysql",
        "mariadb",
        "sqlite3",
        "sqlcmd",
        "mongosh",
        "clickhouse-client",
        "cockroach",
    ];
    const STATEMENTS: &[&str] = &[
        "drop table",
        "drop database",
        "drop schema",
        "truncate table",
        "delete from",
    ];

    // A database client has to be involved somewhere, or `grep "delete from"
    // dump.sql` — which reads a file and touches no database — gets flagged.
    // Any segment counts, so `echo "drop table x" | mysql` is still caught.
    let mentions_a_client = segments(command)
        .iter()
        .filter_map(|s| command_name(s))
        .any(|name| CLIENTS.contains(&name));

    mentions_a_client && STATEMENTS.iter().any(|s| command.contains(s))
}

/// A recursive `Remove-Item` aimed at a critical path.
///
/// The path is checked for the same reason it is in `is_dangerous_rm`:
/// `Remove-Item ./build -Recurse -Force` is routine work, and warning about it
/// would teach people to confirm without reading — the one habit this module
/// must not create. `-Force` is not required: deleting `C:\Windows` recursively
/// is no less destructive for the files that go without it.
fn is_powershell_recursive_delete(command: &str) -> bool {
    // Every one of these is an alias of Remove-Item in PowerShell.
    const ALIASES: &[&str] = &["remove-item", "ri", "rm", "rmdir", "rd", "del", "erase"];

    for segment in segments(command) {
        let Some(name) = command_name(segment) else {
            continue;
        };
        if !ALIASES.contains(&name) {
            continue;
        }
        let tokens = args(segment);
        // PowerShell spells it -Recurse and accepts any unambiguous prefix.
        let recursive = tokens
            .iter()
            .skip(1)
            .any(|t| t.starts_with("-r") && "-recurse".starts_with(*t));
        if !recursive {
            continue;
        }
        if tokens
            .iter()
            .skip(1)
            .filter(|t| !t.starts_with('-'))
            .any(|t| is_critical_windows_path(t))
        {
            return true;
        }
    }
    false
}

fn is_powershell_format(command: &str) -> bool {
    command.contains("format-volume")
        || command.contains("clear-disk")
        || command.contains("initialize-disk")
}

fn is_cmd_recursive_delete(command: &str) -> bool {
    for segment in segments(command) {
        let Some(name) = command_name(segment) else {
            continue;
        };
        if !matches!(name, "del" | "erase" | "rmdir" | "rd") {
            continue;
        }
        let tokens = args(segment);
        if !tokens.contains(&"/s") {
            continue;
        }
        // Switches here start with a slash, so a path cannot be told from one
        // by the leading character alone — exclude both forms.
        if tokens
            .iter()
            .skip(1)
            .filter(|t| !t.starts_with('/') && !t.starts_with('-'))
            .any(|t| is_critical_windows_path(t))
        {
            return true;
        }
    }
    false
}

fn is_disk_partitioning(command: &str) -> bool {
    segments(command).iter().any(|s| {
        matches!(
            program(s).unwrap_or(""),
            "diskpart" | "fdisk" | "parted" | "gparted"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dangerous(cmd: &str) -> bool {
        classify(cmd).reason().is_some()
    }

    // --- matches ---

    #[test]
    fn catches_recursive_root_deletion() {
        assert!(dangerous("rm -rf /"));
        assert!(dangerous("rm -rf /*"));
        assert!(dangerous("sudo rm -rf /usr"));
        assert!(dangerous("rm -fr /etc/"));
        assert!(dangerous("/bin/rm -rf /var/lib"));
    }

    #[test]
    fn catches_home_directory_wipe() {
        assert!(dangerous("rm -rf ~"));
        assert!(dangerous("rm -rf ~/*"));
        assert!(dangerous("rm -rf $HOME"));
    }

    #[test]
    fn catches_deletion_hidden_after_a_harmless_command() {
        // This is exactly why the command is split into segments.
        assert!(dangerous("cd /tmp && rm -rf /"));
        assert!(dangerous("echo starting; sudo rm -rf /etc"));
    }

    #[test]
    fn the_temp_directory_itself_is_still_protected() {
        // The exception covers the contents, not the directory itself:
        // wiping all of /tmp is still worth a question.
        assert!(dangerous("rm -rf /tmp"));
        assert!(dangerous("rm -rf /tmp/"));
        assert!(dangerous("rm -rf /var/tmp"));
    }

    #[test]
    fn a_lookalike_path_does_not_slip_through_the_temp_exception() {
        // /tmpfoo is not inside /tmp — the prefix check includes the slash.
        assert!(dangerous("rm -rf /tmpfoo"));
    }

    #[test]
    fn catches_device_overwrites() {
        assert!(dangerous("dd if=/dev/zero of=/dev/sda bs=1M"));
        assert!(dangerous("echo x > /dev/sda"));
        assert!(dangerous("mkfs.ext4 /dev/sdb1"));
    }

    #[test]
    fn catches_piping_the_internet_into_a_shell() {
        assert!(dangerous("curl https://example.com/install.sh | sh"));
        assert!(dangerous("wget -qO- https://example.com/x | bash"));
    }

    #[test]
    fn catches_fork_bomb() {
        assert!(dangerous(":(){ :|:& };:"));
    }

    #[test]
    fn catches_reckless_chmod() {
        assert!(dangerous("chmod -R 777 /"));
        assert!(dangerous("sudo chmod -R 777 /usr/bin"));
    }

    #[test]
    fn catches_shutdown_and_reboot() {
        assert!(dangerous("shutdown -h now"));
        assert!(dangerous("sudo reboot"));
        assert!(dangerous("Restart-Computer -Force"));
    }

    #[test]
    fn catches_force_push_to_shared_branches() {
        assert!(dangerous("git push --force origin main"));
        assert!(dangerous("git push -f origin master"));
        // With no branch named it targets the current one, which may be main.
        assert!(dangerous("git push --force"));
    }

    #[test]
    fn catches_destructive_sql() {
        assert!(dangerous("psql -c 'DROP TABLE users'"));
        assert!(dangerous("mysql -e 'DROP DATABASE prod'"));
    }

    #[test]
    fn catches_windows_equivalents() {
        assert!(dangerous("Remove-Item -Path C:\\ -Recurse -Force"));
        assert!(dangerous("del /f /s /q C:\\Windows"));
        assert!(dangerous("Format-Volume -DriveLetter D"));
        assert!(dangerous("diskpart"));
    }

    #[test]
    fn a_windows_delete_is_dangerous_without_force_too() {
        // -Force only decides whether read-only files go quietly; everything
        // else under the path is gone either way.
        assert!(dangerous("Remove-Item C:\\Windows -Recurse"));
        assert!(dangerous("rd /s C:\\Users"));
        assert!(dangerous("rm -rf C:\\"));
    }

    #[test]
    fn a_windows_profile_wipe_is_caught() {
        assert!(dangerous("Remove-Item $env:USERPROFILE -Recurse -Force"));
        assert!(dangerous("Remove-Item ~ -Recurse"));
    }

    // --- no false positives ---

    #[test]
    fn ordinary_commands_are_safe() {
        for cmd in [
            "ls -la",
            "git status --short",
            "df -h",
            "ps aux | grep node",
            "find . -name '*.rs'",
            "du -sh * | sort -h",
            "cat /etc/hosts",
            "tail -f /var/log/system.log",
        ] {
            assert!(!dangerous(cmd), "false positive on `{cmd}`");
        }
    }

    #[test]
    fn scoped_deletions_are_safe() {
        for cmd in [
            "rm -rf node_modules",
            "rm -rf ./target/debug",
            "rm -rf ~/projects/old/build",
            "rm -f config.bak",
            "rm -rf /tmp/plz-cache/session-1",
            // Clearing a temp subdirectory is routine, not a reason to ask.
            "rm -rf /tmp/build",
            "rm -rf /var/tmp/cache",
        ] {
            assert!(!dangerous(cmd), "false positive on `{cmd}`");
        }
    }

    #[test]
    fn redirecting_to_dev_null_is_safe() {
        // /dev/null looks like a block device but is harmless.
        assert!(!dangerous("make build > /dev/null 2>&1"));
        assert!(!dangerous("cat /dev/urandom | head -c 10"));
    }

    #[test]
    fn downloading_without_executing_is_safe() {
        assert!(!dangerous("curl -O https://example.com/archive.tar.gz"));
        assert!(!dangerous("wget https://example.com/file.txt"));
        assert!(!dangerous("curl -s https://api.example.com | jq '.items'"));
    }

    #[test]
    fn force_push_to_a_feature_branch_is_safe() {
        assert!(!dangerous("git push --force origin feature/my-branch"));
    }

    #[test]
    fn ordinary_git_and_chmod_are_safe() {
        assert!(!dangerous("git push origin main"));
        assert!(!dangerous("chmod +x scripts/build.sh"));
        assert!(!dangerous("chmod -R 755 ./public"));
    }

    #[test]
    fn selecting_from_a_database_is_safe() {
        assert!(!dangerous("psql -c 'SELECT * FROM users LIMIT 10'"));
    }

    #[test]
    fn sql_outside_a_database_client_is_safe() {
        // Reading a dump is not touching a database.
        assert!(!dangerous("grep 'delete from' dump.sql"));
        assert!(!dangerous("rg 'drop table' migrations/"));
        assert!(!dangerous("cat schema.sql | grep -c 'truncate table'"));
        // But a client on the other end of the pipe still counts.
        assert!(dangerous("echo 'drop table users' | mysql shop"));
    }

    #[test]
    fn scoped_windows_deletions_are_safe() {
        // These are everyday commands. Warning about them is worse than
        // useless: it is what teaches people to confirm without reading.
        for cmd in [
            "Remove-Item ./build -Recurse -Force",
            "Remove-Item .\\target\\debug -Recurse",
            "del /f /s /q .\\dist",
            "rd /s /q C:\\projects\\app\\node_modules",
        ] {
            assert!(!dangerous(cmd), "false positive on `{cmd}`");
        }
    }

    #[test]
    fn a_word_containing_an_alias_is_not_a_delete() {
        // The rules used to match "rd " and "-r " as bare substrings, so an
        // ordinary word like "forward" tripped them.
        assert!(!dangerous("echo forward -recurse -force"));
        assert!(!dangerous("git log --grep 'ri ' -r "));
    }

    #[test]
    fn reason_is_reported_for_dangerous_commands() {
        // The rule itself, not its wording: the sentence is translated, so a
        // string comparison here would only be asserting the English column.
        assert_eq!(classify("rm -rf /").reason(), Some(Reason::RecursiveDelete));
        assert_eq!(classify("shutdown -h now").reason(), Some(Reason::Shutdown));
        assert_eq!(classify("ls").reason(), None);
    }

    #[test]
    fn every_reason_has_a_sentence_of_its_own() {
        // A rule added without its catalogue entry comes back from `t!` as the
        // bare key, which would then be shown to the user as the warning.
        let mut seen = Vec::new();
        for (reason, _) in RULES {
            let text = reason.text();
            assert!(!text.starts_with("safety."), "{reason:?}: {text}");
            assert!(!seen.contains(&text), "{reason:?} repeats an earlier text");
            seen.push(text);
        }
        assert_eq!(seen.len(), RULES.len());
    }

    #[test]
    fn empty_command_is_safe() {
        assert!(!dangerous(""));
        assert!(!dangerous("   "));
    }
}
