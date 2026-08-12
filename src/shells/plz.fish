# plz wrapper for fish. To install:
#   plz init fish > ~/.config/fish/conf.d/plz.fish
#
# The function shadows the binary of the same name, which it calls via
# `command plz`.
function plz --description 'Describe a task in plain language, get a ready command'
    set -l outfile (mktemp -t plz.XXXXXX)
    or return 1

    PLZ_OUTPUT_FILE=$outfile PLZ_SHELL_INTEGRATION=fish command plz $argv
    # $status must be read immediately after the command: anything else overwrites it.
    set -l exit_code $status

    if test -s $outfile
        set -l verb (head -n1 $outfile)
        # string collect preserves the newlines in multi-line commands.
        set -l command_text (tail -n +2 $outfile | string collect)

        switch $verb
            case run
                eval $command_text
                set exit_code $status
            case buffer
                # Put the command in the prompt buffer: visible and editable.
                commandline -r -- $command_text
        end
    end

    rm -f $outfile
    return $exit_code
end
