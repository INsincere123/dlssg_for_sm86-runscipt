# Shared transcript support for launcher entry scripts (Windows PowerShell 5.1+).
function Start-LauncherLog {
    param([Parameter(Mandatory=$true)][string]$ScriptPath)
    $directory = Join-Path (Split-Path -Parent $ScriptPath) 'logs'
    New-Item -ItemType Directory -Path $directory -Force -ErrorAction Stop | Out-Null
    # Only remove ordinary log files in this script directory; locked logs are kept.
    $cutoff = (Get-Date).AddHours(-24)
    Get-ChildItem -LiteralPath $directory -Filter '*.log' -File | Where-Object {
        $_.LastWriteTime -lt $cutoff -and -not ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)
    } | ForEach-Object {
        Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue
    }
    $name = [IO.Path]::GetFileNameWithoutExtension($ScriptPath)
    $path = Join-Path $directory ('{0}_{1}_{2}.log' -f $name, (Get-Date -Format 'yyyyMMdd_HHmmss_fff'), $PID)
    Start-Transcript -LiteralPath $path -Force -ErrorAction Stop | Out-Null
    Write-Host "运行日志：$path"
    Write-Host ("脚本：{0}；进程：{1}；PowerShell：{2}" -f $ScriptPath, $PID, $PSVersionTable.PSVersion)
    return $path
}

function Stop-LauncherLog {
    param([string]$Path)
    if ($Path) {
        Write-Host "运行结束，日志已保存：$Path"
        Stop-Transcript -ErrorAction Continue | Out-Null
    }
}
