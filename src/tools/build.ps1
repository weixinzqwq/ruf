param(
    [ValidateSet('optimization')][string]$Component='optimization',
    [ValidateSet('metadata','check','test','build','swf-build','swf-test','swf-loopback','pet-switch-build','flux-build')][string]$Action='metadata',
    [string]$OutputDirectory='',
    [string]$RoundtripDirectory='',
    [switch]$ClosePhases,
    [switch]$OrphanGoto,
    [switch]$BattleRender,
    [switch]$BattleTexturePool,
    [switch]$PetSwitchTiming,
    [switch]$BattleGlowReuse,
    [switch]$MyProTools
)
$ErrorActionPreference='Stop'
$taskRoot=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$taskComponent=Join-Path $taskRoot "src\$Component"
$taskEngine=Join-Path $taskComponent 'engine'
$taskRust=Join-Path $taskRoot 'toolchain\rust-1.99.1\bin'
$taskJava=Join-Path $taskRoot 'toolchain\jdk-21.0.12.1+1\bin'
$taskVendor=(Join-Path $taskRoot 'deps\vendor').Replace('\','/')
$taskSwfAction=$Action -in @('swf-build','swf-test','swf-loopback')
if ($ClosePhases -and $Action -ne 'flux-build') { throw '-ClosePhases requires -Action flux-build.' }
if ($OrphanGoto -and $Action -ne 'flux-build') { throw '-OrphanGoto requires -Action flux-build.' }
if ($BattleRender -and $Action -ne 'flux-build') { throw '-BattleRender requires -Action flux-build.' }
if ($BattleTexturePool -and $Action -ne 'flux-build') { throw '-BattleTexturePool requires -Action flux-build.' }
if ($PetSwitchTiming -and $Action -ne 'flux-build') { throw '-PetSwitchTiming requires -Action flux-build.' }
if ($BattleGlowReuse -and $Action -ne 'flux-build') { throw '-BattleGlowReuse requires -Action flux-build.' }
if ($MyProTools -and $Action -ne 'flux-build') { throw '-MyProTools requires -Action flux-build.' }
if ($RoundtripDirectory -and !$taskSwfAction) { throw '-RoundtripDirectory requires an SWF action.' }
if ($Action -in @('swf-test','swf-loopback') -and $OutputDirectory) { throw '-OutputDirectory selects a new distribution for swf-build.' }
if ($Action -in @('swf-build','pet-switch-build','flux-build')) {
    $taskPrefix=switch ($Action) {
        'pet-switch-build' { 'aola-pet-switch-' }
        'flux-build' { if ($MyProTools) { 'aola-mypro-tools-' } elseif ($BattleGlowReuse) { 'aola-battle-glow-reuse-' } elseif ($PetSwitchTiming) { 'aola-pet-switch-timing-' } elseif ($BattleTexturePool) { 'aola-battle-texture-pool-' } elseif ($BattleRender) { 'aola-battle-render-' } elseif ($OrphanGoto) { 'aola-orphan-goto-' } elseif ($ClosePhases) { 'aola-close-phases-' } else { 'aola-flux-' } }
        default { 'aola-swf-lifecycle-' }
    }
    if (!$OutputDirectory) { $OutputDirectory=Join-Path $taskRoot ("dist\$taskPrefix"+(Get-Date -Format 'yyyyMMdd-HHmmss')) }
    $OutputDirectory=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
    $taskPython=(Get-Command python -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
}
$taskSaved=@{}
foreach($taskKey in @('PATH','RUSTC','CARGO_HOME','CARGO_TARGET_DIR','CARGO_BUILD_JOBS','RUST_MIN_STACK','CARGO_PROFILE_DEV_DEBUG','CARGO_PROFILE_TEST_DEBUG','AOQI_STAGE3_MODE','AOLA_SUBTREE_SCRIPT_SLEEP','AOLA_TEST_SWF_FILES')) {
    $taskSaved[$taskKey]=[Environment]::GetEnvironmentVariable($taskKey,'Process')
}
if (!$OutputDirectory) { $OutputDirectory=Join-Path $taskRoot ('dist\rebuilt-'+$Component+'-'+(Get-Date -Format 'yyyyMMdd-HHmmss')) }
if ($Action -in @('build','swf-build','pet-switch-build','flux-build') -and (Test-Path -LiteralPath $OutputDirectory)) { throw "Choose a new output directory: $OutputDirectory" }
if ($taskSwfAction) {
    $taskSwfTools=Join-Path $PSScriptRoot 'swf-patches'
    $taskSwfWork=Join-Path $taskRoot ('build\swf-patches\'+$Action+'-'+[Guid]::NewGuid().ToString('N'))
    if ($RoundtripDirectory) {
        $RoundtripDirectory=(Resolve-Path -LiteralPath $RoundtripDirectory).ProviderPath
        foreach ($name in @('reference.swf','run-1\control.swf','run-2\control.swf')) {
            if (!(Test-Path -LiteralPath (Join-Path $RoundtripDirectory $name) -PathType Leaf)) { throw "Missing roundtrip material: $RoundtripDirectory\$name" }
        }
    }
    $taskPython=(Get-Command python -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
}

function Get-SwfRoundtrip {
    if ($RoundtripDirectory) { return $RoundtripDirectory }
    $directory=Join-Path $taskSwfWork 'roundtrip'
    & $taskPython (Join-Path $taskSwfTools 'roundtrip.py') --output-dir $directory | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'SWF control rebuild failed.' }
    return $directory
}
try {
    $env:PATH="$taskRust;$taskJava;"+$env:PATH
    $env:RUSTC=Join-Path $taskRust 'rustc.exe'
    $env:CARGO_HOME=Join-Path $taskRoot 'build\cargo-home'
    $env:CARGO_TARGET_DIR=Join-Path $taskRoot "build\$Component-target"
    $env:CARGO_BUILD_JOBS='1'; $env:RUST_MIN_STACK='8388608'
    $env:CARGO_PROFILE_DEV_DEBUG='0'; $env:CARGO_PROFILE_TEST_DEBUG='0'
    $env:AOQI_STAGE3_MODE='sleep'
    $taskCommon=@('--manifest-path', (Join-Path $taskEngine 'Cargo.toml'), '--config', (Join-Path $taskEngine '.cargo\vendor.toml'), '--config', "source.vendored-sources.directory='$taskVendor'", '--locked', '--offline')
    $taskDesktop=@('--release','-p','ruffle_desktop','--no-default-features','--features','software_video,lzma')
    Push-Location $taskComponent
    try {
        switch($Action) {
            'metadata' {
                $taskEvidence=Join-Path $taskRoot 'doc\evidence\migration-20261121'
                New-Item -ItemType Directory -Force -Path $taskEvidence | Out-Null
                & "$taskRust\cargo.exe" metadata @taskCommon --format-version 1 --filter-platform x86_64-pc-windows-msvc > (Join-Path $taskEvidence "$Component-metadata.json")
                if($LASTEXITCODE -ne 0){throw 'Offline dependency resolution failed'}
            }
            'test' {
                foreach($taskMode in @('0','1')) {
                    $env:AOLA_SUBTREE_SCRIPT_SLEEP=$taskMode
                    foreach($taskFilter in @('aola_','stage3_stopped_tree')) {
                        & "$taskRust\cargo.exe" test @taskCommon --release -p ruffle_core --lib $taskFilter -- --nocapture
                        if($LASTEXITCODE -ne 0){throw "Core test failed: $taskFilter mode=$taskMode"}
                    }
                }
                & "$taskRust\cargo.exe" test @taskCommon @taskDesktop --bin ruffle_desktop -- --test-threads=1
                if($LASTEXITCODE -ne 0){throw 'Desktop regression failed'}
                & "$taskRust\cargo.exe" test @taskCommon --release -p ruffle_frontend_utils --features navigator --lib test_socket_tap_full_queue_preserves_io_and_offsets -- --test-threads=1
                if($LASTEXITCODE -ne 0){throw 'Socket tap loopback regression failed'}
            }
            { $_ -in @('swf-build','pet-switch-build','flux-build') } {
                & "$taskRust\cargo.exe" build @taskCommon @taskDesktop
                if ($LASTEXITCODE -ne 0) { throw 'Optimization desktop build failed.' }
                $taskBinary=Join-Path $env:CARGO_TARGET_DIR 'release\ruffle_desktop.exe'
                if ($Action -eq 'flux-build') {
                    $taskPackageArgs = @('--executable', $taskBinary, '--output', $OutputDirectory)
                    if ($ClosePhases) { $taskPackageArgs += '--close-phases' }
                    if ($OrphanGoto) { $taskPackageArgs += '--orphan-goto' }
                    if ($BattleRender) { $taskPackageArgs += '--battle-render' }
                    if ($BattleTexturePool) { $taskPackageArgs += '--battle-texture-pool' }
                    if ($PetSwitchTiming) { $taskPackageArgs += '--pet-switch-timing' }
                    if ($BattleGlowReuse) { $taskPackageArgs += '--battle-glow-reuse' }
                    if ($MyProTools) { $taskPackageArgs += '--mypro-tools' }
                    & $taskPython (Join-Path $PSScriptRoot 'swf-patches\flux_package.py') @taskPackageArgs
                } elseif ($Action -eq 'pet-switch-build') {
                    & $taskPython (Join-Path $PSScriptRoot 'swf-patches\pet_switch_package.py') --executable $taskBinary --output $OutputDirectory
                } else {
                    $taskControl=Get-SwfRoundtrip
                    & $taskPython (Join-Path $taskSwfTools 'capture_package.py') --roundtrip $taskControl --executable $taskBinary --output $OutputDirectory
                }
                if ($LASTEXITCODE -ne 0) { throw 'Experiment packaging failed.' }
            }
            { $_ -in @('swf-test','swf-loopback') } {
                $taskControl=Get-SwfRoundtrip
                $env:AOLA_TEST_SWF_FILES=Join-Path $taskSwfWork 'fixtures'
                & $taskPython (Join-Path $taskSwfTools 'capture_package.py') --roundtrip $taskControl --fixtures-only --output $env:AOLA_TEST_SWF_FILES | Out-Host
                if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the real SWF regression inputs.' }
                $taskNavigator=@('--release','-p','ruffle_frontend_utils','--features','navigator','--lib')
                if ($Action -eq 'swf-loopback') {
                    & "$taskRust\cargo.exe" test @taskCommon @taskNavigator manual_swf_local_http_delivers_real_roundtrip_through_fetch -- --ignored --nocapture | Out-Host
                    if ($LASTEXITCODE -ne 0) { throw 'HTTP loopback regression failed. Windows socket error 10013 indicates a denied connection to localhost.' }
                    break
                }
                & "$taskRust\cargo.exe" test @taskCommon @taskNavigator aola_ -- --include-ignored --test-threads=1 | Out-Host
                if ($LASTEXITCODE -ne 0) { throw 'SWF navigator regression failed.' }

                # Cargo identifies the executable for this invocation among any older artifacts in deps.
                $taskRunner=@(& "$taskRust\cargo.exe" test @taskCommon --release -p tests --test tests --no-run --message-format=json-render-diagnostics | ForEach-Object {
                    $message=ConvertFrom-Json -InputObject $_
                    if ($message.reason -eq 'compiler-artifact' -and $message.target.name -eq 'tests' -and $message.target.kind -contains 'test' -and $message.executable) {
                        $message.executable
                    }
                })
                if ($LASTEXITCODE -ne 0) { throw 'Headless runner build failed.' }
                if ($taskRunner.Count -ne 1 -or !(Test-Path -LiteralPath $taskRunner[0] -PathType Leaf)) { throw 'Cargo did not report one existing headless test runner.' }
                Write-Output $taskRunner[0]
            }
            default {
                & "$taskRust\cargo.exe" $Action @taskCommon @taskDesktop
                if($LASTEXITCODE -ne 0){throw "$Component $Action failed"}
            }
        }
    } finally { Pop-Location }
    if($Action -eq 'build') {
        New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
        $taskTemplate='aola-subtree-scripts-test-20261109'
        $taskExe='aola-native-test.exe'
        Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'release\ruffle_desktop.exe') -Destination (Join-Path $OutputDirectory $taskExe)
        Get-ChildItem -LiteralPath (Join-Path $taskRoot "dist\$taskTemplate") -Filter '*.cmd' | Copy-Item -Destination $OutputDirectory
        Copy-Item -LiteralPath (Join-Path $taskEngine 'LICENSE.md') -Destination (Join-Path $OutputDirectory 'RUFFLE-LICENSE.md')
        $taskLicenses=Join-Path $taskRoot "dist\$taskTemplate\licenses"
        if(Test-Path -LiteralPath $taskLicenses){Copy-Item -LiteralPath $taskLicenses -Destination $OutputDirectory -Recurse}
        Write-Output $OutputDirectory
    }
} finally {
    foreach($taskKey in $taskSaved.Keys){[Environment]::SetEnvironmentVariable($taskKey,$taskSaved[$taskKey],'Process')}
}
