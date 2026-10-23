@echo off
chcp 65001 >nul
start "" "%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -STA -WindowStyle Hidden -ExecutionPolicy Bypass -File "%~dp0scripts\ui\Show-SteamLauncher.ps1"
exit /b
