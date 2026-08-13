//! Generating the shell wrappers (`plz hook <shell>`).
//!
//! A wrapper exists for exactly one reason: to run the command in the *current*
//! shell rather than a child process. Only then do `cd`, `export`, `source` and
//! venv activation change the session the user is sitting in — a child process
//! cannot hand its state back to its parent, which is an OS constraint.

use crate::cli::Shell;
use crate::context::ShellKind;

const ZSH: &str = include_str!("shells/plz.zsh");
const BASH: &str = include_str!("shells/plz.bash");
const FISH: &str = include_str!("shells/plz.fish");
const POWERSHELL: &str = include_str!("shells/plz.ps1");

/// The single line that goes into the shell's startup file.
///
/// It calls the binary rather than embedding the script, so the wrapper is
/// regenerated at every shell start and follows the binary through upgrades. A
/// copy pasted into the startup file would go stale instead.
pub fn startup_line(shell: Shell) -> Option<&'static str> {
    match shell {
        Shell::Zsh => Some(r#"eval "$(plz hook zsh)""#),
        Shell::Bash => Some(r#"eval "$(plz hook bash)""#),
        Shell::Fish => Some("plz hook fish | source"),
        // Out-String is not optional: without it each line of the script
        // arrives as a separate pipeline object and Invoke-Expression sees
        // only the first one.
        Shell::Powershell => Some("plz hook powershell | Out-String | Invoke-Expression"),
        Shell::Cmd => None,
    }
}

/// The text `plz hook <shell>` prints to stdout.
pub fn script(shell: Shell) -> &'static str {
    match shell {
        Shell::Zsh => ZSH,
        Shell::Bash => BASH,
        Shell::Fish => FISH,
        Shell::Powershell => POWERSHELL,
        Shell::Cmd => CMD_EXPLANATION,
    }
}

/// There is no wrapper for cmd.exe.
///
/// cmd has no functions, and a `doskey` macro can neither branch, nor read a
/// file, nor execute an arbitrary string — that is, it can do none of the things
/// a wrapper exists for. Explaining that is more honest than shipping a script
/// that silently does nothing.
const CMD_EXPLANATION: &str = r#"@rem There is no wrapper for cmd.exe.
@rem
@rem cmd.exe has no functions, and doskey macros can neither branch nor read the
@rem result file, so they cannot run the command in the current session.
@rem
@rem plz works in cmd.exe without a wrapper: the command runs in a child
@rem process. Only commands that change the session's own state (cd, set) have
@rem no effect; run those by hand. plz warns you when that happens.
@rem
@rem For the full experience, use PowerShell instead:
@rem     plz hook powershell --install
"#;

/// How to add the startup line by hand. Printed to stderr so it stays out of
/// `eval`; `plz hook <shell> --install` does the same thing without the typing.
pub fn install_hint(shell: Shell) -> &'static str {
    match shell {
        Shell::Zsh => "echo 'eval \"$(plz hook zsh)\"' >> ~/.zshrc",
        Shell::Bash => "echo 'eval \"$(plz hook bash)\"' >> ~/.bashrc",
        Shell::Fish => "echo 'plz hook fish | source' > ~/.config/fish/conf.d/plz.fish",
        // Add-Content rather than `>>`: in Windows PowerShell 5.1 the redirect
        // writes UTF-16LE, which corrupts an existing UTF-8 profile.
        Shell::Powershell => {
            "Add-Content $PROFILE 'plz hook powershell | Out-String | Invoke-Expression'"
        }
        Shell::Cmd => "a cmd.exe wrapper is not possible — see the note above",
    }
}

