@echo off
setlocal
set "AOQI_STAGE3_MODE=sleep"
set "AOLA_PET_PATCH="
set "AOLA_PANEL_DIAGNOSTICS="
"%~dp0aola-launcher.exe" --aola-host --aola-tools --aola-data-dir "%~dp0aola-launcher-data"
