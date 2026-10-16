[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$KeepLinks,
    [switch]$VerboseLog,
    [string]$ConfigDirectory,
    [switch]$RestoreEngineConfig,
    [ValidateRange(1, 3600)]
    [int]$RestartGraceSeconds = 10
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'ScriptLogging.ps1')
$launcherLogPath = Start-LauncherLog -ScriptPath $PSCommandPath
try {
. (Join-Path $PSScriptRoot 'LauncherConsole.ps1')
. (Join-Path $PSScriptRoot 'LauncherResources.ps1')
$createdLinks = [Collections.Generic.List[string]]::new()
$setupComplete = $false
$consoleState = $null

$gameDir = 'C:\Games\Wuthering Waves Game'
$launcherDir = 'C:\Program Files\Wuthering Waves'
$launcherExe = Join-Path $launcherDir 'launcher.exe'
$renderDir = Join-Path $gameDir 'Client\Binaries\Win64'
$gameExe = Join-Path $renderDir 'Client-Win64-Shipping.exe'
$projectDir = Split-Path -Parent $PSScriptRoot
$projectDll = Join-Path $projectDir 'version.dll'
$storageDll = Join-Path $renderDir '_storage_\version.dll'
$logDir = Join-Path $renderDir 'dlssg_sm86\logs'
$launcherProcessName = [System.IO.Path]::GetFileNameWithoutExtension($launcherExe)
$processName = [System.IO.Path]::GetFileNameWithoutExtension($gameExe)
$diagnosticIniPath = Join-Path $env:TEMP ("dlssg_sm86-wuthering-{0}.ini" -f $PID)
$engineConfigSource = 'D:\Games\WuWa-Configs\当前配置\Engine.ini'
$engineConfigPath = $null
$engineRuntimePath = $null
$engineBackupPath = $null
$engineRuntimeMarker = '; WuWa-Configs launcher runtime copy v1'
$enginePrepared = $false
$engineOriginalMoved = $false
$engineRuntimeCreated = $false
$setupStarted = $false

$legacyDiagnosticIni = @'
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

$diagnosticIni = @'
; 鸣潮 0.4.0 临时诊断配置。
; 由 Run-WutheringWaves-DLSSG.ps1 生成。

[General]
Enabled=1

[FrameGeneration]
Engine=DLSS
MaxGeneratedFrames=3

[DLSSG]
Optimized=2
Preset=Auto

[Logging]
Level=2
Directory=dlssg_sm86\logs

[Runtime]
Mode=Bundled

[SuperResolution]
Mode=Off
'@

$linkSpecs = @(
    @{ Path = (Join-Path $renderDir 'version.dll'); Target = $projectDll },
    @{ Path = (Join-Path $renderDir 'dlssg_sm86.ini'); Target = $diagnosticIniPath }
)

$deployment = New-DlssgDeployment -ProjectDirectory $projectDir -RenderDirectory $renderDir -ProxyName 'version.dll'
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

function Resolve-EngineConfigDirectory {
    if ($ConfigDirectory) {
        $resolved = (Resolve-Path -LiteralPath $ConfigDirectory -ErrorAction Stop).ProviderPath
        if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
            throw "配置目录不存在：$resolved"
        }
        Assert-FileExists -Path (Join-Path $resolved 'GameUserSettings.ini') -Description 'GameUserSettings.ini'
        return $resolved
    }

    # 不绑定 WindowsNoEditor；只在本次安装的用户存档区查找，不按时间猜测。
    $savedRoot = Join-Path $gameDir 'Client\Saved'
    $candidates = @(Get-ChildItem -LiteralPath $savedRoot -Filter 'GameUserSettings.ini' -File -Recurse -ErrorAction Stop |
        Where-Object { (Get-Content -LiteralPath $_.FullName -Raw) -match '(?m)^\[/Script/Engine\.GameUserSettings\]' } |
        Select-Object -ExpandProperty DirectoryName -Unique)
    if ($candidates.Count -ne 1) {
        throw "找到 $($candidates.Count) 个候选配置目录，无法自动确认。请用 -ConfigDirectory 指定当前目录。候选：$($candidates -join '；')"
    }
    return $candidates[0]
}

