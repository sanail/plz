# plz wrapper for zsh. To install:
#   plz hook zsh --install
# or by hand:
#   echo 'eval "$(plz hook zsh)"' >> ~/.zshrc
#
# The function shares its name with the binary and shadows it: in zsh, functions
# take precedence over commands found in PATH. The binary itself is called via
# `command plz`, so there is no recursion.
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

  PLZ_OUTPUT_FILE="$winfile" PLZ_SHELL_INTEGRATION=zsh command plz "$@"
  local exit_code=$?

  # An empty file means the user cancelled, so there is nothing to do.
  if [[ -s "$outfile" ]]; then
    local verb command_text
    verb="$(head -n1 "$outfile")"
    command_text="$(tail -n +2 "$outfile")"

    case "$verb" in
      run)
        # print -s adds the command to history so Ctrl+R can find it.
        print -s -- "$command_text"
        eval "$command_text"
        exit_code=$?
        ;;
      buffer)
        # print -z puts the command in the prompt buffer: it is visible, it can
        # be edited, and it only runs when you press Enter.
        print -z -- "$command_text"
        ;;
    esac
  fi

  rm -f "$outfile"
  return $exit_code
}
