use clap::{Parser, Subcommand, ValueEnum};

/// Describe a task in plain language, get a command ready for your OS and shell.
#[derive(Debug, Parser)]
#[command(name = "plz", version, about, long_about = None)]
pub struct Cli {
    /// The task, in plain language. With no arguments, interactive mode opens.
    #[arg(trailing_var_arg = true)]
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
    /// How to install it:
    ///   zsh:        echo 'eval "$(plz init zsh)"' >> ~/.zshrc
    ///   bash:       echo 'eval "$(plz init bash)"' >> ~/.bashrc
    ///   fish:       plz init fish > ~/.config/fish/conf.d/plz.fish
    ///   powershell: plz init powershell >> $PROFILE
    // Without verbatim_doc_comment clap reflows the block into a single
    // paragraph, turning the aligned example list into an unreadable run-on.
    #[command(verbatim_doc_comment)]
    Init {
        /// The shell to generate a wrapper for
        shell: Shell,
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
