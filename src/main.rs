mod cli;
mod clipboard;
mod config;
mod context;
mod exec;
mod input;
mod integration;
mod prompt;
mod provider;
mod safety;
mod suggestion;
mod ui;

use std::io::IsTerminal;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command, ConfigAction};
use config::Config;
use context::Context;
use provider::openai::OpenAiProvider;
use provider::Provider;
use suggestion::Suggestion;
use ui::Outcome;

fn main() {
    if let Err(err) = run() {
        eprintln!("plz: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Some(Command::Config { action }) => run_config(action),
        Some(Command::Hook { shell }) => {
            // The script goes to stdout, because it is fed to `eval "$(...)"`.
            print!("{}", integration::script(*shell));
            // The hint goes to stderr, or it would land inside that same eval.
            if std::io::stderr().is_terminal() {
                eprintln!("\n# To install: {}", integration::install_hint(*shell));
            }
            Ok(())
        }
        None => match cli.task_text() {
            Some(task) => run_task(&cli, &task),
            None => run_interactive(&cli),
        },
    }
}

/// Direct CLI request: fetch suggestions, let the user pick, then run it.
fn run_task(cli: &Cli, task: &str) -> Result<()> {
    let config = Config::load()?;
    // Beyond nine there is no digit left to pick with, and fewer than one is
    // pointless — clamp here rather than trusting the model to comply.
    let count = cli.count.unwrap_or(config.behavior.suggestions).clamp(1, 9);

    let ctx = Context::detect(config.behavior.send_cwd);
    let provider = OpenAiProvider::from_config(&config)?.with_model(cli.model.clone());
    let suggestions = provider.suggest(&ctx, task, count)?;

    if suggestions.is_empty() {
        anyhow::bail!("the model returned no suggestions");
    }

    // Before the picker: with these flags there is nothing to pick.
    if print_only(cli, &suggestions)? {
        return Ok(());
    }

    let outcome = ui::select::select(&suggestions, buffer_supported(&ctx))?;
    handle_outcome(cli, &config, &ctx, &suggestions, outcome)
}

/// An output mode that prints the suggestions and stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrintOnly {
    /// `--json`
    Json,
    /// `--dry-run`
    Plain,
}

/// Which print-and-stop mode the flags ask for, if any.
fn print_only_mode(cli: &Cli) -> Option<PrintOnly> {
    if cli.json {
        Some(PrintOnly::Json)
    } else if cli.dry_run {
        Some(PrintOnly::Plain)
    } else {
        None
    }
}

/// Print the suggestions if a flag asks for it; `true` means we are done.
///
/// Both modes have to be honoured in the interactive mode as well as the direct
/// one: `--dry-run` is the flag people reach for precisely so that nothing runs,
/// and it must not depend on how the suggestions were obtained.
fn print_only(cli: &Cli, suggestions: &[Suggestion]) -> Result<bool> {
    match print_only_mode(cli) {
        Some(PrintOnly::Json) => println!("{}", serde_json::to_string_pretty(suggestions)?),
        Some(PrintOnly::Plain) => print_suggestions(suggestions),
        None => return Ok(false),
    }
    Ok(true)
}

/// Interactive mode: running plz with no arguments.
fn run_interactive(cli: &Cli) -> Result<()> {
    let config = Config::load()?;
    let count = cli.count.unwrap_or(config.behavior.suggestions).clamp(1, 9);
    let ctx = Context::detect(config.behavior.send_cwd);

    let provider = OpenAiProvider::from_config(&config)?.with_model(cli.model.clone());
    let session = ui::tui::run(
        std::sync::Arc::new(provider),
        ctx.clone(),
        count,
        buffer_supported(&ctx),
    )?;

    // A cancelled session has nothing to print, whatever the flags say.
    if session.outcome != Outcome::Cancel && print_only(cli, &session.suggestions)? {
        return Ok(());
    }

    handle_outcome(cli, &config, &ctx, &session.suggestions, session.outcome)
}

