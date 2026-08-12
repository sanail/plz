# plz wrapper for zsh. To install:
#   echo 'eval "$(plz hook zsh)"' >> ~/.zshrc
#
# The function shares its name with the binary and shadows it: in zsh, functions
# take precedence over commands found in PATH. The binary itself is called via
# `command plz`, so there is no recursion.
plz() {
  local outfile
  outfile="$(mktemp -t plz.XXXXXX)" || return 1

  PLZ_OUTPUT_FILE="$outfile" PLZ_SHELL_INTEGRATION=zsh command plz "$@"
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
