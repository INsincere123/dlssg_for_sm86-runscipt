@echo off
setlocal
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\tools\Export-PortableLauncher.ps1" -Zip -Interactive %*
set "packageExitCode=%ERRORLEVEL%"
if not "%packageExitCode%"=="0" echo Packaging failed. Exit code: %packageExitCode%
pause
exit /b %packageExitCode%
