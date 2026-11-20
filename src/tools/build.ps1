param(
    [ValidateSet('optimization','launcher')][string]$Component='optimization',
    [ValidateSet('metadata','check','test','build')][string]$Action='metadata',
    [string]$OutputDirectory=''
)
$ErrorActionPreference='Stop'
$taskRoot=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$taskComponent=Join-Path $taskRoot "src\$Component"
$taskEngine=Join-Path $taskComponent 'engine'
$taskRust=Join-Path $taskRoot 'toolchain\rust-1.99.1\bin'
$taskJava=Join-Path $taskRoot 'toolchain\jdk-21.0.12.1+1\bin'
$taskVendor=(Join-Path $taskRoot 'deps\vendor').Replace('\','/')
$taskSaved=@{}
foreach($taskKey in @('PATH','RUSTC','CARGO_HOME','CARGO_TARGET_DIR','CARGO_BUILD_JOBS','RUST_MIN_STACK','CARGO_PROFILE_DEV_DEBUG','CARGO_PROFILE_TEST_DEBUG','AOQI_STAGE3_MODE','AOLA_SUBTREE_SCRIPT_SLEEP')) {
    $taskSaved[$taskKey]=[Environment]::GetEnvironmentVariable($taskKey,'Process')
}
if (!$OutputDirectory) { $OutputDirectory=Join-Path $taskRoot ('dist\rebuilt-'+$Component+'-'+(Get-Date -Format 'yyyyMMdd-HHmmss')) }
if ($Action -eq 'build' -and (Test-Path -LiteralPath $OutputDirectory)) { throw "Choose a new output directory: $OutputDirectory" }
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
                if($Component -eq 'optimization') {
                    foreach($taskMode in @('0','1')) {
                        $env:AOLA_SUBTREE_SCRIPT_SLEEP=$taskMode
                        foreach($taskFilter in @('aola_','stage3_stopped_tree')) {
                            & "$taskRust\cargo.exe" test @taskCommon --release -p ruffle_core --lib $taskFilter -- --nocapture
                            if($LASTEXITCODE -ne 0){throw "Core test failed: $taskFilter mode=$taskMode"}
                        }
                    }
                }
                & "$taskRust\cargo.exe" test @taskCommon @taskDesktop --bin ruffle_desktop aola_ -- --test-threads=1
                if($LASTEXITCODE -ne 0){throw 'Desktop regression failed'}
            }
            default {
                & "$taskRust\cargo.exe" $Action @taskCommon @taskDesktop
                if($LASTEXITCODE -ne 0){throw "$Component $Action failed"}
            }
        }
    } finally { Pop-Location }
    if($Action -eq 'build') {
        New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
        $taskTemplate=if($Component -eq 'launcher'){'aola-launcher-20261107-release'}else{'aola-subtree-scripts-test-20261109'}
        $taskExe=if($Component -eq 'launcher'){'aola-launcher.exe'}else{'aola-native-test.exe'}
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
