[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$KeepLinks,
    [switch]$VerboseLog,
    [switch]$RestoreEngineConfig,
    [switch]$RestoreDeployment,
    [string]$ConfigDirectory,
    [string]$EngineConfigSource
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot '..\common\LauncherConfig.ps1')
$launcherRoot = Get-LauncherRoot

. (Join-Path $PSScriptRoot '..\common\ScriptLogging.ps1')
$launcherLogPath = Start-LauncherLog -ScriptPath $PSCommandPath
try {
. (Join-Path $PSScriptRoot '..\common\LauncherConsole.ps1')
. (Join-Path $PSScriptRoot '..\common\LauncherSession.ps1')
. (Join-Path $PSScriptRoot '..\common\LauncherResources.ps1')
. (Join-Path $PSScriptRoot '..\common\DeploymentJournal.ps1')
$settings = Get-LauncherGameSettings -GameId 'Nte' -Overrides $PSBoundParameters
$enginePrepared = $false
$engineOriginalMoved = $false
$engineRuntimeCreated = $false
$engineConfigSource = $settings.EngineConfigSource
$engineRuntimeMarker = '; NTE launcher runtime copy v1'
$ConfigDirectory = $settings.ConfigDirectory
$engineConfigPath = Join-Path $ConfigDirectory 'Engine.ini'
$engineRuntimePath = Join-Path $ConfigDirectory 'Engine.nteconfig-runtime.ini'
$engineBackupPath = Join-Path $ConfigDirectory 'Engine.ini.before-nteconfig-link'
$createdLinks = [Collections.Generic.List[string]]::new()
$setupComplete = $false
$consoleState = $null

$gameRoot = $settings.GameDirectory
$renderDir = Join-Path $gameRoot 'Client\WindowsNoEditor\HT\Binaries\Win64'
$launcherExe = $settings.LauncherPath
$launcherDir = if ($launcherExe) { Split-Path -Parent $launcherExe } else { '' }
$gameExe = Join-Path $renderDir 'HTGame.exe'
$projectDir = Split-Path -Parent $launcherRoot
$projectProxy = Join-Path $projectDir 'alternatives\winmm.dll'
$projectIni = Join-Path $projectDir 'dlssg_sm86.ini'
$diagnosticIniPath = Join-Path $env:TEMP ("dlssg_sm86-nte-{0}.ini" -f $PID)
$diagnosticIni = @'
; 异环 0.4.0 临时诊断配置。
; 由 Run-NevernessToEverness-DLSSG.ps1 生成。

[General]
Enabled=1

[FrameGeneration]
Engine=DLSS
MaxGeneratedFrames=5

[DLSSG]
Optimized=2
Preset=Auto

[Logging]
Level=2
Directory=dlssg_sm86\logs

[Runtime]
Mode=Bundled

[SuperResolution]
Mode=On
'@
$storageDll = Join-Path $renderDir '_storage_\winmm.dll'
$logDir = Join-Path $renderDir 'dlssg_sm86\logs'
$launcherProcessName = @(Get-LauncherProcessNames -GameId Nte -LauncherPath $launcherExe)
$gameProcessName = [System.IO.Path]::GetFileNameWithoutExtension($gameExe)

$deployment = New-DlssgDeployment -ProjectDirectory $projectDir -RenderDirectory $renderDir -ProxyName 'winmm.dll'
$storageExistedBefore = $false
$gameExitCode = $null
$failed = $false
$setupStarted = $false

$linkSpecs = @(
    @{ Path = (Join-Path $renderDir 'winmm.dll'); Target = $projectProxy },
    @{ Path = (Join-Path $renderDir 'dlssg_sm86.ini'); Target = $diagnosticIniPath }
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
    param([string[]]$Name, [string]$Description)
    $running = @(Get-Process -Name $Name -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        throw "$Description 已经在运行，请先关闭它再使用此脚本。"
    }
}

function Assert-NteEnginePaths {
    foreach ($path in @($engineConfigPath, $engineRuntimePath, $engineBackupPath)) {
        if ((Get-NormalizedPath $engineConfigSource) -eq (Get-NormalizedPath $path)) {
            throw '配置源不能使用游戏原文件、临时文件或备份路径。'
        }
    }
}

function Assert-OwnedEngineRuntime {
    Assert-FileExists -Path $engineRuntimePath -Description '脚本生成的 Engine.ini 临时副本'
    # 兼容旧版可写副本；恢复时检查普通文件，不要求标记注释或只读属性。
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
    Remove-Item -LiteralPath $engineRuntimePath -Force
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
        Assert-NoRunningProcess -Name $launcherProcessName -Description 'NTE launcher'
        Assert-NoRunningProcess -Name $gameProcessName -Description 'NTE game'
        Restore-EngineConfig
        Write-Log '已恢复上次配置，继续准备本次游戏启动。'
    }
    $original = Get-Item -LiteralPath $engineConfigPath -Force -ErrorAction SilentlyContinue
    if ($null -ne $original -and (($original.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $original.PSIsContainer)) {
        throw "原 Engine.ini 是链接或目录，拒绝替换：$engineConfigPath"
    }
    if ($DryRun) {
        Write-Log "演练模式：将备份原 Engine.ini 到 $engineBackupPath（若原文件存在）"
        Write-Log "演练模式：将从 $engineConfigSource 生成只读临时副本 $engineRuntimePath"
        Write-Log "演练模式：将创建软链接 $engineConfigPath -> $engineRuntimePath"
        return
    }
    $engineText = [IO.File]::ReadAllText($engineConfigSource)
    $record = Start-DeploymentOperation -Kind runtime -Path $engineRuntimePath
    [IO.File]::WriteAllText($engineRuntimePath, ($engineRuntimeMarker + "`r`n" + $engineText), [Text.UTF8Encoding]::new($false))
    [IO.File]::SetAttributes($engineRuntimePath, ([IO.File]::GetAttributes($engineRuntimePath) -bor [IO.FileAttributes]::ReadOnly))
    $script:engineRuntimeCreated = $true
    Complete-DeploymentOperation $record
    if ($null -ne $original) {
        $record = Start-DeploymentOperation -Kind backup -Path $engineConfigPath -Target $engineBackupPath -Hash (Get-HashOrNull $engineConfigPath)
        Move-Item -LiteralPath $engineConfigPath -Destination $engineBackupPath
        Complete-DeploymentOperation $record
        $script:engineOriginalMoved = $true
    }
    $record = Start-DeploymentOperation -Kind engineLink -Path $engineConfigPath -Target $engineRuntimePath
    New-Item -ItemType SymbolicLink -Path $engineConfigPath -Target $engineRuntimePath | Out-Null
    Complete-DeploymentOperation $record
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
    if ($engineRuntimeCreated) { Remove-Item -LiteralPath $engineRuntimePath -Force }
}

function Test-StaleDiagnosticIniTarget {
    param([string]$Path, [string]$Target)

    if ((Get-NormalizedPath $Path) -ne (Get-NormalizedPath $linkSpecs[1].Path)) { return $false }
    if ((Split-Path -Parent (Get-NormalizedPath $Target)) -ne (Get-NormalizedPath $env:TEMP)) { return $false }
    if ([System.IO.Path]::GetFileName($Target) -notmatch '^dlssg_sm86-nte-\d+\.ini$') { return $false }
    if (-not (Test-Path -LiteralPath $Target -PathType Leaf)) { return $false }

    $contents = [System.IO.File]::ReadAllText($Target)
    return ($contents -ceq $diagnosticIni)
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
        if ((Get-NormalizedPath $Path) -eq (Get-NormalizedPath $linkSpecs[1].Path) -and
            (Get-NormalizedPath $actualTarget) -eq (Get-NormalizedPath $projectIni)) {
            if (-not $DryRun) { Remove-Item -LiteralPath $Path }
            Write-Log '将原仓库 INI 链接迁移为本次临时配置；仓库文件保持不变。'
            return $false
        }
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

    if (Assert-LinkOrAbsent -Path $Path -ExpectedTarget $Target) {
        if (-not $DryRun) { Register-ReusedDeploymentLink -Path $Path -Target $Target }
        return
    }

    if ($DryRun) {
        Write-Log "演练模式：将创建软链接 $Path -> $Target"
        return
    }

    $record = Start-DeploymentOperation -Kind link -Path $Path -Target $Target
    New-Item -ItemType SymbolicLink -Path $Path -Target $Target | Out-Null
    Complete-DeploymentOperation $record
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
        Write-Log '发现日志仅表示有日志文件，帧生成是否生效需检查加载和内核记录。'
    }
    else {
        Write-Warning "目录下未找到 DLSSG 日志：$logDir"
    }
}

