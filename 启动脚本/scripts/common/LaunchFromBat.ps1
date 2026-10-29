# BAT entry point: launch independently, with no launcher transcript.
param([string]$ScriptPath)
$ErrorActionPreference = 'Stop'
try {
    $script = Get-Item -LiteralPath $ScriptPath -ErrorAction Stop
    $argumentText = @($args | ForEach-Object {
        if ($_ -match '^-[A-Za-z][A-Za-z0-9]*$') { [string]$_ }
        else { "'" + ([string]$_).Replace("'", "''") + "'" }
    }) -join ' '
    $command = @"
`$ErrorActionPreference = 'Stop'
`$exitCode = 1
`$failure = ''
`$logDirectory = '__LOG_DIRECTORY__'
try {
    Set-Location -LiteralPath '__DIRECTORY__'
    `$global:LASTEXITCODE = 0
    & '__SCRIPT__' __ARGUMENTS__
    `$succeeded = `$?
    `$exitCode = `$global:LASTEXITCODE
    if (-not `$succeeded -and `$exitCode -eq 0) { `$exitCode = 1 }
}
catch {
    `$failure = `$_.Exception.Message
    Write-Host (`$_ | Format-List * -Force | Out-String) -ForegroundColor Red
    `$exitCode = 1
}
if (`$exitCode -ne 0) {
    Add-Type -AssemblyName System.Windows.Forms
    `$message = "脚本运行失败，退出代码：`$exitCode``r``n`$failure``r``n``r``n日志目录：`$logDirectory"
    [void][System.Windows.Forms.MessageBox]::Show(`$message, '启动脚本失败', 'OK', 'Error')
}
exit `$exitCode
"@
    $command = $command.Replace('__DIRECTORY__', $script.DirectoryName.Replace("'", "''")).Replace('__SCRIPT__', $script.FullName.Replace("'", "''")).Replace('__ARGUMENTS__', $argumentText)
    $logDirectory = Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'logs'
    $command = $command.Replace('__LOG_DIRECTORY__', $logDirectory.Replace("'", "''"))
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $powershellExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\conhost.exe') -Verb RunAs -ArgumentList ('"{0}" -NoLogo -NoProfile -ExecutionPolicy Bypass -EncodedCommand {1}' -f $powershellExe, $encoded)
}
catch {
    Add-Type -AssemblyName System.Windows.Forms
    [void][System.Windows.Forms.MessageBox]::Show($_.Exception.Message, '启动脚本失败', 'OK', 'Error')
    exit 1
}
