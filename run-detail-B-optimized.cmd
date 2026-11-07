@echo off
setlocal
cd /d "%~dp0"
set "AOQI_STAGE3_MODE=sleep"
set "AOQI_POOL_IDLE_BUDGET_MB=256"
set "AOQI_POOL_RECENT_FRAMES=120"
set "AOLA_PET_PATCH=optimized"
start "" "%~dp0aola-native-test.exe" --aola-host --frame-rate 50 --aola-data-dir "%~dp0aola-data-optimized"