function Assert-OwnedEngineRuntime {
    Assert-FileExists -Path $engineRuntimePath -Description '脚本生成的 Engine.ini 临时副本'
    # 游戏可能重写副本并移除注释，因此这里只检查它是不是普通文件。
    $runtimeItem = Get-Item -LiteralPath $engineRuntimePath -Force
    if ($runtimeItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "临时副本被替换为链接，拒绝操作：$engineRuntimePath"
    }
}

function Restore-EngineConfig {
    $item = Get-Item -LiteralPath $engineConfigPath -Force -ErrorAction SilentlyContinue
    if ($null -ne $item) {
        if ($item.LinkType -ne 'SymbolicLink' -or
            (Get-NormalizedPath ([string]($item.Target -join ''))) -ne (Get-NormalizedPath $engineRuntimePath)) {
            throw "Engine.ini 已不是本脚本的链接，拒绝覆盖。原文件备份（如存在）：$engineBackupPath"
        }
    }
    Assert-OwnedEngineRuntime
    if ($DryRun) {
        Write-Log "演练模式：将移除 Engine.ini 临时链接和副本，并恢复原文件（如存在）：$engineBackupPath"
        return
    }
    if ($null -ne $item) { Remove-Item -LiteralPath $engineConfigPath }
    if (Test-Path -LiteralPath $engineBackupPath) {
        Move-Item -LiteralPath $engineBackupPath -Destination $engineConfigPath
        Write-Log "已恢复原 Engine.ini：$engineConfigPath"
    }
    Remove-Item -LiteralPath $engineRuntimePath
}