/// The `plz hook` argument for a detected shell, if a wrapper exists for it.
///
/// The detected shell is a wider set than the wrapper covers, and its labels
/// ("cmd.exe", "PowerShell") are meant for prose — suggesting them verbatim
/// would produce a `plz hook` line that does not parse.
pub fn hook_arg(kind: ShellKind) -> Option<&'static str> {
    match kind {
        ShellKind::Zsh => Some("zsh"),
        ShellKind::Bash => Some("bash"),
        ShellKind::Fish => Some("fish"),
        ShellKind::PowerShell => Some("powershell"),
        ShellKind::Nushell | ShellKind::Cmd | ShellKind::Posix | ShellKind::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_SHELLS: &[Shell] = &[Shell::Zsh, Shell::Bash, Shell::Fish, Shell::Powershell];

    #[test]
    fn every_shell_has_a_script() {
        for shell in [
            Shell::Zsh,
            Shell::Bash,
            Shell::Fish,
            Shell::Powershell,
            Shell::Cmd,
        ] {
            assert!(!script(shell).trim().is_empty(), "{shell:?}");
        }
    }

    #[test]
    fn scripts_call_the_binary_without_recursing() {
        // The function shares its name with the binary, so the call to the
        // binary has to be explicit — otherwise the wrapper calls itself.
        for shell in [Shell::Zsh, Shell::Bash, Shell::Fish] {
            assert!(
                script(shell).contains("command plz"),
                "{shell:?} must call the binary through `command plz`"
            );
        }
        // In PowerShell the equivalent is an explicit Application lookup.
        assert!(script(Shell::Powershell).contains("Get-Command plz -CommandType Application"));
    }

    #[test]
    fn scripts_pass_both_protocol_variables() {
        // Without PLZ_OUTPUT_FILE the binary runs the command itself and the
        // wrapper is pointless; without PLZ_SHELL_INTEGRATION shell detection
        // breaks.
        for shell in REAL_SHELLS {
            let text = script(*shell);
            assert!(text.contains("PLZ_OUTPUT_FILE"), "{shell:?}");
            assert!(text.contains("PLZ_SHELL_INTEGRATION"), "{shell:?}");
        }
    }

    #[test]
    fn scripts_handle_both_protocol_verbs() {
        for shell in REAL_SHELLS {
            let text = script(*shell);
            assert!(text.contains("run"), "{shell:?} does not handle run");
            assert!(text.contains("buffer"), "{shell:?} does not handle buffer");
        }
    }

    #[test]
    fn scripts_clean_up_the_temporary_file() {
        for shell in [Shell::Zsh, Shell::Bash, Shell::Fish] {
            assert!(
                script(shell).contains("rm -f"),
                "{shell:?} leaves litter in /tmp"
            );
        }
        assert!(script(Shell::Powershell).contains("Remove-Item"));
    }

    #[test]
    fn buffer_uses_the_native_mechanism_of_each_shell() {
        assert!(script(Shell::Zsh).contains("print -z"));
        assert!(script(Shell::Fish).contains("commandline -r"));
        assert!(script(Shell::Powershell).contains("PSConsoleReadLine]::Insert"));
    }

    #[test]
    fn run_adds_the_command_to_shell_history() {
        // Otherwise the executed command cannot be found with Ctrl+R.
        assert!(script(Shell::Zsh).contains("print -s"));
        assert!(script(Shell::Bash).contains("history -s"));
        assert!(script(Shell::Powershell).contains("AddToHistory"));
    }

    #[test]
    fn cmd_output_is_commented_out_so_it_cannot_be_executed() {
        // The output may end up in a .bat file, so it must be inert.
        for line in script(Shell::Cmd).lines().filter(|l| !l.trim().is_empty()) {
            assert!(
                line.starts_with("@rem"),
                "executable line in the placeholder: {line}"
            );
        }
    }

    #[test]
    fn hook_args_are_accepted_by_the_cli() {
        // The suggestion is meant to be copy-pasted, so every value we hand out
        // has to parse as the `plz hook <shell>` argument.
        use clap::ValueEnum;
        for kind in [
            ShellKind::Zsh,
            ShellKind::Bash,
            ShellKind::Fish,
            ShellKind::PowerShell,
        ] {
            let arg = hook_arg(kind).unwrap_or_else(|| panic!("{kind:?} has a wrapper"));
            assert!(Shell::from_str(arg, true).is_ok(), "{arg}");
        }
    }

    #[test]
    fn shells_without_a_wrapper_get_no_hook_arg() {
        for kind in [ShellKind::Nushell, ShellKind::Cmd, ShellKind::Posix] {
            assert!(hook_arg(kind).is_none(), "{kind:?}");
        }
    }

    #[test]
    fn install_hints_name_the_right_config_file() {
        assert!(install_hint(Shell::Zsh).contains(".zshrc"));
        assert!(install_hint(Shell::Bash).contains(".bashrc"));
        assert!(install_hint(Shell::Fish).contains("conf.d"));
        assert!(install_hint(Shell::Powershell).contains("PROFILE"));
    }

    #[test]
    fn startup_lines_call_the_binary_rather_than_embedding_the_script() {
        // This is what makes the wrapper follow the binary through upgrades:
        // an embedded copy is a snapshot and goes stale.
        for (shell, arg) in [
            (Shell::Zsh, "zsh"),
            (Shell::Bash, "bash"),
            (Shell::Fish, "fish"),
            (Shell::Powershell, "powershell"),
        ] {
            let line = startup_line(shell).unwrap_or_else(|| panic!("{shell:?} has no line"));
            assert!(
                line.contains(&format!("plz hook {arg}")),
                "{shell:?}: {line}"
            );
        }
    }

    #[test]
    fn the_powershell_line_pipes_through_out_string() {
        // Without Out-String each line of the script is a separate pipeline
        // object and Invoke-Expression evaluates only the first.
        let line = startup_line(Shell::Powershell).unwrap();
        assert!(line.contains("Out-String"), "{line}");
        assert!(line.contains("Invoke-Expression"), "{line}");
    }

    #[test]
    fn cmd_has_no_startup_line() {
        assert!(startup_line(Shell::Cmd).is_none());
    }

    #[test]
    fn install_hints_write_the_same_line_that_install_does() {
        // Otherwise the documented command and `--install` drift apart and one
        // of the two silently stops working.
        for shell in REAL_SHELLS {
            let line = startup_line(*shell).unwrap();
            assert!(
                install_hint(*shell).contains(line),
                "{shell:?}: hint `{}` does not contain `{line}`",
                install_hint(*shell)
            );
        }
    }
}
