[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$KeepLinks,
    [switch]$VerboseLog
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$gameRoot = 'D:\Neverness To Everness'
$renderDir = Join-Path $gameRoot 'Client\WindowsNoEditor\HT\Binaries\Win64'
$launcherExe = Join-Path $gameRoot 'NTELauncher.exe'
$gameExe = Join-Path $renderDir 'HTGame.exe'
$projectDir = 'D:\Games\dlssg_for_sm86'
$projectProxy = Join-Path $projectDir 'alternatives\winmm.dll'
$projectIni = Join-Path $projectDir 'dlssg_sm86.ini'
$storageDll = Join-Path $renderDir '_storage_\winmm.dll'
$logDir = Join-Path $renderDir 'dlssg_sm86'
$launcherProcessName = [System.IO.Path]::GetFileNameWithoutExtension($launcherExe)
$gameProcessName = [System.IO.Path]::GetFileNameWithoutExtension($gameExe)

$storageExistedBefore = $false
$gameExitCode = $null
$failed = $false

$linkSpecs = @(
    @{ Path = (Join-Path $renderDir 'winmm.dll'); Target = $projectProxy },
    @{ Path = (Join-Path $renderDir 'dlssg_sm86.ini'); Target = $projectIni }
)

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

function Assert-NoRunningProcess {
    param([string]$Name, [string]$Description)
    $running = @(Get-Process -Name $Name -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        throw "$Description 已经在运行，请先关闭它再使用此脚本。"
    }
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
        Write-Log '未发现本次生成的 _storage_\winmm.dll。'
        return
    }

    $sourceHash = Get-HashOrNull $projectProxy
    $storageHash = Get-HashOrNull $storageDll
    if ($storageHash -eq $sourceHash) {
        Remove-Item -LiteralPath $storageDll
        Write-Log '已删除本次生成的 _storage_\winmm.dll。'
    }
    else {
        Write-Warning '保留 _storage_\winmm.dll，因为 SHA256 与项目代理 DLL 不匹配。'
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

function Wait-ForGameProcess {
    param([int]$TimeoutSeconds = 120)

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $processes = @(Get-Process -Name $gameProcessName -ErrorAction SilentlyContinue)
        if ($processes.Count -gt 0) {
            Write-Log "已检测到游戏进程：$gameProcessName"
            return $processes[0]
        }

        Start-Sleep -Seconds 2
    } while ((Get-Date) -lt $deadline)

    throw "等待游戏进程超时（$TimeoutSeconds 秒）：$gameExe"
}

try {
    Assert-FileExists -Path $launcherExe -Description 'NTE launcher'
    Assert-FileExists -Path $gameExe -Description 'NTE game executable'
    Assert-FileExists -Path $projectProxy -Description 'DLSSG winmm.dll proxy'
    Assert-FileExists -Path $projectIni -Description 'DLSSG configuration INI'
    Assert-NoRunningProcess -Name $launcherProcessName -Description 'NTE launcher'
    Assert-NoRunningProcess -Name $gameProcessName -Description 'NTE game'

    $storageExistedBefore = Test-Path -LiteralPath $storageDll -PathType Leaf
    if ($storageExistedBefore) {
        Write-Log "发现启动前已存在的 _storage_\winmm.dll，将保留。SHA256：$(Get-HashOrNull $storageDll)"
    }

    if ($DryRun) {
        Write-Log '演练模式：环境检查通过。'
        Ensure-Link -Path $linkSpecs[0].Path -Target $linkSpecs[0].Target
        Ensure-Link -Path $linkSpecs[1].Path -Target $linkSpecs[1].Target
        exit 0
    }

    Ensure-Link -Path $linkSpecs[0].Path -Target $linkSpecs[0].Target
    Ensure-Link -Path $linkSpecs[1].Path -Target $linkSpecs[1].Target

    Write-Log '正在启动官方异环启动器。本脚本不会绕过或关闭 ACE。'
    $launcher = Start-Process -FilePath $launcherExe -WorkingDirectory $gameRoot -PassThru
    $game = Wait-ForGameProcess
    $game.WaitForExit()
    $gameExitCode = $game.ExitCode
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

    if (-not $DryRun) {
        Show-PostRunStatus
    }
}

if ($failed) { exit 1 }
if ($null -ne $gameExitCode) { exit $gameExitCode }