/// Whether the shell can take a command into its prompt buffer instead of
/// running it.
///
/// Without the wrapper there is nowhere to insert it, so Tab degrades to a copy.
fn buffer_supported(ctx: &Context) -> bool {
    exec::integration_active() && exec::supports_buffer(ctx.shell.kind)
}

/// Shared tail for both modes: what to do with the chosen suggestion.
fn handle_outcome(
    cli: &Cli,
    config: &Config,
    ctx: &Context,
    suggestions: &[Suggestion],
    outcome: Outcome,
) -> Result<()> {
    match outcome {
        Outcome::Cancel => std::process::exit(130),
        Outcome::Copy(index) => {
            let command = &suggestions[index].command;
            let method = clipboard::copy(command)?;
            eprintln!("{}: {command}", method.describe());
            Ok(())
        }
        Outcome::Buffer(index) => exec::hand_off(exec::Verb::Buffer, &suggestions[index].command),
        Outcome::Run(index) => execute(cli, config, ctx, &suggestions[index]),
    }
}

/// Run the chosen suggestion, asking for confirmation on risky commands.
fn execute(cli: &Cli, config: &Config, ctx: &Context, suggestion: &Suggestion) -> Result<()> {
    let command = &suggestion.command;

    if config.behavior.confirm_dangerous && !cli.yes {
        let risk = safety::classify(command);
        if let Some(reason) = risk.reason() {
            eprintln!("Warning: {reason}.");
            eprintln!("  {command}");
            if !input::confirm("Run it?")? {
                eprintln!("Cancelled.");
                std::process::exit(130);
            }
        }
    }

    if exec::integration_active() {
        // The wrapper runs it in the current shell, where cd and export work.
        return exec::hand_off(exec::Verb::Run, command);
    }

    let code = exec::run_in_child_shell(&ctx.shell, command)?;
    if changes_shell_state(command) {
        // Name the shell only when a wrapper exists for it; otherwise the
        // generic form, so the suggestion is always a line that works.
        let hook = match integration::hook_arg(ctx.shell.kind) {
            Some(arg) => format!("plz hook {arg}"),
            None => "plz hook <shell>".to_string(),
        };
        eprintln!(
            "Note: this command changes shell state but ran in a child process.\n\
             Install the wrapper — `{hook}` — to make such commands affect your current shell."
        );
    }
    // Pass the exit code outwards, or `plz "..." && next-step` would run the
    // next step even after a failure.
    std::process::exit(code);
}

/// Whether the command changes the shell's own state.
///
/// Such commands run to no effect in a child process, and without a note that
/// looks like "the tool did nothing".
fn changes_shell_state(command: &str) -> bool {
    let first = command.split_whitespace().next().unwrap_or("");
    matches!(
        first,
        "cd" | "export" | "source" | "." | "alias" | "unalias" | "set" | "unset" | "pushd" | "popd"
    )
}

fn print_suggestions(suggestions: &[Suggestion]) {
    for (i, s) in suggestions.iter().enumerate() {
        println!("{}. {}", i + 1, s.command);
        if !s.explanation.is_empty() {
            println!("   {}", s.explanation);
        }
    }
}

fn run_config(action: &ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Path => {
            println!("{}", Config::path()?.display());
            Ok(())
        }
        ConfigAction::Show => {
            let config = Config::load()?;
            print!("{}", toml::to_string_pretty(&config.redacted())?);
            print_environment(&config);
            Ok(())
        }
        ConfigAction::Edit => edit_config(),
        ConfigAction::Init => init_config(),
    }
}

/// Show exactly what plz will tell the model about the environment.
///
/// This is the first thing to check when commands come back for the wrong
/// shell, and the first thing to attach to a bug report.
fn print_environment(config: &Config) {
    let ctx = Context::detect(config.behavior.send_cwd);
    println!("\n# Detected environment (sent to the model)");
    print!("# OS: {}", ctx.os);
    match &ctx.os_version {
        Some(version) => println!(" {version}"),
        None => println!(),
    }
    println!("# Architecture: {}", ctx.arch);
    println!("# Shell: {}", ctx.shell);
    match &ctx.cwd {
        Some(cwd) => println!("# Directory: {cwd}"),
        None => println!("# Directory: not sent (send_cwd = false)"),
    }
    if std::env::var_os("PLZ_OUTPUT_FILE").is_some() {
        println!("# Shell wrapper: active");
    } else {
        println!("# Shell wrapper: not installed (see `plz hook <shell>`)");
    }
}

