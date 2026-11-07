@echo off
setlocal
cd /d "%~dp0"
set "AOQI_STAGE3_MODE=baseline"
set "AOQI_POOL_IDLE_BUDGET_MB=256"
set "AOQI_POOL_RECENT_FRAMES=120"
start "" "%~dp0aola-native-test.exe" --aola-host
