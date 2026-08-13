use clap::{Parser, Subcommand, ValueEnum};

/// Describe a task in plain language, get a command ready for your OS and shell.
#[derive(Debug, Parser)]
#[command(name = "plz", version, about, long_about = None)]
pub struct Cli {
    /// The task, in plain language. With no arguments, interactive mode opens.
    ///
    /// Flags go before the task: everything after it is read as part of it, so
    /// `plz what does git push --force do` asks about the flag instead of using
    /// it. A task starting with the word `config` or `hook` collides with the
    /// subcommand of that name — quote it, or put `--` in front.
    // trailing_var_arg is what lets an unquoted task contain a word starting
    // with a dash, which is a question this tool gets asked constantly.
    #[arg(trailing_var_arg = true, verbatim_doc_comment)]
    pub task: Vec<String>,

    /// How many command suggestions to request
    #[arg(short = 'n', long, value_name = "N")]
    pub count: Option<usize>,

    /// Use a different model instead of the one in the config
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,

    /// Only show the suggestions, run nothing
    #[arg(long)]
    pub dry_run: bool,

    /// Do not ask for confirmation on risky commands
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Print the suggestions as JSON and exit (for scripts)
    #[arg(long)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage the configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Print a shell wrapper so commands run in your current shell
    ///
    /// `plz hook <shell> --install` adds the line for you and asks before it
    /// writes anything. To do it by hand:
    ///   zsh:        echo 'eval "$(plz hook zsh)"' >> ~/.zshrc
    ///   bash:       echo 'eval "$(plz hook bash)"' >> ~/.bashrc
    ///   fish:       echo 'plz hook fish | source' > ~/.config/fish/conf.d/plz.fish
    ///   powershell: Add-Content $PROFILE 'plz hook powershell | Out-String | Invoke-Expression'
    // Without verbatim_doc_comment clap reflows the block into a single
    // paragraph, turning the aligned example list into an unreadable run-on.
    #[command(verbatim_doc_comment)]
    Hook {
        /// The shell to generate a wrapper for
        shell: Shell,

        /// Add the line to the shell's startup file instead of printing the
        /// script. Asks first; --yes answers for you
        #[arg(long)]
        install: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Interactive first-time setup: provider, model, key
    Init,
    /// Print the path to the configuration file
    Path,
    /// Print the current configuration (the key is masked)
    Show,
    /// Open the configuration in $EDITOR
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
    #[value(alias = "pwsh")]
    Powershell,
    Cmd,
}

impl Shell {
    /// The spelling `plz hook <shell>` accepts, for messages that hand the user
    /// a command to run. A test keeps it in step with what clap parses.
    pub fn arg(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
            Self::Powershell => "powershell",
            Self::Cmd => "cmd",
        }
    }
}

impl Cli {
    /// The task text, joined from the positional arguments.
    pub fn task_text(&self) -> Option<String> {
        let joined = self.task.join(" ").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_from(args)
    }

    #[test]
    fn a_task_may_contain_a_word_starting_with_a_dash() {
        // This is what trailing_var_arg buys, and it is the reason the flag
        // placement below is a documented rule rather than a parser fix: asking
        // about a flag is a routine use of this tool.
        let cli = parse(&["plz", "what", "does", "git", "push", "--force", "do"]);
        assert_eq!(
            cli.task_text().as_deref(),
            Some("what does git push --force do")
        );
    }

    #[test]
    fn flags_are_recognised_before_the_task() {
        let cli = parse(&["plz", "--dry-run", "-n", "5", "clear the cache"]);
        assert!(cli.dry_run);
        assert_eq!(cli.count, Some(5));
        assert_eq!(cli.task_text().as_deref(), Some("clear the cache"));
    }

    #[test]
    fn flags_after_the_task_are_part_of_the_task() {
        // Documented behaviour, not an accident: the same rule that lets a task
        // mention `--force` also swallows a real flag put in that position.
        let cli = parse(&["plz", "clear the cache", "--dry-run"]);
        assert!(!cli.dry_run);
        assert_eq!(
            cli.task_text().as_deref(),
            Some("clear the cache --dry-run")
        );
    }

    #[test]
    fn a_task_starting_with_a_subcommand_name_needs_quoting_or_a_separator() {
        // Bare `plz config nginx` is parsed as the `config` subcommand and
        // fails; these two forms are the documented ways around it.
        assert_eq!(
            parse(&["plz", "config nginx"]).task_text().as_deref(),
            Some("config nginx")
        );
        assert_eq!(
            parse(&["plz", "--", "config", "nginx"])
                .task_text()
                .as_deref(),
            Some("config nginx")
        );
        assert!(Cli::try_parse_from(["plz", "config", "nginx"]).is_err());
    }

    #[test]
    fn every_shell_arg_is_one_the_parser_accepts() {
        // These strings are handed to the user to type back, so a rename that
        // misses one has to fail here rather than in their terminal.
        for shell in [
            Shell::Zsh,
            Shell::Bash,
            Shell::Fish,
            Shell::Powershell,
            Shell::Cmd,
        ] {
            let parsed = parse(&["plz", "hook", shell.arg()]);
            assert!(
                matches!(parsed.command, Some(Command::Hook { shell: got, .. }) if got == shell),
                "{shell:?} does not round-trip through `{}`",
                shell.arg()
            );
        }
    }

    #[test]
    fn the_hook_subcommand_takes_an_optional_install_flag() {
        let plain = parse(&["plz", "hook", "powershell"]);
        assert!(matches!(
            plain.command,
            Some(Command::Hook {
                shell: Shell::Powershell,
                install: false
            })
        ));

        let installing = parse(&["plz", "-y", "hook", "powershell", "--install"]);
        assert!(installing.yes);
        assert!(matches!(
            installing.command,
            Some(Command::Hook { install: true, .. })
        ));
    }
}
