//! Turning a POSIX path from the shell into one this process can open.
//!
//! plz ships as a native Windows binary, and under Cygwin it is launched from a
//! shell that lives in a POSIX filesystem. MSYS2 and Git Bash rewrite path-shaped
//! arguments and environment variables on the way to a native child; Cygwin, by
//! design, does not. So `/home/name` and `/tmp/plz.abc123` arrive verbatim,
//! Windows resolves them against the current drive, and the result is
//! `C:\home\name` — a silently wrong location rather than an error.
//!
//! `cygpath` is the authority on the mapping, because only it knows the
//! installation root and the mount table. It ships with Cygwin, MSYS2 and Git
//! Bash alike, so one question answers all three.

use std::path::PathBuf;

/// The native form of a POSIX path, when there is a `cygpath` to ask.
///
/// `None` when the path is already native or relative, when `cygpath` is absent
/// — a plain Windows shell — and on every other platform, where paths reach the
/// process the way the shell wrote them.
#[cfg(windows)]
pub fn to_native(path: &str) -> Option<PathBuf> {
    // A leading slash is what a native Windows path never starts with, and what
    // every path needing translation does.
    if !path.starts_with('/') {
        return None;
    }

    let output = std::process::Command::new("cygpath")
        .arg("-w")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let native = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!native.is_empty()).then(|| PathBuf::from(native))
}

#[cfg(not(windows))]
pub fn to_native(_path: &str) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_and_relative_paths_are_left_alone() {
        // No `cygpath` is spawned for these, so the test says the same thing on
        // every platform.
        assert_eq!(to_native(r"C:\Users\name\.bashrc"), None);
        assert_eq!(to_native("plz.abc123"), None);
        assert_eq!(to_native(""), None);
    }
}
