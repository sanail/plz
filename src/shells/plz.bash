# plz wrapper for bash. To install:
#   plz hook bash --install
# or by hand:
#   echo 'eval "$(plz hook bash)"' >> ~/.bashrc
#
# The function shadows the binary of the same name, which it calls via
# `command plz`.
#
# Unlike zsh, bash has no equivalent of `print -z` for putting a command into
# the prompt buffer. So plz does not offer prompt insertion in bash — the Tab
# key copies the command to the clipboard there instead.
plz() {
  local outfile winfile
  outfile="$(mktemp -t plz.XXXXXX)" || return 1

  # On Windows plz is a native binary, and Cygwin — unlike MSYS2 and Git Bash —
  # passes POSIX paths to such a child untranslated: it would write the answer
  # to C:\tmp\plz.XXXXXX while this function waited for it in /tmp. The POSIX
  # name stays for the shell's own reads and for rm.
  winfile="$outfile"
  if command -v cygpath >/dev/null 2>&1; then
    winfile="$(cygpath -w "$outfile")"
  fi

  PLZ_OUTPUT_FILE="$winfile" PLZ_SHELL_INTEGRATION=bash command plz "$@"
  local exit_code=$?

  if [[ -s "$outfile" ]]; then
    local verb command_text
    verb="$(head -n1 "$outfile")"
    command_text="$(tail -n +2 "$outfile")"

    case "$verb" in
      run)
        history -s "$command_text"
        eval "$command_text"
        exit_code=$?
        ;;
      buffer)
        # There is no native prompt insertion: add it to history so it is one
        # arrow-up away, and print it.
        history -s "$command_text"
        printf '%s\n' "$command_text"
        ;;
    esac
  fi

  rm -f "$outfile"
  return $exit_code
}
