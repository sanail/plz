//! Building the request to the model and parsing its reply.

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::context::{Context, ShellKind};
use crate::suggestion::Suggestion;

/// System prompt: the role, the environment and the required reply format.
pub fn system_prompt(ctx: &Context, count: usize) -> String {
    let mut prompt = String::new();

    prompt.push_str(
        "You turn a task described in plain language into ready-to-run \
         terminal commands.\n\n",
    );

    prompt.push_str("User's environment:\n");
    prompt.push_str(&format!("- OS: {}", ctx.os));
    if let Some(version) = &ctx.os_version {
        prompt.push_str(&format!(" ({version})"));
    }
    prompt.push('\n');
    prompt.push_str(&format!("- Architecture: {}\n", ctx.arch));
    prompt.push_str(&format!("- Shell: {}\n", ctx.shell));
    if let Some(cwd) = &ctx.cwd {
        prompt.push_str(&format!("- Working directory: {cwd}\n"));
    }

    if let Some(hint) = shell_hint(ctx.shell.kind) {
        prompt.push_str(&format!("- Note: {hint}\n"));
    }

    prompt.push_str(&format!(
        "\nRules:\n\
         1. Give exactly {count} command suggestion(s), most direct first, then more \
            specialised ones. If only one sensible option exists, give one.\n\
         2. Every command must run in {shell} on {os} unchanged.\n\
         3. Do not invent flags or utilities. If a command needs a tool that may not \
            be installed, say so in the explanation.\n\
         4. Do not wrap commands in markdown and do not prefix them with `$`.\n\
         5. Keep each explanation to one short phrase, and write it in the same \
            language the task was written in. If that language is not obvious — \
            a short task, bare command names, or mixed languages — write the \
            explanation in English.\n\
         6. Reply with a JSON object ONLY, with no prose around it.\n\n",
        count = count,
        shell = ctx.shell.kind.label(),
        os = ctx.os,
    ));

    prompt.push_str(
        "Reply format:\n\
         {\"suggestions\":[{\"command\":\"...\",\"explanation\":\"...\"}]}\n",
    );

    prompt
}

/// Shell-specific traps that models routinely forget about.
fn shell_hint(kind: ShellKind) -> Option<&'static str> {
    match kind {
        ShellKind::PowerShell => Some(
            "this is PowerShell — use cmdlets (Get-ChildItem, Select-Object) \
             or their aliases, not GNU coreutils",
        ),
        ShellKind::Cmd => Some(
            "this is cmd.exe — only its built-ins and Windows utilities are available; \
             PowerShell and Unix syntax do not work here",
        ),
        ShellKind::Fish => Some(
            "this is fish — the syntax differs from POSIX: no `$(...)` inside quotes, \
             and variables are set with `set`, not `VAR=value`",
        ),
        ShellKind::Nushell => Some(
            "this is nushell — a language of its own with typed pipelines; \
             POSIX syntax does not work here",
        ),
        _ => None,
    }
}

/// The user message carrying the task itself.
pub fn user_prompt(task: &str) -> String {
    format!("Task: {}", task.trim())
}

#[derive(Debug, Deserialize)]
struct SuggestionsPayload {
    #[serde(default)]
    suggestions: Vec<Suggestion>,
}

/// Parse the model's reply into a list of suggestions.
///
/// Models routinely wrap the JSON in a ``` block even when told not to, and
/// sometimes add a sentence before or after the object, so we try three
/// strategies in order of increasing input messiness.
pub fn parse_suggestions(raw: &str) -> Result<Vec<Suggestion>> {
    let candidates = [
        Some(raw.trim().to_string()),
        strip_code_fence(raw),
        extract_balanced_object(raw),
    ];

    for candidate in candidates.into_iter().flatten() {
        if let Some(suggestions) = try_parse(&candidate) {
            return Ok(suggestions);
        }
    }

    Err(anyhow!(
        "could not parse the model's reply as JSON. The reply was:\n{}",
        truncate(raw, 500)
    ))
}

fn try_parse(text: &str) -> Option<Vec<Suggestion>> {
    // The documented shape: {"suggestions": [...]}
    if let Ok(payload) = serde_json::from_str::<SuggestionsPayload>(text) {
        if !payload.suggestions.is_empty() {
            return Some(clean(payload.suggestions));
        }
    }
    // Some models return a bare array despite the schema in the prompt.
    if let Ok(list) = serde_json::from_str::<Vec<Suggestion>>(text) {
        if !list.is_empty() {
            return Some(clean(list));
        }
    }
    None
}

/// Strip the decoration models like to add inside the command itself.
fn clean(suggestions: Vec<Suggestion>) -> Vec<Suggestion> {
    suggestions
        .into_iter()
        .map(|mut s| {
            s.command = s
                .command
                .trim()
                .trim_start_matches("$ ")
                .trim_start_matches("> ")
                .trim()
                .to_string();
            s.explanation = s.explanation.trim().to_string();
            s
        })
        .filter(|s| !s.command.is_empty())
        .collect()
}

/// Pull the body out of a markdown ```...``` block.
fn strip_code_fence(raw: &str) -> Option<String> {
    let start = raw.find("```")?;
    let after_fence = &raw[start + 3..];
    // Drop the language tag (```json) along with its newline.
    let body_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
    let body = &after_fence[body_start..];
    let end = body.find("```")?;
    Some(body[..end].trim().to_string())
}

