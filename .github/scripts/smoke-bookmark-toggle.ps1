param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,
    [Parameter(Mandatory = $true)]
    [string]$ScratchRoot
)

$ErrorActionPreference = "Stop"
$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
if (Test-Path -LiteralPath $ScratchRoot) {
    Remove-Item -LiteralPath $ScratchRoot -Recurse -Force
}

$previousLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = $ScratchRoot
try {
    foreach ($argument in @(
        "--native-bookmark-toggle-add-smoke",
        "--native-bookmark-toggle-remove-smoke"
    )) {
        $process = Start-Process -FilePath $resolvedExecutable -ArgumentList $argument -PassThru -NoNewWindow
        if (-not $process.WaitForExit(60000)) {
            $process.Kill()
            throw "bookmark toggle smoke $argument timed out"
        }
        if ($process.ExitCode -ne 0) {
            throw "bookmark toggle smoke $argument failed with exit code $($process.ExitCode)"
        }
    }
} finally {
    $env:LOCALAPPDATA = $previousLocalAppData
}
