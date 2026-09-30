[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$KeepLinks,
    [switch]$VerboseLog
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$gameDir = 'C:\Games\Wuthering Waves\Wuthering Waves Game'
$launcherDir = 'C:\Program Files\Wuthering Waves'
$launcherExe = Join-Path $launcherDir 'launcher.exe'
$renderDir = Join-Path $gameDir 'Client\Binaries\Win64'
$gameExe = Join-Path $renderDir 'Client-Win64-Shipping.exe'
$projectDir = 'D:\Games\dlssg_for_sm86'
$projectDll = Join-Path $projectDir 'version.dll'
$storageDll = Join-Path $renderDir '_storage_\version.dll'
$logDir = Join-Path $renderDir 'dlssg_sm86'
$launcherProcessName = [System.IO.Path]::GetFileNameWithoutExtension($launcherExe)
$processName = [System.IO.Path]::GetFileNameWithoutExtension($gameExe)
$diagnosticIniPath = Join-Path $env:TEMP ("dlssg_sm86-wuthering-{0}.ini" -f $PID)

$diagnosticIni = @'
; 鸣潮 0.3.2 临时诊断配置。
; 由 Run-WutheringWaves-DLSSG.ps1 生成。

[General]
Enabled=1

[FrameGeneration]
Optimized=1
MaxGeneratedFrames=3

[Compatibility]
Preset=Auto

[Logging]
Level=2
Directory=dlssg_sm86\logs

[Runtime]
Mode=Bundled
CacheDirectory=
'@

$linkSpecs = @(
    @{ Path = (Join-Path $renderDir 'version.dll'); Target = $projectDll },
    @{ Path = (Join-Path $renderDir 'dlssg_sm86.ini'); Target = $diagnosticIniPath }
)

$storageExistedBefore = $false
$gameExitCode = $null
$failed = $false

function Write-Log {
    param([string]$Message)
    Write-Host ("[{0}] {1}" -f (Get-Date -Format 'HH:mm:ss'), $Message)
}

function Write-Detail {
    param([string]$Message)
    if ($VerboseLog) { Write-Log $Message }
}

function Get-HashOrNull {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToUpperInvariant()
}

function Get-NormalizedPath {
    param([string]$Path)
    return [System.IO.Path]::GetFullPath($Path).TrimEnd('\').ToLowerInvariant()
}

function Assert-FileExists {
    param([string]$Path, [string]$Description)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "未找到$Description：$Path"
    }
}

function Assert-NoGameProcess {
    $launcherRunning = @(Get-Process -Name $launcherProcessName -ErrorAction SilentlyContinue)
    if ($launcherRunning.Count -gt 0) {
        throw "$launcherProcessName 已经在运行，请先关闭启动器再使用此脚本。"
    }

    $running = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        throw "$processName 已经在运行，请先关闭游戏再使用此脚本。"
    }
}

function Wait-ForGameProcess {
    param([int]$TimeoutSeconds = 120)

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $processes = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
        if ($processes.Count -gt 0) {
            Write-Log "已检测到游戏进程：$processName"
            return $processes[0]
        }

        Start-Sleep -Seconds 2
    } while ((Get-Date) -lt $deadline)

    throw "等待游戏进程超时（$TimeoutSeconds 秒）：$gameExe"
}

function Test-StaleDiagnosticIniTarget {
    param([string]$Path, [string]$Target)

    if ((Get-NormalizedPath $Path) -ne (Get-NormalizedPath $linkSpecs[1].Path)) { return $false }
    if ((Split-Path -Parent (Get-NormalizedPath $Target)) -ne (Get-NormalizedPath $env:TEMP)) { return $false }
    if ([System.IO.Path]::GetFileName($Target) -notmatch '^dlssg_sm86-wuthering-\d+\.ini$') { return $false }
    if (-not (Test-Path -LiteralPath $Target -PathType Leaf)) { return $false }

    $contents = [System.IO.File]::ReadAllText($Target)
    return $contents -ceq $diagnosticIni
}

function Assert-LinkOrAbsent {
    param([string]$Path, [string]$ExpectedTarget)

    $item = Get-Item -LiteralPath $Path -ErrorAction SilentlyContinue
    if ($null -eq $item) { return $false }

    if ($item.LinkType -ne 'SymbolicLink') {
        throw "拒绝覆盖现有的普通文件：$Path"
    }

    $actualTarget = [string](($item.Target -join ''))
    if ((Get-NormalizedPath $actualTarget) -ne (Get-NormalizedPath $ExpectedTarget)) {
        if (Test-StaleDiagnosticIniTarget -Path $Path -Target $actualTarget) {
            if ($DryRun) {
                Write-Log "演练模式：将清理脚本生成的过期临时配置软链接：$Path -> $actualTarget"
                return $false
            }

            Remove-Item -LiteralPath $Path
            Remove-Item -LiteralPath $actualTarget
            Write-Log "已清理脚本生成的过期临时配置软链接及文件：$Path"
            return $false
        }

        throw "拒绝替换指向异常目标的软链接：$Path -> $actualTarget"
    }

    Write-Detail "Existing expected symbolic link: $Path -> $actualTarget"
    return $true
}