if (-not $DryRun -and -not $RestoreEngineConfig -and -not $RestoreDeployment) { $consoleState = Initialize-LauncherConsole -ScriptPath $PSCommandPath -Parameters $PSBoundParameters }

try {
    Assert-NteEnginePaths
    Assert-NoRunningProcess -Name $launcherProcessName -Description 'NTE launcher'
    Assert-NoRunningProcess -Name $gameProcessName -Description 'NTE game'
    Open-DeploymentJournal -GameId 'nte' -GameProcess $gameProcessName -LauncherProcess $launcherProcessName -DiagnosticPrefix 'dlssg_sm86-nte-'
    if ($RestoreDeployment) {
        if ($null -ne $script:deploymentJournal) { Restore-RecordedDeployment -Preview:$DryRun }
        elseif ($null -ne (Get-JournalItem $engineBackupPath) -or $null -ne (Get-JournalItem $engineRuntimePath)) { Restore-EngineConfig }
        Restore-LegacyResourceDeployment -Preview:$DryRun
        exit 0
    }
    if ($RestoreEngineConfig) {
        if ($null -ne $script:deploymentJournal) { Restore-RecordedDeployment -EngineOnly -Preview:$DryRun }
        elseif ($null -ne (Get-JournalItem $engineBackupPath) -or $null -ne (Get-JournalItem $engineRuntimePath)) { Restore-EngineConfig }
        else { Write-Log '没有需要恢复的画面配置。' }
        exit 0
    }
    Resolve-PreviousDeployment
    Resolve-LegacyEngineDeployment
    Assert-FileExists -Path $engineConfigSource -Description '异环明文 Engine.ini 配置源'
    if (-not (Test-Path -LiteralPath $ConfigDirectory -PathType Container)) { throw "配置目录不存在：$ConfigDirectory" }
    [void](Save-LauncherDetectedConfigDirectory -GameId Nte -Directory $ConfigDirectory -Overrides $PSBoundParameters)
    Write-Log "本次配置目录：$ConfigDirectory"
    Assert-FileExists -Path $launcherExe -Description 'NTE launcher'
    Assert-FileExists -Path $gameExe -Description 'NTE game executable'
    Assert-FileExists -Path $projectProxy -Description 'DLSSG winmm.dll proxy'
    Assert-FileExists -Path $projectIni -Description 'DLSSG configuration INI'
    Assert-NoRunningProcess -Name $launcherProcessName -Description 'NTE launcher'
    Assert-NoRunningProcess -Name $gameProcessName -Description 'NTE game'

    Assert-DlssgDeployment $deployment
    $storageExistedBefore = Test-Path -LiteralPath $storageDll -PathType Leaf
    if ($storageExistedBefore) {
        Write-Log "发现启动前已存在的 _storage_\winmm.dll，将按项目代理哈希核验归属。SHA256：$(Get-HashOrNull $storageDll)"
    }

    if ($DryRun) {
        Install-DlssgResources -Deployment $deployment -DryRun
        Write-Log '演练模式：环境检查通过；临时配置启用 INT8 超分（Mode=On），仅 L/M 预设会使用 INT8。'
        Ensure-Link -Path $linkSpecs[0].Path -Target $linkSpecs[0].Target
        Ensure-Link -Path $linkSpecs[1].Path -Target $linkSpecs[1].Target
        Prepare-EngineConfig
        exit 0
    }

    Test-DeploymentInputs
    New-DeploymentJournal
    $setupStarted = $true
    Register-OwnedStorageDll
    Install-DlssgResources -Deployment $deployment
    Prepare-EngineConfig
    $record = Start-DeploymentOperation -Kind diagnostic -Path $diagnosticIniPath -Hash (Get-JournalTextHash $diagnosticIni)
    [IO.File]::WriteAllText($diagnosticIniPath, $diagnosticIni, [Text.UTF8Encoding]::new($false))
    Complete-DeploymentOperation $record
    Ensure-Link -Path $linkSpecs[0].Path -Target $linkSpecs[0].Target
    Ensure-Link -Path $linkSpecs[1].Path -Target $linkSpecs[1].Target

    Set-DeploymentPhase 'active'
    $setupComplete = $true
    Write-Log '正在启动官方异环启动器。本脚本不会绕过或关闭 ACE。'
    if (-not $storageExistedBefore) {
        $record = Start-DeploymentOperation -Kind storage -Path $storageDll -Hash (Get-HashOrNull $projectProxy)
        Complete-DeploymentOperation $record
    }
    $launcher = Start-Process -FilePath $launcherExe -WorkingDirectory $launcherDir -PassThru
    $game = Wait-ForGameProcess -GameProcessName $gameProcessName -LauncherProcessName $launcherProcessName
    if ($null -ne $game) {
        $game.WaitForExit()
        $gameExitCode = $game.ExitCode
        Write-Log "游戏已退出，退出代码：$gameExitCode"
    }
}
catch {
    $failed = $true
    Write-Error $_
}
finally {
    try {
        $gameStillRunning = @(Get-Process -Name $gameProcessName -ErrorAction SilentlyContinue).Count -gt 0
        if ($setupStarted -and -not $DryRun -and -not $gameStillRunning -and (-not $KeepLinks -or -not $setupComplete)) {
            try { Restore-RecordedDeployment -Automatic } catch { Write-Warning $_; $failed = $true }
        }
        elseif ($setupStarted -and -not $DryRun) {
            Set-DeploymentPhase 'retained'
            Write-Warning '部署与记录已保留；关闭游戏和启动器后用 -RestoreDeployment 完整恢复。'
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
exit 0

}
catch {
    # Include the full error and source position even for errors before main setup.
    Write-Host ($_ | Format-List * -Force | Out-String) -ForegroundColor Red
    throw
}
finally {
    Close-DeploymentJournal
    Stop-LauncherLog -Path $launcherLogPath
}
