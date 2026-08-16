# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-08-16

### Added

- The interface follows the system language, with English, Russian, Spanish,
  French and German supported out of the box. Anything else — an unsupported
  language, a locale that cannot be read — falls back to English. Set
  `language` under `[behavior]` to pin one, or `PLZ_LANG` for a single run.
- The confirmation prompt accepts the local spelling of yes alongside `y`.
- The query field of interactive mode is a real line editor. Until now it only
  appended characters and erased the last one, so a typo at the start of a long
  query meant retyping the rest. It now has a cursor: `←`/`→`, `Home`/`End`
  (`Ctrl+A`/`Ctrl+E`), word movements on `Ctrl+←`/`Ctrl+→`, `Alt+←`/`Alt+→` and
  `Alt+B`/`Alt+F`, `Delete`, `Ctrl+W`, `Ctrl+U` and `Ctrl+K`. Every movement
  answers to several keys because terminals disagree about which ones they send.
- Pasted text goes in as a single edit, with its line breaks turned into spaces,
  so a multi-line paste no longer sends the query halfway through. Windows
  consoles do not report a paste to the program and keep handing it over key by
  key, so there it behaves as before.

### Changed

- Each explanation is now written in the interface language rather than in the
  language of the request. Guessing from the request left the explanation in a
  different language from everything around it whenever the task was typed in
  English by a non-English speaker.

### Fixed

- On layouts that need AltGr — `@`, `€`, `ą` — those characters never reached
  the query in interactive mode on Windows: a console reports AltGr as
  Ctrl+Alt, and every Ctrl combination was discarded as a shortcut.

## [0.1.3] - 2026-08-14

### Fixed

- Under Cygwin and Git Bash the chosen command ran in WSL. The shell was
  launched by name, and Windows searches its system directory before the
  inherited `PATH` — there `bash.exe` is WSL's launcher. `plz` now runs the
  binary it was started from.

## [0.1.2] - 2026-08-14

### Fixed

- The note that a command changed shell state but ran in a child process never
  appeared in PowerShell. The check knew only the POSIX spellings (`cd`,
  `export`), while the model answers `Set-Location ..` or `$env:FOO = 'bar'`, so
  the run looked like `plz` had done nothing at all. Cmdlets, their aliases and
  `Env:` assignments now count, and the comparison ignores casing.
- `Tab` in PowerShell glued the command onto the line just submitted —
  `plz go to parent foldercd ..` — and then lost it: `PSConsoleReadLine::Insert`
  writes into a buffer the next prompt clears. The command now goes into
  PSReadLine's history, one `↑` away from editing.

## [0.1.1] - 2026-08-13

### Added

- `plz hook <shell> --install` writes the wrapper line into the shell's startup
  file for you. It shows the file and the line, asks, and writes nothing unless
  you answer `y` — a refusal does not even create the file. `plz -y hook <shell>
  --install` answers for scripted setups.

### Fixed

- The PowerShell wrapper never loaded. The line to add is now
  `plz hook powershell | Out-String | Invoke-Expression`: without `Out-String`,
  `Invoke-Expression` sees only the script's first line. The fish line became
  `plz hook fish | source` for the same reason — it now calls the binary at every
  shell start instead of holding a stale copy of the script.
- Explanations came back in an arbitrary language, Chinese among them, when the
  task was too short or too neutral to place. The prompt now names English as the
  fallback when the language of the request is not obvious.

### Changed

- The readme covers the four installation routes separately, splits setting up
  the model into its own step, and documents the PowerShell execution policy.

## [0.1.0] - 2026-08-12

First release.

### Added

- Describe a task in plain language and get runnable shell commands back. The
  OS, OS version, architecture, shell and — optionally — the working directory
  go to the model as context, so the command runs unedited on the machine that
  asked for it.
- Each explanation is written in the language of the request.
- Direct mode (`plz "task"`) lists the suggestions under the prompt; interactive
  mode (`plz` with no arguments) opens a full-screen interface where `Ctrl+R`
  retries a query and `Ctrl+N` starts a new one. Pick with digits or the arrow
  keys, run with `Enter`, copy with `c`, or `Tab` to edit the command first.
- Any endpoint compatible with the OpenAI Chat Completions API works.
  `plz config init` walks through presets for DeepSeek, OpenAI, OpenRouter,
  Ollama (local, no key) and a custom address; `plz config path|show|edit`
  manage the file afterwards. The config file is created with mode 0600, and the
  API key can live in an environment variable instead.
- Safety heuristics run before a command executes — recursive deletion of
  critical directories, `dd` onto a device, `mkfs`, `curl | sh`, fork bombs,
  force-pushes to shared branches, `DROP TABLE`, and their PowerShell and cmd
  equivalents. A match asks `y/N`, defaulting to no.
- `plz hook zsh|bash|fish|powershell` prints a shell wrapper so `cd`, `export`
  and `source` affect the current shell instead of a child process. There is no
  wrapper for `cmd.exe`, which instead warns when the chosen command changes
  session state.
- Flags `-n/--count`, `--model`, `--dry-run`, `-y/--yes` and `--json`, honoured
  in interactive mode as well as direct mode.
- Prebuilt binaries for macOS (Intel and Apple silicon), Linux and Windows, with
  shell and PowerShell installers and a Homebrew formula.

[0.2.0]: https://github.com/sanail/plz/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/sanail/plz/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/sanail/plz/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/sanail/plz/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/sanail/plz/releases/tag/v0.1.0
