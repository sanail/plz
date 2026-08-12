# plz

Describe a task in plain language, get a command ready for your OS and shell.

```console
$ plz "find the 10 largest files in this folder"
❯ 1  du -ah . | sort -rh | head -n 10
  2  find . -type f -exec du -h {} + | sort -rh | head -n 10
  3  ls -lAS | head -n 11

  files and directories by size, largest first
  1-9/↑↓ select · Enter run · Tab to edit · c copy · Esc cancel
```

The same task is solved differently under zsh on macOS, PowerShell, and cmd on
Windows. So `plz` tells the model which OS, OS version, architecture and shell
it was launched from, and gets back a command that runs unedited.

Ask in any language you like: `plz` writes each explanation in the language of
your request.

## Installation

No prebuilt binaries yet; build from source (needs Rust 1.85+):

```sh
cargo install --path .
```

Then run the first-time setup:

```sh
plz config init
```

It asks for a provider, a model and an API key. Any endpoint compatible with the
OpenAI Chat Completions API works; ready-made presets cover DeepSeek, OpenAI,
OpenRouter, Ollama (local, no key) and a custom address.

## Running in your current shell

`plz` works with no extra setup, but out of the box it runs the command in a
**child** process. That means `cd`, `export`, `source` and venv activation will
not affect your session — a child process cannot change its parent shell's
state. That is how processes work in the OS, not a shortcoming of the tool.

To make those commands work, install the wrapper:

```sh
echo 'eval "$(plz hook zsh)"'  >> ~/.zshrc            # zsh
echo 'eval "$(plz hook bash)"' >> ~/.bashrc           # bash
plz hook fish > ~/.config/fish/conf.d/plz.fish        # fish
plz hook powershell >> $PROFILE                       # PowerShell
```

The wrapper is a function with the same name as the binary. It shadows the
binary in PATH, receives the chosen command through a temporary file, and runs
it in your current shell. It calls the binary itself via `command plz`, so there
is no recursion.

There is no wrapper for `cmd.exe`: it has no functions, and `doskey` macros can
neither branch nor read a file. There `plz` runs in child-process mode and warns
you when the chosen command changes session state.

## Modes

**Direct request.** `plz "describe your task"` shows the suggestions under your
prompt; pick one with a digit or the arrow keys.

**Interactive mode.** `plz` with no arguments opens a full-screen interface:
type a query, wait for the reply, pick from the list. `Ctrl+R` retries the same
query, `Ctrl+N` starts a new one without leaving the mode.

### Keys

| Key | Action |
|---|---|
| `1`–`9` | pick a suggestion and run it immediately |
| `↑`/`↓`, `k`/`j` | move through the list |
| `Enter` | run the selected suggestion |
| `Tab` / `e` | insert into the prompt for editing (or copy) |
| `c` | copy the command to the clipboard and exit |
| `Esc` / `q` / `Ctrl+C` / `Ctrl+D` | cancel, exiting with code 130 |

`Tab` inserts the command into the prompt only where the shell supports it —
zsh (`print -z`), fish (`commandline -r`) and PowerShell
(`PSConsoleReadLine::Insert`) — and only with the wrapper installed. Everywhere
else that key copies the command instead.

## Flags

```
plz [OPTIONS] [TASK DESCRIPTION...]

  -n, --count <N>     how many suggestions to request (1-9)
      --model <NAME>  use a different model instead of the configured one
      --dry-run       only show the suggestions, run nothing
  -y, --yes           do not ask for confirmation on risky commands
      --json          print the suggestions as JSON (for scripts)
```

Flags go **before** the task. Everything after the task is read as part of it —
which is what lets you ask about a flag without quoting anything:

```sh
plz --dry-run "clear the cache"      # a flag, so it goes first
plz what does git push --force do    # --force is part of the question
```

A task whose first word is `config` or `hook` collides with the subcommand of the
same name. Quote it, or put `--` in front:

```sh
plz "config nginx as a reverse proxy"
plz -- config nginx as a reverse proxy
```

Subcommands: `plz config init|path|show|edit`, `plz hook <shell>`.

`plz config show` prints the configuration (with the key masked) plus the
detected environment — the first thing to look at if commands come back for the
wrong shell.

## Configuration

`plz config path` prints the location:

* Linux — `~/.config/plz/config.toml`
* macOS — `~/Library/Application Support/plz/config.toml`
* Windows — `%APPDATA%\plz\config.toml`

```toml
[provider]
preset   = "deepseek"
base_url = "https://api.deepseek.com/v1"
model    = "deepseek-v4-flash"
api_key  = "sk-..."

[behavior]
suggestions       = 3      # how many suggestions to request
confirm_dangerous = true   # ask before running risky commands
timeout_secs      = 30
send_cwd          = true   # send the working directory to the model
json_mode         = true   # ask for response_format = json_object
disable_thinking  = true   # ask the model not to reason before answering
```

On Unix the file is created with mode `0600`. You need not keep the key in it at
all: the resolution order is `PLZ_API_KEY`, then the preset's variable
(`DEEPSEEK_API_KEY`, `OPENAI_API_KEY`, …), then the config field.

If your endpoint answers 400 on `response_format`, set `json_mode = false`; the
format is then requested through the prompt text alone.

`disable_thinking = true` adds `thinking = {"type": "disabled"}` to the request.
It is meant for models that reason by default: one shell command is not worth a
chain of reasoning, and the wait and the tokens are paid for either way. The
field is DeepSeek's, not part of the OpenAI API, so it is off unless the preset
turns it on — an endpoint that does not know it will answer 400.

## About safety

Before a command runs it is checked against a set of heuristics: recursive
deletion of critical directories, `dd` onto a device, `mkfs`, `curl | sh`, fork
bombs, force-pushes to shared branches, `DROP TABLE`, and their PowerShell and
cmd equivalents. On a match `plz` shows a warning and asks `y/N`, defaulting to
no.

**These are heuristics, not a sandbox.** The list is knowingly incomplete and
trivial to slip past. It is meant to catch a mistyped digit in the suggestion
list, not to defend against a malicious model. You see the command before it
runs — read it.

## What is sent to the provider

The task text, the OS name and version, the architecture, the shell type, and —
when `send_cwd = true` — the path to your working directory. File contents,
shell history and environment variables are never sent. A directory path can
carry project and client names; turn it off with `send_cwd = false`.

## Development

```sh
cargo test                              # unit tests, no network needed
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Builds and tests on macOS, Linux and Windows. TLS is `rustls`, so no system
OpenSSL is needed to build on any of them.

English is the project language: code, comments, messages, prompts and docs are
all written in English. Interface translations will come with i18n later; today
the only language that varies is the one the model explains commands in, which
follows your request.

## License

MIT