function Prepare-EngineConfig {
    # 固定名称便于中断后恢复；恢复前征求同意，不直接覆盖遗留文件。
    $leftovers = @(@($engineBackupPath, $engineRuntimePath) | Where-Object {
        $null -ne (Get-Item -LiteralPath $_ -Force -ErrorAction SilentlyContinue)
    })
    if ($leftovers.Count -gt 0) {
        Write-Log "检测到上次运行遗留的画面配置：$($leftovers -join '；')"
        if ($DryRun) {
            Write-Log '演练模式：正式运行时将询问是否恢复原 Engine.ini，演练不会修改遗留文件。'
            return
        }
        $answer = Read-Host '是否恢复原 Engine.ini 并继续启动游戏？[y/N]'
        if ($answer.Trim() -notmatch '^(?i:y|yes|是|同意)$') {
            throw '已取消启动，遗留配置保持不变。'
        }
        # 用户确认期间可能启动了游戏，恢复前再次检查。
        Assert-NoGameProcess
        Restore-EngineConfig
        Write-Log '已恢复上次配置，继续准备本次游戏启动。'
    }
    $original = Get-Item -LiteralPath $engineConfigPath -Force -ErrorAction SilentlyContinue
    if ($null -ne $original -and (($original.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $original.PSIsContainer)) {
        throw "原 Engine.ini 是链接或目录，拒绝替换：$engineConfigPath"
    }
    if ($DryRun) {
        Write-Log "演练模式：将备份原 Engine.ini 到 $engineBackupPath（若原文件存在）"
        Write-Log "演练模式：将从 $engineConfigSource 生成可写副本 $engineRuntimePath"
        Write-Log "演练模式：将创建软链接 $engineConfigPath -> $engineRuntimePath"
        return
    }
    $engineText = [IO.File]::ReadAllText($engineConfigSource)
    [IO.File]::WriteAllText($engineRuntimePath, ($engineRuntimeMarker + "`r`n" + $engineText), [Text.UTF8Encoding]::new($false))
    $script:engineRuntimeCreated = $true
    if ($null -ne $original) {
        Move-Item -LiteralPath $engineConfigPath -Destination $engineBackupPath
        $script:engineOriginalMoved = $true
    }
    New-Item -ItemType SymbolicLink -Path $engineConfigPath -Target $engineRuntimePath | Out-Null
    $script:enginePrepared = $true
    Write-Log "已启用画面配置：$engineConfigSource"
    Write-Log "Engine.ini 软链接：$engineConfigPath -> $engineRuntimePath"
}

function Undo-EngineConfigSetup {
    if ($enginePrepared) { Restore-EngineConfig; return }
    # 包括缺少创建链接权限等部分失败；只撤回本次已经完成的步骤。
    if ($engineOriginalMoved) {
        if ($null -ne (Get-Item -LiteralPath $engineConfigPath -Force -ErrorAction SilentlyContinue)) {
            throw "恢复路径被占用，保留备份：$engineBackupPath"
        }
        Move-Item -LiteralPath $engineBackupPath -Destination $engineConfigPath
    }
    if ($engineRuntimeCreated) { Remove-Item -LiteralPath $engineRuntimePath }
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

function Wait-ForGameSession {
    param([System.Diagnostics.Process]$InitialProcess)

    $currentProcess = $InitialProcess
    while ($true) {
        $currentProcess.WaitForExit()
        $exitCode = $currentProcess.ExitCode
        Write-Log "游戏进程已退出，退出代码：$exitCode。等待重启，连续 $RestartGraceSeconds 秒未发现游戏才清理退出。"

        # 使用单调计时，避免系统时间调整影响热更新等待窗口。
        $absenceTimer = [System.Diagnostics.Stopwatch]::StartNew()
        while ($true) {
            $processes = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
            if ($processes.Count -gt 0) {
                $currentProcess = $processes[0]
                Write-Log "检测到游戏重新运行（PID：$($currentProcess.Id)），继续保留配置并等待游戏退出。"
                break
            }
            if ($absenceTimer.Elapsed.TotalSeconds -ge $RestartGraceSeconds) {
                Write-Log "连续 $RestartGraceSeconds 秒未发现游戏进程，本次游戏会话结束。"
                return $exitCode
            }
            Start-Sleep -Milliseconds 500
        }
    }
}

function Test-StaleDiagnosticIniTarget {
    param([string]$Path, [string]$Target)

    if ((Get-NormalizedPath $Path) -ne (Get-NormalizedPath $linkSpecs[1].Path)) { return $false }
    if ((Split-Path -Parent (Get-NormalizedPath $Target)) -ne (Get-NormalizedPath $env:TEMP)) { return $false }
    if ([System.IO.Path]::GetFileName($Target) -notmatch '^dlssg_sm86-wuthering-\d+\.ini$') { return $false }
    if (-not (Test-Path -LiteralPath $Target -PathType Leaf)) { return $false }

    $contents = [System.IO.File]::ReadAllText($Target)
    return ($contents -ceq $diagnosticIni -or $contents -ceq $legacyDiagnosticIni)
}

function Assert-LinkOrAbsent {
    param([string]$Path, [string]$ExpectedTarget)

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
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
    $createdLinks.Add($Path)
    Write-Log "已创建软链接：$Path -> $Target"
}

function Remove-ExpectedLink {
    param([string]$Path, [string]$ExpectedTarget)

    if (-not $createdLinks.Contains($Path)) { return }
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
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
        Write-Log '发现日志仅表示有日志文件，帧生成是否生效需检查加载和内核记录。'
    }
    else {
        Write-Warning "目录下未找到 DLSSG 日志：$logDir"
    }
}

# 恢复配置不启动游戏，无需控制台保护；避免终端不支持菜单操作时无法恢复。
if (-not $RestoreEngineConfig -and -not $DryRun) {
    $consoleState = Initialize-LauncherConsole -ScriptPath $PSCommandPath -Parameters $PSBoundParameters
}

try {
    Assert-NoGameProcess
    $resolvedConfigDir = Resolve-EngineConfigDirectory
    $engineConfigPath = Join-Path $resolvedConfigDir 'Engine.ini'
    $engineRuntimePath = Join-Path $resolvedConfigDir 'Engine.wuwaconfig-runtime.ini'
    $engineBackupPath = Join-Path $resolvedConfigDir 'Engine.ini.before-wuwaconfig-link'
    Write-Log "本次配置目录：$resolvedConfigDir"
    if ($RestoreEngineConfig) {
        Restore-EngineConfig
        exit 0
    }
    Assert-FileExists -Path $launcherExe -Description 'Wuthering Waves launcher'
    Assert-FileExists -Path $gameExe -Description 'Wuthering Waves rendering executable'
    Assert-FileExists -Path $projectDll -Description 'Project version.dll'
    Assert-FileExists -Path $engineConfigSource -Description 'WuWa-Configs 当前 Engine.ini'
    if (Test-Path -LiteralPath (Join-Path $gameDir 'Client\Config\UserEngine.ini')) {
        throw '发现旧 Client\Config\UserEngine.ini，请先备份并移出该文件，再使用当前配置。'
    }

    Assert-DlssgDeployment $deployment
    $storageExistedBefore = Test-Path -LiteralPath $storageDll -PathType Leaf
    if ($storageExistedBefore) {
        Write-Log "发现启动前已存在的 _storage_\\version.dll，将保留。SHA256：$(Get-HashOrNull $storageDll)"
    }

    if ($DryRun) {
        Install-DlssgResources -Deployment $deployment -DryRun
        Write-Log '演练模式：环境检查通过。'
        Ensure-Link -Path $linkSpecs[0].Path -Target $linkSpecs[0].Target
        Ensure-Link -Path $linkSpecs[1].Path -Target $linkSpecs[1].Target
        Prepare-EngineConfig
        exit 0
    }

    $setupStarted = $true
    Install-DlssgResources -Deployment $deployment
    Prepare-EngineConfig
    [System.IO.File]::WriteAllText($diagnosticIniPath, $diagnosticIni, [System.Text.UTF8Encoding]::new($false))
    Ensure-Link -Path $linkSpecs[0].Path -Target $linkSpecs[0].Target
    Ensure-Link -Path $linkSpecs[1].Path -Target $linkSpecs[1].Target

    $setupComplete = $true
    Write-Log '正在启动鸣潮官方启动器。本脚本不会绕过或关闭 ACE。'
    $launcher = Start-Process -FilePath $launcherExe -WorkingDirectory $launcherDir -PassThru
    $process = Wait-ForGameProcess
    $gameExitCode = Wait-ForGameSession -InitialProcess $process
}
catch {
    $failed = $true
    Write-Error $_
}
finally {
    try {
        $gameStillRunning = @(Get-Process -Name $processName -ErrorAction SilentlyContinue).Count -gt 0
        if ($setupStarted -and -not $DryRun -and -not $gameStillRunning -and (-not $KeepLinks -or -not $setupComplete)) {
            try { Undo-DlssgResources $deployment } catch { Write-Warning $_ }
            try { Undo-EngineConfigSetup } catch { Write-Warning $_ }
            try { Remove-ExpectedLink -Path $linkSpecs[1].Path -ExpectedTarget $linkSpecs[1].Target } catch { Write-Warning $_ }
            try { Remove-ExpectedLink -Path $linkSpecs[0].Path -ExpectedTarget $linkSpecs[0].Target } catch { Write-Warning $_ }
            try { Remove-GeneratedStorageDll } catch { Write-Warning $_ }
        }
        elseif ($setupStarted -and $KeepLinks -and -not $DryRun) {
            Write-Log '已指定 KeepLinks，临时软链接和 storage 文件不会删除。'
            Write-Log '下次启动前先运行 -RestoreEngineConfig 恢复 Engine.ini。'
        }
        if ($setupStarted -and $gameStillRunning) {
            Write-Warning '游戏仍在运行，保留配置链接与备份。关闭游戏和启动器后可用 -RestoreEngineConfig 恢复 Engine.ini。'
        }

        if ($setupStarted -and -not $DryRun -and (-not $KeepLinks -or -not $setupComplete) -and -not $gameStillRunning -and (Test-Path -LiteralPath $diagnosticIniPath -PathType Leaf)) {
            Remove-Item -LiteralPath $diagnosticIniPath -ErrorAction SilentlyContinue
        }

        if ($setupStarted -and -not $DryRun) {
            Show-PostRunStatus
        }
    }
    finally {
        Restore-LauncherConsole -State $consoleState
    }
}

if ($failed) { exit 1 }
if ($null -ne $gameExitCode) { exit $gameExitCode }

}
catch {
    # Include the full error and source position even for errors before main setup.
    Write-Host ($_ | Format-List * -Force | Out-String) -ForegroundColor Red
    throw
}
finally {
    Stop-LauncherLog -Path $launcherLogPath
}