/// Find the first balanced `{...}` object in the text.
///
/// Braces are counted with string literals and escapes in mind: otherwise a `}`
/// inside a command (`awk '{print $1}'`, say) would end the object too early.
fn extract_balanced_object(raw: &str) -> Option<String> {
    let bytes: Vec<char> = raw.chars().collect();
    let start = bytes.iter().position(|&c| c == '{')?;

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, &ch) in bytes[start..].iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    let end = start + offset + 1;
                    return Some(bytes[start..end].iter().collect());
                }
            }
            _ => {}
        }
    }
    None
}

fn truncate(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Shell;

    fn ctx(kind: ShellKind) -> Context {
        Context {
            os: "macOS".into(),
            os_version: Some("15.0".into()),
            arch: "aarch64".into(),
            shell: Shell {
                kind,
                name: kind.label().into(),
                environment: None,
                path: None,
            },
            cwd: Some("/tmp/project".into()),
        }
    }

    #[test]
    fn system_prompt_carries_os_and_shell() {
        let prompt = system_prompt(&ctx(ShellKind::Zsh), 3);
        assert!(prompt.contains("macOS"));
        assert!(prompt.contains("15.0"));
        assert!(prompt.contains("zsh"));
        assert!(prompt.contains("/tmp/project"));
        assert!(prompt.contains("exactly 3"));
    }

    #[test]
    fn system_prompt_asks_for_the_task_language() {
        // Non-English users should get explanations they can actually read,
        // and the model handles that without any i18n machinery on our side.
        // Short English tasks read as language-neutral, though, so the prompt
        // names English explicitly instead of letting the model pick at random.
        let prompt = system_prompt(&ctx(ShellKind::Zsh), 3);
        assert!(prompt.contains("same language the task was written in"));
        assert!(prompt.contains("explanation in English"));
    }

    #[test]
    fn powershell_and_cmd_get_dedicated_hints() {
        assert!(system_prompt(&ctx(ShellKind::PowerShell), 1).contains("cmdlets"));
        assert!(system_prompt(&ctx(ShellKind::Cmd), 1).contains("cmd.exe"));
        // zsh needs no dedicated hint; adding one would only pad the prompt.
        assert!(!system_prompt(&ctx(ShellKind::Zsh), 1).contains("- Note:"));
    }

    #[test]
    fn parses_plain_json() {
        let raw = r#"{"suggestions":[{"command":"ls -la","explanation":"list files"}]}"#;
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].command, "ls -la");
        assert_eq!(parsed[0].explanation, "list files");
    }

    #[test]
    fn parses_json_wrapped_in_a_code_fence() {
        let raw =
            "```json\n{\"suggestions\":[{\"command\":\"pwd\",\"explanation\":\"path\"}]}\n```";
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed[0].command, "pwd");
    }

    #[test]
    fn parses_json_surrounded_by_prose() {
        let raw = "Sure! Here are some options:\n\
                   {\"suggestions\":[{\"command\":\"df -h\",\"explanation\":\"disks\"}]}\n\
                   Hope that helps.";
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed[0].command, "df -h");
    }

    #[test]
    fn parses_a_bare_array() {
        let raw = r#"[{"command":"uptime","explanation":"uptime"}]"#;
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed[0].command, "uptime");
    }

    #[test]
    fn tolerates_missing_explanation() {
        let raw = r#"{"suggestions":[{"command":"id"}]}"#;
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed[0].explanation, "");
    }

    #[test]
    fn explanations_in_any_language_survive_parsing() {
        // Rule 5 asks the model to answer in the task's language, so the parser
        // has to carry non-ASCII explanations through untouched.
        let raw = r#"{"suggestions":[{"command":"df -h","explanation":"espace disque"}]}"#;
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed[0].explanation, "espace disque");
    }

    #[test]
    fn braces_inside_a_command_do_not_truncate_the_object() {
        // The classic case: awk with braces inside a JSON string.
        let raw = "prose before\n{\"suggestions\":[{\"command\":\"ps aux | awk '{print $2}'\",\
                   \"explanation\":\"process ids\"}]}\nprose after";
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed[0].command, "ps aux | awk '{print $2}'");
    }

    #[test]
    fn escaped_quotes_do_not_break_extraction() {
        let raw = r#"here: {"suggestions":[{"command":"echo \"hi\"","explanation":"greet"}]} done"#;
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed[0].command, r#"echo "hi""#);
    }

    #[test]
    fn strips_shell_prompt_markers_from_commands() {
        let raw = r#"{"suggestions":[{"command":"$ ls -la","explanation":""}]}"#;
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed[0].command, "ls -la");
    }

    #[test]
    fn drops_entries_with_empty_commands() {
        let raw = r#"{"suggestions":[{"command":"  ","explanation":"empty"},
                     {"command":"ls","explanation":"fine"}]}"#;
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].command, "ls");
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(parse_suggestions("sorry, I can't help with that").is_err());
        assert!(parse_suggestions("").is_err());
        assert!(parse_suggestions("{ broken json").is_err());
    }

    #[test]
    fn empty_suggestion_list_is_an_error() {
        // An empty list is not a success: there would be nothing to show.
        assert!(parse_suggestions(r#"{"suggestions":[]}"#).is_err());
    }

    #[test]
    fn error_message_includes_the_raw_response() {
        let err = parse_suggestions("I cannot").unwrap_err().to_string();
        assert!(
            err.contains("I cannot"),
            "the raw reply is what explains the failure"
        );
    }
}
