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

Ask in any language you like: `plz` follows your system language, and writes
each explanation in it whatever language the request itself was in.

## Installation

Four routes to the same binary — take whichever fits your machine.

### Homebrew (macOS, Linux)

```sh
brew install sanail/tap/plz
```

### Install script (macOS, Linux)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sanail/plz/releases/latest/download/plz-installer.sh | sh
```

The script downloads a prebuilt binary, puts it in `~/.local/bin` and adds that
directory to your `PATH`.

### Install script (Windows)

```powershell
powershell -c "irm https://github.com/sanail/plz/releases/latest/download/plz-installer.ps1 | iex"
```

The same thing for Windows: a prebuilt binary in `~/.local/bin`, with that
directory added to your `PATH`.

### By hand

Take the archive for your platform from the
[releases page](https://github.com/sanail/plz/releases): on Apple Silicon
`plz-aarch64-apple-darwin.tar.xz`, on an Intel Mac
`plz-x86_64-apple-darwin.tar.xz` — `uname -m` says which you are on —
`plz-x86_64-unknown-linux-gnu.tar.xz` on Linux,
`plz-x86_64-pc-windows-msvc.zip` on Windows. Each archive holds the binary, this
README and the license; `sha256.sum` in the release covers them all.

On macOS and Linux:

```sh
tar -xf plz-aarch64-apple-darwin.tar.xz
cd plz-aarch64-apple-darwin
xattr -c plz                      # macOS only
sudo mv plz /usr/local/bin/
```

`/usr/local/bin` is on the default `PATH` on both systems. Without `sudo`, move
it to `~/.local/bin` instead and add that directory to `PATH` yourself.

The `xattr` line matters only on macOS: a file the browser downloaded carries a
quarantine attribute, and an unsigned binary that carries it refuses to start.
Downloads made with `curl` never get the attribute, so the line is a no-op then.

On Windows, unpack the zip, put `plz.exe` wherever you keep such things and add
that directory to `PATH` through "Edit environment variables for your account".
The binary is unsigned, so the first run may raise a SmartScreen warning.

## Setting up the model

However you installed it, the binary has no model to ask until you give it one.
Run the first-time setup:

```sh
plz config init
```

It asks for a provider, a model and an API key. Any endpoint compatible with the
OpenAI Chat Completions API works; ready-made presets cover DeepSeek, OpenAI,
OpenRouter, Ollama (local, no key) and a custom address.

That is all `plz` needs to work. [Configuration](#configuration) below covers
where the file lives, every field in it, and how to keep the key in an
environment variable instead.

## Running in your current shell

This step is optional — skip it unless you want `cd`, `export` and `source` to
stick.

`plz` works with no extra setup, but out of the box it runs the command in a
**child** process. That means `cd`, `export`, `source` and venv activation will
not affect your session — a child process cannot change its parent shell's
state. That is how processes work in the OS, not a shortcoming of the tool.

To make those commands work, install the wrapper:

```sh
plz hook zsh --install
plz hook bash --install
plz hook fish --install
plz hook powershell --install
```

It shows the file and the line, asks, and writes nothing unless you answer `y` —
a refusal does not even create the file. `plz -y hook <shell> --install` answers
for you, for scripted setups. By hand it is one line:

```sh
echo 'eval "$(plz hook zsh)"'  >> ~/.zshrc                       # zsh
echo 'eval "$(plz hook bash)"' >> ~/.bashrc                      # bash
echo 'plz hook fish | source' > ~/.config/fish/conf.d/plz.fish   # fish
```

```powershell
Add-Content $PROFILE 'plz hook powershell | Out-String | Invoke-Expression'
```

The line calls the binary rather than holding a copy of the script, so the
wrapper is regenerated at every shell start and follows the binary through
upgrades. `Out-String` is what joins the script back into one string for
PowerShell; without it `Invoke-Expression` sees only the first line.

The wrapper is a function with the same name as the binary. It shadows the
binary in PATH, receives the chosen command through a temporary file, and runs
it in your current shell. It calls the binary itself via `command plz`, so there
is no recursion.

There is no wrapper for `cmd.exe`: it has no functions, and `doskey` macros can
neither branch nor read a file. There `plz` runs in child-process mode and warns
you when the chosen command changes session state.

### PowerShell: the execution policy

On Windows the default execution policy is `Restricted`, which means PowerShell
refuses to run *any* `.ps1` file — including your own `$PROFILE`. The line above
lands in the profile and then never runs, with no hint as to why beyond an error
at startup. Allow local scripts once, for your user only:

```powershell
Set-ExecutionPolicy RemoteSigned -Scope CurrentUser
```

This needs no administrator rights and writes to your own registry key. Under
`RemoteSigned` scripts you wrote or created locally run, while scripts downloaded
from the internet still need a signature — that is the setting Microsoft ships as
the default on Windows Server, and the one every profile-based tool (starship,
zoxide, oh-my-posh) asks for.

`plz hook powershell --install` checks the policy after writing the line and
offers to run that command for you. As with the profile itself, it does nothing
without a `y`.

Windows PowerShell 5.1 and PowerShell 7 keep separate profiles *and* separate
policy settings, so run `--install` from the one you actually use — it asks that
shell where its `$PROFILE` is instead of guessing.

## Modes

**Direct request.** `plz "describe your task"` shows the suggestions under your
prompt; pick one with a digit or the arrow keys.

**Interactive mode.** `plz` with no arguments opens a full-screen interface:
type a query, wait for the reply, pick from the list. `Ctrl+R` retries the same
query, `Ctrl+N` starts a new one without leaving the mode.

### Keys

Editing the query in interactive mode:

| Key | Action |
|---|---|
| `←` / `→` | move the cursor by a character |
| `Ctrl+←` / `Ctrl+→`, `Alt+←` / `Alt+→`, `Alt+B` / `Alt+F` | move it by a word |
| `Home` / `End`, `Ctrl+A` / `Ctrl+E` | jump to the start or the end |
| `Backspace` / `Delete` | delete on either side of the cursor |
| `Ctrl+W`, `Alt+Backspace` | delete the word before the cursor |
| `Ctrl+U` / `Ctrl+K` | delete to the start or to the end of the line |
| `Enter` | send the query |

Each movement answers to several keys because terminals differ in what they
send: macOS Terminal has no `Ctrl+←`, a bare Windows console has no `Alt+B`.
`Ctrl+A` and `Ctrl+E` work everywhere.

Pasting into the query keeps its line breaks as spaces — the query is a single
line — and does not send it early. Windows consoles hand a paste over key by
key instead, so there a line break inside the pasted text sends the query.

Choosing a suggestion:

| Key | Action |
|---|---|
| `1`–`9` | pick a suggestion and run it immediately |
| `↑`/`↓`, `k`/`j` | move through the list |
| `Enter` | run the selected suggestion |
| `Tab` / `e` | hand the command over for editing (or copy) |
| `c` | copy the command to the clipboard and exit |
| `Esc` / `q` / `Ctrl+C` / `Ctrl+D` | cancel, exiting with code 130 |

`Tab` puts the command in front of you unrun, and it needs the wrapper installed.
zsh (`print -z`) and fish (`commandline -r`) drop it straight into the prompt.
PowerShell cannot: PSReadLine clears its editing buffer at the start of every
prompt, so nothing written there from outside its own read loop survives — the
command goes into the history instead, and `↑` recalls it for editing. Everywhere
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

Subcommands: `plz config init|path|show|edit`, `plz hook <shell> [--install]`.

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
language          = "auto" # interface language; "auto" follows the system
```

On Unix the file is created with mode `0600`. You need not keep the key in it at
all: the resolution order is `PLZ_API_KEY`, then the preset's variable
(`DEEPSEEK_API_KEY`, `OPENAI_API_KEY`, …), then the config field.

`PLZ_LANG` overrides `language` for a single run, the same way `PLZ_API_KEY`
overrides the key.

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

None of the installation routes above involve a Rust toolchain — they all fetch
a prebuilt binary. To build from source instead (needs Rust 1.85+):

```sh
cargo install --path .
```

```sh
cargo test                              # unit tests, no network needed
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Builds and tests on macOS, Linux and Windows. TLS is `rustls`, so no system
OpenSSL is needed to build on any of them.

What changed between versions is in [CHANGELOG.md](CHANGELOG.md).

## License

MIT
