use serde::{Deserialize, Serialize};

/// A single command suggestion returned by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// A ready-to-run command for the current shell
    pub command: String,
    /// A short note on what it does
    #[serde(default)]
    pub explanation: String,
}

impl Suggestion {
    /// Test-only constructor: in production suggestions arrive as JSON.
    #[cfg(test)]
    pub fn new(command: impl Into<String>, explanation: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            explanation: explanation.into(),
        }
    }

    /// The command rendered as a single list line.
    ///
    /// The model rarely returns multi-line commands, but when it does the
    /// newline breaks the list layout, so join them instead.
    pub fn one_line(&self) -> String {
        let joined = self
            .command
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        if joined.is_empty() {
            self.command.trim().to_string()
        } else {
            joined
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_joins_multiline_commands() {
        let s = Suggestion::new("cd /tmp\nls -la", "");
        assert_eq!(s.one_line(), "cd /tmp; ls -la");
    }

    #[test]
    fn one_line_leaves_single_line_alone() {
        let s = Suggestion::new("git status --short", "");
        assert_eq!(s.one_line(), "git status --short");
    }

    #[test]
    fn one_line_skips_blank_lines() {
        let s = Suggestion::new("echo a\n\n\necho b", "");
        assert_eq!(s.one_line(), "echo a; echo b");
    }
}