function Ensure-Link {
    param([string]$Path, [string]$Target)

    if (Assert-LinkOrAbsent -Path $Path -ExpectedTarget $Target) { return }

    if ($DryRun) {
        Write-Log "演练模式：将创建软链接 $Path -> $Target"
        return
    }

    New-Item -ItemType SymbolicLink -Path $Path -Target $Target | Out-Null
    Write-Log "已创建软链接：$Path -> $Target"
}

function Remove-ExpectedLink {
    param([string]$Path, [string]$ExpectedTarget)

    $item = Get-Item -LiteralPath $Path -ErrorAction SilentlyContinue
    if ($null -eq $item) { return }

    if ($item.LinkType -eq 'SymbolicLink') {
        $actualTarget = [string](($item.Target -join ''))
        if ((Get-NormalizedPath $actualTarget) -eq (Get-NormalizedPath $ExpectedTarget)) {
            Remove-Item -LiteralPath $Path
            Write-Log "已删除临时软链接：$Path"
            return
        }
    }

    Write-Warning "未删除 $Path，因为它已不再是预期的软链接。"
}

function Remove-GeneratedStorageDll {
    if ($storageExistedBefore) {
        Write-Detail "保留启动前已存在的 storage DLL：$storageDll"
        return
    }

    if (-not (Test-Path -LiteralPath $storageDll -PathType Leaf)) {
        Write-Log '未发现本次生成的 _storage_\\version.dll。'
        return
    }

    $sourceHash = Get-HashOrNull $projectDll
    $storageHash = Get-HashOrNull $storageDll
    if ($storageHash -eq $sourceHash) {
        Remove-Item -LiteralPath $storageDll
        Write-Log '已删除本次生成的 _storage_\\version.dll。'
    }
    else {
        Write-Warning '保留 _storage_\\version.dll，因为 SHA256 与项目 DLL 不匹配。'
    }
}

function Show-PostRunStatus {
    $logs = @()
    if (Test-Path -LiteralPath $logDir) {
        $logs = @(Get-ChildItem -LiteralPath $logDir -Recurse -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending)
    }

    if ($logs.Count -gt 0) {
        Write-Log ("找到 {0} 个 DLSSG 日志文件" -f $logs.Count)
        Write-Log ("最新日志：{0}" -f $logs[0].FullName)
    }
    else {
        Write-Warning "目录下未找到 DLSSG 日志：$logDir"
    }
}

try {
    Assert-FileExists -Path $launcherExe -Description 'Wuthering Waves launcher'
    Assert-FileExists -Path $gameExe -Description 'Wuthering Waves rendering executable'
    Assert-FileExists -Path $projectDll -Description 'Project version.dll'
    Assert-NoGameProcess

    $storageExistedBefore = Test-Path -LiteralPath $storageDll -PathType Leaf
    if ($storageExistedBefore) {
        Write-Log "发现启动前已存在的 _storage_\\version.dll，将保留。SHA256：$(Get-HashOrNull $storageDll)"
    }

    if ($DryRun) {
        Write-Log '演练模式：环境检查通过。'
        Ensure-Link -Path $linkSpecs[0].Path -Target $linkSpecs[0].Target
        Ensure-Link -Path $linkSpecs[1].Path -Target $linkSpecs[1].Target
        exit 0
    }

    [System.IO.File]::WriteAllText($diagnosticIniPath, $diagnosticIni, [System.Text.UTF8Encoding]::new($false))
    Ensure-Link -Path $linkSpecs[0].Path -Target $linkSpecs[0].Target
    Ensure-Link -Path $linkSpecs[1].Path -Target $linkSpecs[1].Target

    Write-Log '正在启动鸣潮官方启动器。本脚本不会绕过或关闭 ACE。'
    $launcher = Start-Process -FilePath $launcherExe -WorkingDirectory $launcherDir -PassThru
    $process = Wait-ForGameProcess
    $process.WaitForExit()
    $gameExitCode = $process.ExitCode
    Write-Log "游戏已退出，退出代码：$gameExitCode"
}
catch {
    $failed = $true
    Write-Error $_
}
finally {
    if (-not $DryRun -and -not $KeepLinks) {
        try { Remove-ExpectedLink -Path $linkSpecs[1].Path -ExpectedTarget $linkSpecs[1].Target } catch { Write-Warning $_ }
        try { Remove-ExpectedLink -Path $linkSpecs[0].Path -ExpectedTarget $linkSpecs[0].Target } catch { Write-Warning $_ }
        try { Remove-GeneratedStorageDll } catch { Write-Warning $_ }
    }
    elseif ($KeepLinks -and -not $DryRun) {
        Write-Log '已指定 KeepLinks，临时软链接和 storage 文件不会删除。'
    }

    if (Test-Path -LiteralPath $diagnosticIniPath -PathType Leaf) {
        Remove-Item -LiteralPath $diagnosticIniPath -ErrorAction SilentlyContinue
    }

    if (-not $DryRun) {
        Show-PostRunStatus
    }
}

if ($failed) { exit 1 }
if ($null -ne $gameExitCode) { exit $gameExitCode }
