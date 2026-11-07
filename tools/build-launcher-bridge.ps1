param([string]$FFDecPath='D:\Programs\FFdec\ffdec-cli.exe')
$ErrorActionPreference='Stop'
$taskRoot=Split-Path -Parent $PSScriptRoot
$taskBridge=Join-Path $taskRoot 'source\launcher-bridge'
$taskInput=Join-Path $taskBridge 'source\start-original.fws.swf'
$taskSource=Join-Path $taskBridge 'Load.as'
$taskOutput=Join-Path $taskBridge 'variants\start-launcher.fws.swf'
$taskRun=Join-Path $taskBridge ('evidence\build-'+[DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss-fff'))
New-Item -ItemType Directory -Path $taskRun | Out-Null
$taskCandidate=Join-Path $taskRun 'candidate.swf'
& $FFDecPath -onerror abort -replace $taskInput $taskCandidate mmo.checkversion.Load $taskSource *> (Join-Path $taskRun 'replace.log')
if($LASTEXITCODE -ne 0){throw 'Launcher bridge compilation failed'}
& $FFDecPath -onerror abort -export script,symbolClass (Join-Path $taskRun 'export') $taskCandidate *> (Join-Path $taskRun 'export.log')
if($LASTEXITCODE -ne 0){throw 'Launcher bridge reread failed'}
$taskExport=Get-Content -LiteralPath (Join-Path $taskRun 'export\scripts\mmo\checkversion\Load.as') -Raw
foreach($taskToken in @('aolaSendCommand','aolaBridgeReady','aolaCommandCompleted','ActClientSender','ApplicationDomain.currentDomain','loadBytes','loadVersion')){
    if(!$taskExport.Contains($taskToken)){throw "Missing bridge/startup behavior: $taskToken"}
}
$taskSymbols=Get-Content -LiteralPath (Join-Path $taskRun 'export\symbolClass\symbols.csv') -Raw
if(!$taskSymbols.Contains('mmo.checkversion.Load')){throw 'Document class missing'}
& python (Join-Path $taskBridge 'verify-tags.py') $taskCandidate
if($LASTEXITCODE -ne 0){throw 'Bridge changed non-code SWF tags'}
Copy-Item -LiteralPath $taskCandidate -Destination $taskOutput -Force
& python (Join-Path $taskBridge 'fixtures\embed-mock.py')
if($LASTEXITCODE -ne 0){throw 'Offline bridge fixture generation failed'}
Write-Output $taskOutput