fn edit_config() -> Result<()> {
    let path = Config::path()?;
    if !path.exists() {
        anyhow::bail!(
            "no configuration found at {}.\nRun `plz config init`.",
            path.display()
        );
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| default_editor().to_string());

    let status = std::process::Command::new(&editor).arg(&path).status();
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => anyhow::bail!("{editor} exited with status {status}"),
        Err(err) => anyhow::bail!(
            "could not launch the editor `{editor}`: {err}\nSet a different one via $EDITOR."
        ),
    }
}

fn default_editor() -> &'static str {
    if cfg!(windows) {
        "notepad"
    } else {
        "vi"
    }
}

fn init_config() -> Result<()> {
    use provider::presets;

    let path = Config::path()?;
    if path.exists() && !input::confirm(&format!("{} already exists. Overwrite?", path.display()))?
    {
        println!("Cancelled.");
        return Ok(());
    }

    println!("Setting up plz. Choose a provider:");
    let titles: Vec<String> = presets::ALL
        .iter()
        .map(|p| format!("{} ({})", p.title, p.base_url_display()))
        .collect();
    let choice = input::choose("Number", &titles, 0)?;
    let preset = presets::ALL[choice];

    let base_url = input::read_line("Base URL", Some(preset.base_url))?;
    if base_url.is_empty() {
        anyhow::bail!("a base URL is required");
    }
    let model = input::read_line("Model", Some(preset.model))?;
    if model.is_empty() {
        anyhow::bail!("a model name is required");
    }

    let mut config = Config::default();
    config.provider.preset = preset.name.to_string();
    config.provider.base_url = base_url.trim_end_matches('/').to_string();
    config.provider.model = model;
    config.behavior.json_mode = preset.json_mode;
    config.behavior.disable_thinking = preset.disable_thinking;

    if preset.key_env.is_some() {
        println!("Get a key here: {}", preset.key_hint);
        if let Some(var) = preset.key_env {
            println!("Leave this blank if the key is already set in {var} or PLZ_API_KEY.");
        }
        let key = input::read_secret("API key")?;
        if !key.is_empty() {
            config.provider.api_key = Some(key);
        }
    }

    let saved = config.save()?;
    println!("Configuration saved to {}", saved.display());

    if config.key_required() && config.api_key().is_none() {
        println!("Warning: no key set, neither in the config nor in the environment.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        Cli::parse_from(args)
    }

    #[test]
    fn dry_run_and_json_print_instead_of_running() {
        assert_eq!(
            print_only_mode(&cli(&["plz", "--json", "list files"])),
            Some(PrintOnly::Json)
        );
        assert_eq!(
            print_only_mode(&cli(&["plz", "--dry-run", "list files"])),
            Some(PrintOnly::Plain)
        );
        assert_eq!(print_only_mode(&cli(&["plz", "list files"])), None);
    }

    #[test]
    fn the_flags_also_apply_with_no_task_argument() {
        // No task means interactive mode, which used to ignore both flags and
        // run the chosen command anyway.
        assert_eq!(
            print_only_mode(&cli(&["plz", "--dry-run"])),
            Some(PrintOnly::Plain)
        );
        assert_eq!(
            print_only_mode(&cli(&["plz", "--json"])),
            Some(PrintOnly::Json)
        );
        assert!(cli(&["plz", "--dry-run"]).task_text().is_none());
    }

    #[test]
    fn json_wins_over_dry_run() {
        // Both at once is a contradiction; --json is the machine-readable one,
        // so it is the one that answers.
        assert_eq!(
            print_only_mode(&cli(&["plz", "--json", "--dry-run", "x"])),
            Some(PrintOnly::Json)
        );
    }
}
