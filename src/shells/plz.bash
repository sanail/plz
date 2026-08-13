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
  local outfile
  outfile="$(mktemp -t plz.XXXXXX)" || return 1

  PLZ_OUTPUT_FILE="$outfile" PLZ_SHELL_INTEGRATION=bash command plz "$@"
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
