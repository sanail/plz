mod cli;
mod clipboard;
mod config;
mod context;
mod exec;
mod i18n;
mod input;
mod install;
mod integration;
mod prompt;
mod provider;
mod safety;
mod suggestion;
#[cfg(test)]
mod testutil;
mod ui;
mod winpath;

use std::io::IsTerminal;

use anyhow::Result;
use clap::Parser;
use rust_i18n::t;

use cli::{Cli, Command, ConfigAction};
use config::Config;
use context::Context;
use provider::openai::OpenAiProvider;
use provider::Provider;
use suggestion::Suggestion;
use ui::Outcome;

// A message missing from a language falls back to English rather than showing
// its key. `--help` is deliberately not translated and stays with clap.
rust_i18n::i18n!("locales", fallback = "en");

fn main() {
    if let Err(err) = run() {
        eprintln!("plz: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // Before anything can be printed, including the complaint about a missing
    // config — hence the best-effort read: a broken or absent config must not
    // stop the language from being settled.
    i18n::init(
        Config::load()
            .ok()
            .and_then(|config| config.behavior.language)
            .as_deref(),
    );
    let cli = Cli::parse();

    match &cli.command {
        Some(Command::Config { action }) => run_config(action),
        Some(Command::Hook { shell, install }) => {
            if *install {
                return install::run(*shell, cli.yes);
            }
            // The script goes to stdout, because it is fed to `eval "$(...)"`.
            print!("{}", integration::script(*shell));
            // The hint goes to stderr, or it would land inside that same eval.
            // Gated on *stdout*: a redirected stdout means the output is being
            // consumed — by `eval`, by `Invoke-Expression`, by a file — and the
            // startup files hold exactly such a line, so a hint keyed on stderr
            // prints on every shell start.
            if std::io::stdout().is_terminal() {
                let hint = integration::install_hint(*shell);
                eprintln!("\n# {}", t!("install.to_install", hint = hint));
                if integration::startup_line(*shell).is_some() {
                    eprintln!("# {}", t!("install.or_let_plz_do_it", shell = shell.arg()));
                }
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
        anyhow::bail!("{}", t!("errors.no_suggestions"));
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
        // The suggestions have already been printed and there was no terminal
        // to choose with; that is a completed run, not a failed one.
        Outcome::Listed => Ok(()),
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
            eprintln!("{}", t!("ui.warning", reason = reason.text()));
            eprintln!("  {command}");
            if !input::confirm(&t!("ui.run_it"))? {
                eprintln!("{}", t!("ui.cancelled"));
                std::process::exit(130);
            }
        }
    }

    if exec::integration_active() {
        // The wrapper runs it in the current shell, where cd and export work.
        return exec::hand_off(exec::Verb::Run, command);
    }

    // The wrapper is installed, but the file it named cannot be opened from here
    // — under Cygwin that is a POSIX path a native binary never sees. Handing
    // the command over would write it where nobody reads it, so we run it in a
    // child instead and say why the session did not change.
    if let Some(path) = exec::requested_output_file() {
        eprintln!("{}", t!("ui.wrapper_file_unreachable", path = path));
    }

    let code = exec::run_in_child_shell(&ctx.shell, command)?;
    if !exec::integration_requested() && changes_shell_state(command) {
        // Name the shell only when a wrapper exists for it; otherwise the
        // generic form, so the suggestion is always a line that works.
        let hook = match integration::hook_arg(ctx.shell.kind) {
            Some(arg) => format!("plz hook {arg} --install"),
            None => "plz hook <shell> --install".to_string(),
        };
        eprintln!("{}", t!("ui.child_process_note", hook = hook));
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
    // PowerShell is case-insensitive and the model varies the casing of its
    // cmdlets; a POSIX command in capitals is not a real thing, so lowercasing
    // for every shell costs nothing.
    let first = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    // `$env:FOO = 'bar'` has no space before the colon, so the assignment
    // arrives as a single token.
    if first.starts_with("$env:") {
        return true;
    }
    // Set-Item and its relatives touch session state only on the Env: drive;
    // everywhere else they are ordinary file operations.
    if matches!(
        first.as_str(),
        "set-item" | "new-item" | "remove-item" | "si" | "ni" | "ri"
    ) {
        return command.to_ascii_lowercase().contains("env:");
    }

    matches!(
        first.as_str(),
        "cd" | "chdir" | "export" | "source" | "." | "alias" | "unalias" | "set" | "unset"
            | "pushd" | "popd"
            // PowerShell: the cmdlet form comes back from the model more often
            // than `cd` does, and used to slip past this check unnoticed.
            | "set-location" | "sl" | "push-location" | "pop-location"
            | "set-variable" | "set-alias" | "new-alias"
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
    // The labels are prose and translated; the values beside them are what
    // goes to the model, and stay exactly as they are sent.
    println!("\n# {}", t!("wizard.detected_environment"));
    print!("# {} {}", t!("wizard.field_os"), ctx.os);
    match &ctx.os_version {
        Some(version) => println!(" {version}"),
        None => println!(),
    }
    println!("# {} {}", t!("wizard.field_architecture"), ctx.arch);
    println!("# {} {}", t!("wizard.field_shell"), ctx.shell);
    let directory = match &ctx.cwd {
        Some(cwd) => cwd.clone(),
        None => t!("wizard.directory_not_sent").to_string(),
    };
    println!("# {} {directory}", t!("wizard.field_directory"));
    // Reported by what the protocol can actually do, not by the presence of a
    // variable: a wrapper whose file we cannot open is not an active one.
    let wrapper = if exec::integration_active() {
        t!("wizard.wrapper_active")
    } else {
        t!("wizard.wrapper_not_installed")
    };
    println!("# {} {wrapper}", t!("wizard.field_wrapper"));
}

fn edit_config() -> Result<()> {
    let path = Config::path()?;
    if !path.exists() {
        anyhow::bail!("{}", t!("errors.no_config_to_edit", path = path.display()));
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| default_editor().to_string());

    let status = std::process::Command::new(&editor).arg(&path).status();
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => anyhow::bail!(
            "{}",
            t!("errors.editor_failed", editor = editor, status = status)
        ),
        Err(err) => anyhow::bail!(
            "{}",
            t!("errors.editor_not_launched", editor = editor, err = err)
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
    if path.exists() && !input::confirm(&t!("wizard.overwrite", path = path.display()))? {
        println!("{}", t!("ui.cancelled"));
        return Ok(());
    }

    println!("{}", t!("wizard.choose_a_provider"));
    let titles: Vec<String> = presets::ALL
        .iter()
        .map(|p| format!("{} ({})", p.title_display(), p.base_url_display()))
        .collect();
    let choice = input::choose(&t!("wizard.number"), &titles, 0)?;
    let preset = presets::ALL[choice];

    let base_url = input::read_line(&t!("wizard.base_url"), Some(preset.base_url))?;
    if base_url.is_empty() {
        anyhow::bail!("{}", t!("errors.base_url_required"));
    }
    let model = input::read_line(&t!("wizard.model"), Some(preset.model))?;
    if model.is_empty() {
        anyhow::bail!("{}", t!("errors.model_required"));
    }

    let mut config = Config::default();
    config.provider.preset = preset.name.to_string();
    config.provider.base_url = base_url.trim_end_matches('/').to_string();
    config.provider.model = model;
    config.behavior.json_mode = preset.json_mode;
    config.behavior.disable_thinking = preset.disable_thinking;

    if preset.key_env.is_some() {
        println!(
            "{}",
            t!("wizard.get_a_key", hint = preset.key_hint_display())
        );
        if let Some(var) = preset.key_env {
            println!("{}", t!("wizard.leave_blank", var = var));
        }
        let key = input::read_secret(&t!("wizard.api_key"))?;
        if !key.is_empty() {
            config.provider.api_key = Some(key);
        }
    }

    let saved = config.save()?;
    println!("{}", t!("wizard.saved", path = saved.display()));

    if config.key_required() && config.api_key().is_none() {
        println!("{}", t!("wizard.no_key_set"));
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
    fn state_changing_commands_are_recognised_in_every_shell() {
        // The PowerShell spellings used to be missing, so the note about the
        // child process never appeared there — the model answers with the
        // cmdlet far more often than with `cd`.
        for command in [
            "cd ..",
            "export FOO=bar",
            "source ./venv/bin/activate",
            "Set-Location ..",
            "set-location C:\\src",
            "sl ..",
            "Push-Location ..",
            "$env:FOO = 'bar'",
            "Set-Item Env:FOO bar",
        ] {
            assert!(changes_shell_state(command), "{command}");
        }
        for command in [
            "ls -la",
            "Get-ChildItem",
            "git status",
            "Set-Item .\\file.txt x",
        ] {
            assert!(!changes_shell_state(command), "{command}");
        }
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
