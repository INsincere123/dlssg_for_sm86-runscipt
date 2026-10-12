@echo off
start "" "%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "%~dp0LaunchFromBat.ps1" -ScriptPath "%~dpn0.ps1" %*
exit /b
