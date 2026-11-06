param(
    [ValidateSet('build','test')][string]$Action='build',
    [ValidateSet('gc-progress','gc-pool','frame-library','switch-fps','tick-profile','pet-detail','window-move','ab-verify','panel-diagnostic','date-compat')][string]$Variant='date-compat'
)
$ErrorActionPreference='Stop'
$taskRoot=Split-Path -Parent $PSScriptRoot
$taskFramework=Join-Path (Split-Path -Parent $taskRoot) 'aoqi.1.03\reconstructed'
if ($Action -eq 'build') {
    $taskDate=if ($Variant -in @('panel-diagnostic','date-compat')) { '20261107' } elseif ($Variant -in @('tick-profile','pet-detail','window-move','ab-verify')) { '20261106' } else { '20261105' }
    $taskOut=Join-Path $taskRoot "dist\aola-$Variant-test-$taskDate"
    if (Test-Path -LiteralPath (Join-Path $taskOut 'aola-native-test.exe')) { throw "Retained build already exists: $taskOut" }
    $taskMarker="aola-$Variant-test-$taskDate"
    if (!(Get-Content -LiteralPath (Join-Path $taskFramework 'engine\desktop\src\aola_diagnostics.rs') -Raw).Contains($taskMarker)) {
        throw "Source build marker does not match $taskMarker; output naming does not select an optimization."
    }
    & (Join-Path $taskFramework 'tools\build_engine.ps1') -Action build
    New-Item -ItemType Directory -Force -Path $taskOut | Out-Null
    Copy-Item -LiteralPath (Join-Path $taskFramework 'build\engine-target\release\ruffle_desktop.exe') -Destination (Join-Path $taskOut 'aola-native-test.exe')
    Copy-Item -LiteralPath (Join-Path $taskRoot 'README.md') -Destination $taskOut
    Copy-Item -LiteralPath (Join-Path $taskRoot 'BATTLE-MODEL-PERFORMANCE.md') -Destination $taskOut
    Copy-Item -LiteralPath (Join-Path $taskRoot 'GC-POOL-TEST.md') -Destination $taskOut
    if ($Variant -eq 'frame-library') { Copy-Item -LiteralPath (Join-Path $taskRoot 'FRAME-LIBRARY-TEST.md') -Destination $taskOut }
    if ($Variant -in @('switch-fps','tick-profile','pet-detail','window-move','ab-verify')) {
        Copy-Item -LiteralPath (Join-Path $taskRoot 'SWITCH-FPS-TEST.md') -Destination $taskOut
        foreach ($taskLauncher in @('run-aola-50fps.cmd','run-aola-60fps.cmd','run-aola-baseline.cmd')) {
            Copy-Item -LiteralPath (Join-Path $taskRoot $taskLauncher) -Destination $taskOut
        }
    }
    if ($Variant -eq 'tick-profile') { Copy-Item -LiteralPath (Join-Path $taskRoot 'TICK-PROFILE-TEST.md') -Destination $taskOut }
    if ($Variant -in @('pet-detail','window-move','ab-verify')) {
        $taskEvidence=Join-Path $taskRoot 'evidence\pet-detail-20261106'
        New-Item -ItemType Directory -Force -Path (Join-Path $taskOut 'swf') | Out-Null
        Copy-Item -LiteralPath (Join-Path $taskEvidence 'source\pet_newpetpackage~2026102936286521.swf') -Destination (Join-Path $taskOut 'swf\newpetpackage-original.swf')
        Copy-Item -LiteralPath (Join-Path $taskEvidence 'variants\newpetpackage-detail-v4.swf') -Destination (Join-Path $taskOut 'swf\newpetpackage-detail.swf')
        foreach ($taskLauncher in @('run-detail-A-original.cmd','run-detail-B-optimized.cmd')) {
            Copy-Item -LiteralPath (Join-Path $taskRoot $taskLauncher) -Destination $taskOut
        }
        Copy-Item -LiteralPath (Join-Path $taskRoot 'PET-DETAIL-TEST.txt') -Destination $taskOut
    }
    if ($Variant -in @('window-move','ab-verify')) { Copy-Item -LiteralPath (Join-Path $taskRoot 'WINDOW-MOVE-TEST.md') -Destination $taskOut }
    if ($Variant -eq 'ab-verify') { Copy-Item -LiteralPath (Join-Path $taskRoot 'START-A-B-TEST.txt') -Destination $taskOut }
    if ($Variant -eq 'panel-diagnostic') {
        foreach ($taskFile in @('run-panel-diagnostic.cmd','PANEL-DIAGNOSTIC-TEST.txt')) {
            Copy-Item -LiteralPath (Join-Path $taskRoot $taskFile) -Destination $taskOut
        }
    }
    if ($Variant -eq 'date-compat') {
        foreach ($taskFile in @('run-date-compat.cmd','DATE-COMPAT-TEST.txt')) {
            Copy-Item -LiteralPath (Join-Path $taskRoot $taskFile) -Destination $taskOut
        }
    }
    Copy-Item -LiteralPath (Join-Path $taskRoot 'run-aola-test.cmd') -Destination $taskOut
    Copy-Item -LiteralPath (Join-Path $taskFramework 'dist\command-batch-test-20261105\licenses') -Destination $taskOut -Recurse -Force
    Write-Output $taskOut
} else {
    $taskToolchain=Join-Path (Split-Path -Parent $taskFramework) 'toolchain\rust-1.99.1'
    $taskEnvironment=@{}
    foreach($taskName in @('PATH','RUSTC','CARGO_HOME','CARGO_TARGET_DIR','CARGO_BUILD_JOBS','RUST_MIN_STACK')) {
        $taskEnvironment[$taskName]=[Environment]::GetEnvironmentVariable($taskName,'Process')
    }
    try {
        $env:PATH=(Join-Path $taskToolchain 'bin')+';D:\game\Rea\tools\java\jdk-21.0.12.1+1\bin;'+$env:PATH
        $env:RUSTC=Join-Path $taskToolchain 'bin\rustc.exe'
        $env:CARGO_HOME=Join-Path $taskFramework 'build\engine-cargo-home'
        $env:CARGO_TARGET_DIR=Join-Path $taskFramework 'build\engine-target'
        $env:CARGO_BUILD_JOBS='1'
        $env:RUST_MIN_STACK='8388608'
        Push-Location $taskFramework
        try {
            & (Join-Path $taskToolchain 'bin\cargo.exe') test --manifest-path engine\Cargo.toml --config engine\.cargo\vendor.toml --release --locked --offline -p ruffle_desktop --no-default-features --features software_video,lzma --bin ruffle_desktop aola_
            if ($LASTEXITCODE -ne 0) { throw 'Aola bridge tests failed.' }
        } finally { Pop-Location }
    } finally {
        foreach($taskName in $taskEnvironment.Keys) { [Environment]::SetEnvironmentVariable($taskName,$taskEnvironment[$taskName],'Process') }
    }
}
