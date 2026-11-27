[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$UpdateEmbedded,
    [string]$DataDirectory,
    [string]$CatalogDirectory,
    [string]$OriginalsDirectory,
    [string]$Runner,
    [string]$Report,
    [string]$PythonExe
)

$ErrorActionPreference = 'Stop'
$weeklyRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$weeklyJdk = Join-Path $weeklyRoot 'toolchain/jdk-21.0.12.1+1'
if (!$DryRun -and !(Test-Path -LiteralPath (Join-Path $weeklyJdk 'bin/java.exe') -PathType Leaf)) {
    throw 'The project JDK 21 is required in toolchain/jdk-21.0.12.1+1.'
}

$weeklyPythonArgs = @()
if (!$PythonExe) {
    $bundled = Get-ChildItem -LiteralPath (Join-Path $weeklyRoot 'toolchain') -Directory -Filter 'python*' -ErrorAction SilentlyContinue |
        ForEach-Object { Join-Path $_.FullName 'python.exe' } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if ($bundled) {
        $PythonExe = $bundled
    } elseif ($command = Get-Command python -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1) {
        $PythonExe = $command.Source
    } elseif ($command = Get-Command py -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1) {
        $PythonExe = $command.Source
        $weeklyPythonArgs += '-3'
    } else {
        throw 'Python 3 was not found. Set -PythonExe to a Python 3 executable.'
    }
}

$weeklyPythonArgs += (Join-Path $PSScriptRoot 'swf-patches/sync_weekly.py')
foreach ($pair in @(
    @('--data-dir', $DataDirectory), @('--catalog-dir', $CatalogDirectory),
    @('--originals-dir', $OriginalsDirectory), @('--runner', $Runner), @('--report', $Report)
)) {
    if ($pair[1]) { $weeklyPythonArgs += @($pair[0], $pair[1]) }
}
if ($DryRun) { $weeklyPythonArgs += '--dry-run' }
if ($UpdateEmbedded) { $weeklyPythonArgs += '--update-embedded' }

& $PythonExe @weeklyPythonArgs
exit $LASTEXITCODE
