# plz wrapper for PowerShell. To install:
#   plz hook powershell --install
# or by hand:
#   Add-Content $PROFILE 'plz hook powershell | Out-String | Invoke-Expression'
#
# The function shadows the executable of the same name. To call the executable
# rather than itself, it looks the file up explicitly as an Application — the
# equivalent of `command plz`.
function plz {
    $binary = (Get-Command plz -CommandType Application -ErrorAction SilentlyContinue |
               Select-Object -First 1).Source
    if (-not $binary) {
        Write-Error 'The plz executable was not found in PATH.'
        return
    }

    $outfile = [System.IO.Path]::GetTempFileName()
    $previousFile = $env:PLZ_OUTPUT_FILE
    $previousShell = $env:PLZ_SHELL_INTEGRATION

    try {
        $env:PLZ_OUTPUT_FILE = $outfile
        $env:PLZ_SHELL_INTEGRATION = 'powershell'

        & $binary @args
        $exitCode = $LASTEXITCODE

        $content = Get-Content -LiteralPath $outfile -ErrorAction SilentlyContinue
        # An empty file means the user cancelled.
        if ($content) {
            $verb = $content[0]
            $commandText = ($content | Select-Object -Skip 1) -join "`n"

            switch ($verb) {
                'run' {
                    # Add the command to history so Ctrl+R can find it.
                    [Microsoft.PowerShell.PSConsoleReadLine]::AddToHistory($commandText)
                    Invoke-Expression $commandText
                }
                'buffer' {
                    # PSReadLine's editing buffer cannot be reached from here:
                    # this function runs after ReadLine has returned, and the
                    # next ReadLine clears the buffer before drawing the prompt,
                    # so anything inserted into it is lost. History does survive
                    # that boundary, which leaves the command one arrow-up away.
                    Write-Host $commandText
                    [Microsoft.PowerShell.PSConsoleReadLine]::AddToHistory($commandText)
                    Write-Host 'Press the Up arrow to recall and edit it.'
                }
            }
        }

        $global:LASTEXITCODE = $exitCode
    }
    finally {
        Remove-Item -LiteralPath $outfile -ErrorAction SilentlyContinue
        $env:PLZ_OUTPUT_FILE = $previousFile
        $env:PLZ_SHELL_INTEGRATION = $previousShell
    }
}
