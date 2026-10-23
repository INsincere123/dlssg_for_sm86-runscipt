[CmdletBinding()]
param(
    [ValidatePattern('^[1-9][0-9]*$')][string]$AppId,
    [ValidateSet('Steam','Direct')][string]$LaunchMode='Steam',
    [ValidatePattern('^[a-f0-9]{32}$')][string]$GameKey,
    [string]$Profile,
    [string]$LauncherConfigPath,
    [ValidatePattern('^[1-6]$')][string]$ProxyDll,
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
. (Join-Path $PSScriptRoot '..\common\LauncherResources.ps1')
. (Join-Path $PSScriptRoot '..\common\DeploymentJournal.ps1')
. (Join-Path $PSScriptRoot '..\common\SteamLauncher.ps1')
. (Join-Path $PSScriptRoot '..\common\DirectLauncher.ps1')
if ($LaunchMode -eq 'Direct') {
    if (-not $GameKey) { throw '直接启动缺少稳定的游戏标识。' }
    $settingsArguments=@{GameKey=$GameKey;Profile=$Profile;Overrides=$PSBoundParameters}
    if ($LauncherConfigPath) { $settingsArguments.Path=$LauncherConfigPath }
    $settings=Get-DirectUESettings @settingsArguments
    $gameId=$settings.GameId
    Write-Host ("配置分组：{0}；直接EXE启动；游戏标识：{1}" -f $settings.Profile,$gameId)
} else {
    $settingsArguments = @{ AppId=$AppId; Profile=$Profile; Overrides=$PSBoundParameters }
    if ($LauncherConfigPath) { $settingsArguments.Path = $LauncherConfigPath }
    $settings = Get-SteamUE5Settings @settingsArguments
    $AppId = $settings.AppId
    Write-Host ("配置分组：{0}；AppID：{1}（{2}）" -f $settings.Profile, $AppId, $settings.AppIdSource)
    $gameId="steam-$AppId"
}
$enginePrepared = $false
$engineOriginalMoved = $false
$engineRuntimeCreated = $false
$engineConfigSource = $settings.EngineConfigSource
$engineRuntimeMarker = if ($LaunchMode -eq 'Steam') { '; Steam UE5 launcher runtime copy v1 appid=' + $AppId } else { '; Direct UE launcher runtime copy v1 game=' + $gameId }
$ConfigDirectory = $settings.ConfigDirectory
$engineConfigPath = Join-Path $ConfigDirectory 'Engine.ini'
$engineRuntimePath = Join-Path $ConfigDirectory ('Engine.' + $gameId + '-runtime.ini')
$engineBackupPath = Join-Path $ConfigDirectory ('Engine.ini.before-' + $gameId + '-link')
$createdLinks = [Collections.Generic.List[string]]::new()
$setupComplete = $false
$consoleState = $null

$gameRoot = $settings.GameDirectory
$gameExe = $settings.RenderExecutable
$renderDir = Split-Path -Parent $gameExe
$launcherExe = $settings.LauncherPath
$launcherDir = if ($launcherExe) { Split-Path -Parent $launcherExe } else { '' }

$projectDir = Split-Path -Parent $launcherRoot
$projectIni = Join-Path $projectDir 'dlssg_sm86.ini'
$diagnosticIniPath = Join-Path $env:TEMP ("dlssg_sm86-{1}-{0}.ini" -f $PID, $gameId)
$diagnosticIni = @'
; Steam UE5 0.4.0 临时诊断配置。
; 由 Run-SteamUE5-DLSSG.ps1 生成。

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
Mode=On
'@
if ($LaunchMode -eq 'Direct') { $diagnosticIni=$diagnosticIni.Replace('; Steam UE5 0.4.0','; Direct UE 0.4.0') }
$logDir = Join-Path $renderDir 'dlssg_sm86\logs'
$launcherProcessName = @()
$gameProcessName = [System.IO.Path]::GetFileNameWithoutExtension($gameExe)

$storageExistedBefore = $false
$gameExitCode = $null
$failed = $false
$setupStarted = $false

$linkSpecs = @(
    @{},
    @{ Path = (Join-Path $renderDir 'dlssg_sm86.ini'); Target = $diagnosticIniPath }
)
Select-DlssgProxy -ProjectDirectory $projectDir -RenderDirectory $renderDir -ProxyName $settings.ProxyName

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

function Assert-SteamEnginePaths {
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
        Assert-SteamGameStopped -Executable $gameExe
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

function Test-StaleDiagnosticIniTarget {
    param([string]$Path, [string]$Target)

    if ((Get-NormalizedPath $Path) -ne (Get-NormalizedPath $linkSpecs[1].Path)) { return $false }
    if ((Split-Path -Parent (Get-NormalizedPath $Target)) -ne (Get-NormalizedPath $env:TEMP)) { return $false }
    if ([System.IO.Path]::GetFileName($Target) -notmatch ('^dlssg_sm86-' + [regex]::Escape($gameId) + '-\d+\.ini$')) { return $false }
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
    Assert-SteamEnginePaths
    Assert-SteamGameStopped -Executable $gameExe
    Open-DeploymentJournal -GameId $gameId -GameProcess $gameProcessName -LauncherProcess @() -DiagnosticPrefix ("dlssg_sm86-$gameId-") -GameExecutable $gameExe -SelectableProxy
    if ($null -ne $script:deploymentJournal) {
        Write-Log ("旧部署恢复代理：{0}；来源：{1}" -f $deployment.Proxy, $projectProxy)
    }
    if ($RestoreDeployment) {
        if ($null -ne $script:deploymentJournal) { Restore-RecordedDeployment -Preview:$DryRun; exit 0 }
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
    Select-DlssgProxy -ProjectDirectory $projectDir -RenderDirectory $renderDir -ProxyName $settings.ProxyName
    Write-Log ("游戏：{0}（{1}）；代理 {2}：{3}；来源：{4}" -f $settings.GameName, $gameId, $settings.ProxyDll, $deployment.Proxy, $projectProxy)
    Write-Log "渲染 EXE：$gameExe"
    Resolve-LegacyEngineDeployment
    Assert-FileExists -Path $engineConfigSource -Description 'UE 明文 Engine.ini 配置源'
    if (-not (Test-Path -LiteralPath $ConfigDirectory -PathType Container)) { throw "配置目录不存在，请先原版启动一次或使用 -ConfigDirectory 指定：$ConfigDirectory" }
    Write-Log "本次配置目录：$ConfigDirectory"
    Assert-FileExists -Path $launcherExe -Description '启动程序'
    Assert-FileExists -Path $gameExe -Description '游戏渲染 EXE'
    Assert-FileExists -Path $projectProxy -Description '所选 DLSSG 代理'
    Assert-FileExists -Path $projectIni -Description 'DLSSG configuration INI'
    Assert-SteamGameStopped -Executable $gameExe

    Assert-DlssgDeployment $deployment
    $storageExistedBefore = Test-Path -LiteralPath $storageDll -PathType Leaf
    if ($storageExistedBefore) {
        Write-Log "发现启动前已存在的 _storage_\$($deployment.Proxy)，将按项目代理哈希核验归属。SHA256：$(Get-HashOrNull $storageDll)"
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
    Write-Log ("启动方式：{0}；游戏：{1}；启动程序：{2}" -f $LaunchMode,$settings.GameName,$launcherExe)
    if (-not $storageExistedBefore) {
        $record = Start-DeploymentOperation -Kind storage -Path $storageDll -Hash (Get-HashOrNull $projectProxy)
        Complete-DeploymentOperation $record
    }
    Start-ConfiguredUEGame -LaunchMode $LaunchMode -Executable $launcherExe -AppId $AppId
    Wait-SteamGameSession -Executable $gameExe

}
catch {
    $failed = $true
    Write-Error $_
}
finally {
    try {
        $gameStillRunning = $true
        try { $gameStillRunning = @(Get-SteamGameProcesses -Executable $gameExe).Count -gt 0 }
        catch { Write-Warning $_; $failed = $true }
        if ($setupStarted -and -not $DryRun -and -not $gameStillRunning -and (-not $KeepLinks -or -not $setupComplete)) {
            try { Restore-RecordedDeployment -Automatic } catch { Write-Warning $_; $failed = $true }
        }
        elseif ($setupStarted -and -not $DryRun) {
            Set-DeploymentPhase 'retained'
            Write-Warning '部署与记录已保留；退出目标游戏后用原 -Profile 和 -RestoreDeployment 完整恢复；Steam清单丢失时额外指定原 -AppId；直接启动保留原 -LaunchMode Direct -GameKey。'
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
