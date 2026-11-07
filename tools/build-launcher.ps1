param(
    [ValidateSet('build','check','test')][string]$Action='build',
    [string]$ToolchainRoot='',
    [string]$JavaHome='',
    [string]$VendorDirectory='',
    [string]$OutputDirectory=''
)
$ErrorActionPreference='Stop'
$taskRoot=Split-Path -Parent $PSScriptRoot
$taskRea=Split-Path -Parent $taskRoot
$taskFramework=Join-Path $taskRea 'aoqi.1.03\reconstructed'
if (!$ToolchainRoot) { $ToolchainRoot=Join-Path $taskRea 'aoqi.1.03\toolchain\rust-1.99.1' }
if (!$JavaHome) { $JavaHome=Join-Path $taskRea 'tools\java\jdk-21.0.12.1+1' }
if (!$VendorDirectory) { $VendorDirectory=Join-Path $taskFramework 'engine\vendor' }
if (!$OutputDirectory) { $OutputDirectory=Join-Path $taskRoot 'dist\aola-launcher-20261107-release' }
if ($Action -eq 'build' -and (Test-Path -LiteralPath (Join-Path $OutputDirectory 'aola-launcher.exe'))) {
    throw "Retained launcher already exists: $OutputDirectory. Choose a new OutputDirectory."
}
$taskSaved=@{}
foreach($taskKey in @('PATH','RUSTC','CARGO_HOME','CARGO_TARGET_DIR','CARGO_BUILD_JOBS','RUST_MIN_STACK','TEMP','TMP')) { $taskSaved[$taskKey]=[Environment]::GetEnvironmentVariable($taskKey,'Process') }
try {
    $env:PATH=(Join-Path $ToolchainRoot 'bin')+';'+(Join-Path $JavaHome 'bin')+';'+$env:PATH
    $env:RUSTC=Join-Path $ToolchainRoot 'bin\rustc.exe'
    $env:CARGO_HOME=Join-Path $taskFramework 'build\engine-cargo-home'
    $env:CARGO_TARGET_DIR=Join-Path $taskFramework 'build\engine-target'
    $env:CARGO_BUILD_JOBS='1'
    $env:RUST_MIN_STACK='8388608'
    $env:TEMP=Join-Path $taskRoot 'build\temp'
    $env:TMP=$env:TEMP
    New-Item -ItemType Directory -Force -Path $env:TEMP | Out-Null
    $taskCommon=@('--manifest-path',(Join-Path $taskRoot 'engine\Cargo.toml'),'--config',(Join-Path $taskRoot 'engine\.cargo\vendor.toml'),'--config',("source.vendored-sources.directory='"+$VendorDirectory.Replace('\','/')+"'"),'--locked','--offline','--release','-p','ruffle_desktop','--no-default-features','--features','software_video,lzma')
    Push-Location $taskRoot
    try {
        if ($Action -eq 'test') {
            & (Join-Path $ToolchainRoot 'bin\cargo.exe') test @taskCommon --bin ruffle_desktop aola_ -- --test-threads=1
        } else {
            & (Join-Path $ToolchainRoot 'bin\cargo.exe') $Action @taskCommon
        }
        if ($LASTEXITCODE -ne 0) { throw "Launcher $Action failed ($LASTEXITCODE)" }
    } finally { Pop-Location }
    if ($Action -eq 'build') {
        New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
        Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'release\ruffle_desktop.exe') -Destination (Join-Path $OutputDirectory 'aola-launcher.exe')
        foreach($taskFile in @('run-launcher.cmd','LAUNCHER.md')) { Copy-Item -LiteralPath (Join-Path $taskRoot $taskFile) -Destination $OutputDirectory }
        Copy-Item -LiteralPath (Join-Path $taskRoot 'engine\LICENSE.md') -Destination (Join-Path $OutputDirectory 'RUFFLE-LICENSE.md')
        Write-Output $OutputDirectory
    }
} finally {
    foreach($taskKey in $taskSaved.Keys) { [Environment]::SetEnvironmentVariable($taskKey,$taskSaved[$taskKey],'Process') }
}
