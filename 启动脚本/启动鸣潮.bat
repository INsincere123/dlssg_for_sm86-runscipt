@echo off
start "" "%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "%~dp0scripts\common\LaunchFromBat.ps1" -ScriptPath "%~dp0scripts\games\Run-WutheringWaves-DLSSG.ps1" %*
exit /b
