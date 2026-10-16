# Shared deployment journal, Windows PowerShell 5.1+. No configuration contents or keys.
$script:deploymentJournal = $null
$script:deploymentMutex = $null
$script:configurationMutex = $null
$script:journalPath = $null

function Get-JournalItem {
    param([string]$Path)
    try { Get-Item -LiteralPath $Path -Force -ErrorAction Stop }
    catch [System.Management.Automation.ItemNotFoundException] { return $null }
}

function Assert-JournalOrdinaryParents {
    param([string]$Path)
    $parent = Split-Path -Parent ([IO.Path]::GetFullPath($Path))
    while ($parent) {
        $item = Get-JournalItem $parent
        if ($null -ne $item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Reparse-point parent refused: $parent"
        }
        $next = Split-Path -Parent $parent
        if ($next -eq $parent) { break }
        $parent = $next
    }
}

function Save-DeploymentJournal {
    Assert-JournalOrdinaryParents $script:journalPath
    $existing = Get-JournalItem $script:journalPath
    if ($null -ne $existing -and ($existing.PSIsContainer -or ($existing.Attributes -band [IO.FileAttributes]::ReparsePoint))) {
        throw 'Journal path is not an ordinary file.'
    }
    $directory = Split-Path -Parent $script:journalPath
    [void][IO.Directory]::CreateDirectory($directory)
    $temporary = Join-Path $directory ([guid]::NewGuid().ToString('N') + '.tmp')
    $previous = $temporary + '.previous'
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($script:deploymentJournal | ConvertTo-Json -Depth 12))
        $stream = [IO.FileStream]::new($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($script:journalPath)) { [IO.File]::Replace($temporary, $script:journalPath, $previous) }
        else { [IO.File]::Move($temporary, $script:journalPath) }
    }
    finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
        if ([IO.File]::Exists($previous)) { [IO.File]::Delete($previous) }
    }
}

function Open-DeploymentJournal {
    param([string]$GameId, [string]$GameProcess, [string]$LauncherProcess, [string]$DiagnosticPrefix)
    $script:journalContext = @{
        GameId = $GameId; GameProcess = $GameProcess; LauncherProcess = $LauncherProcess
        Render = [IO.Path]::GetFullPath($renderDir); Engine = [IO.Path]::GetFullPath($engineConfigPath)
        Runtime = [IO.Path]::GetFullPath($engineRuntimePath); Backup = [IO.Path]::GetFullPath($engineBackupPath)
        DiagnosticPrefix = $DiagnosticPrefix
    }
    $script:journalPath = Join-Path $PSScriptRoot ".state\$GameId\active.json"
    Assert-JournalOrdinaryParents $script:journalPath
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $identity = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes((Get-NormalizedPath $renderDir))))).Replace('-', '') }
    finally { $sha.Dispose() }
    $script:deploymentMutex = [Threading.Mutex]::new($false, ('Global\DLSSG-Deployment-' + $identity))
    $acquired = $false
    try { $acquired = $script:deploymentMutex.WaitOne(0) }
    catch [Threading.AbandonedMutexException] { $acquired = $true }
    if (-not $acquired) { $script:deploymentMutex.Dispose(); $script:deploymentMutex = $null; throw 'Another deployment/recovery is running for this game.' }
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $identity = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes((Get-NormalizedPath $engineConfigPath))))).Replace('-', '') }
    finally { $sha.Dispose() }
    $script:configurationMutex = [Threading.Mutex]::new($false, ('Global\DLSSG-Config-' + $identity))
    $acquired = $false
    try { $acquired = $script:configurationMutex.WaitOne(0) }
    catch [Threading.AbandonedMutexException] { $acquired = $true }
    if (-not $acquired) {
        $script:configurationMutex.Dispose(); $script:configurationMutex = $null
        Close-DeploymentJournal
        throw 'Another deployment/recovery is using this configuration directory.'
    }
    $item = Get-JournalItem $script:journalPath
    if ($null -ne $item) {
        if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Invalid journal file.' }
        $script:deploymentJournal = [IO.File]::ReadAllText($script:journalPath) | ConvertFrom-Json
        Assert-DeploymentJournal
    }
}

function Close-DeploymentJournal {
    if ($null -ne $script:configurationMutex) {
        $script:configurationMutex.ReleaseMutex()
        $script:configurationMutex.Dispose()
        $script:configurationMutex = $null
    }
    if ($null -ne $script:deploymentMutex) {
        $script:deploymentMutex.ReleaseMutex()
        $script:deploymentMutex.Dispose()
        $script:deploymentMutex = $null
    }
}

function Assert-DeploymentJournal {
    $j = $script:deploymentJournal
    if ($j.Version -ne 1 -or $j.GameId -ne $script:journalContext.GameId) { throw 'Unsupported/corrupt deployment journal.' }
    foreach ($name in @('Render','Engine','Runtime','Backup')) {
        if ((Get-NormalizedPath $j.$name) -ne (Get-NormalizedPath $script:journalContext[$name])) {
            throw "Journal context changed ($name); use the original game/config directory."
        }
    }
    $expectedLinks = @($deployment.Specs) + @($linkSpecs[0])
    $allowed = @{}
    foreach ($spec in $expectedLinks) { $allowed[(Get-NormalizedPath $spec.Path)] = (Get-NormalizedPath $spec.Target) }
    $seen = @{}
    foreach ($op in @($j.Operations)) {
        if ($op.State -notin @('intent','done','restored')) { throw 'Invalid operation state.' }
        $key = Get-NormalizedPath $op.Path
        $operationKey = $op.Kind + ':' + $key
        if ($seen.ContainsKey($operationKey)) { throw 'Duplicate journal operation.' }; $seen[$operationKey] = $true
        switch ($op.Kind) {
            'backup' { if ($key -ne (Get-NormalizedPath $engineConfigPath) -or (Get-NormalizedPath $op.Target) -ne (Get-NormalizedPath $engineBackupPath) -or $op.Hash -notmatch '^[A-Fa-f0-9]{64}$') { throw 'Invalid backup operation.' } }
            'runtime' { if ($key -ne (Get-NormalizedPath $engineRuntimePath)) { throw 'Invalid runtime operation.' } }
            'diagnostic' {
                if ((Split-Path -Parent $key) -ne (Get-NormalizedPath $env:TEMP) -or
                    [IO.Path]::GetFileName($key) -notmatch ('^' + [regex]::Escape($script:journalContext.DiagnosticPrefix) + '\d+\.ini$')) { throw 'Invalid diagnostic operation.' }
            }
            'link' {
                $diagnosticLink = Get-NormalizedPath $linkSpecs[1].Path
                if ($key -eq $diagnosticLink) {
                    $tempKey = Get-NormalizedPath $op.Target
                    if ((Split-Path -Parent $tempKey) -ne (Get-NormalizedPath $env:TEMP) -or [IO.Path]::GetFileName($tempKey) -notmatch ('^' + [regex]::Escape($script:journalContext.DiagnosticPrefix) + '\d+\.ini$')) { throw 'Invalid diagnostic link.' }
                }
                elseif (-not $allowed.ContainsKey($key) -or $allowed[$key] -ne (Get-NormalizedPath $op.Target)) { throw 'Unexpected resource link in journal.' }
            }
            'engineLink' { if ($key -ne (Get-NormalizedPath $engineConfigPath) -or (Get-NormalizedPath $op.Target) -ne (Get-NormalizedPath $engineRuntimePath)) { throw 'Invalid engine link.' } }
            'storage' { if ($key -ne (Get-NormalizedPath $storageDll) -or $op.Hash -notmatch '^[A-Fa-f0-9]{64}$') { throw 'Invalid storage operation.' } }
            'directory' { if ($key -ne (Get-NormalizedPath $deployment.Root)) { throw 'Invalid resource directory operation.' } }
            default { throw 'Unknown journal operation.' }
        }
        Assert-JournalOrdinaryParents $op.Path
    }
}

function New-DeploymentJournal {
    if ($null -ne $script:deploymentJournal) { throw 'Unrecovered journal exists.' }
    $c = $script:journalContext
    $script:deploymentJournal = [pscustomobject]@{
        Version = 1; GameId = $c.GameId; Session = [guid]::NewGuid().ToString('N')
        ScriptPid = $PID; ScriptStarted = (Get-Process -Id $PID).StartTime.ToUniversalTime().ToString('o')
        Render = $c.Render; Engine = $c.Engine; Runtime = $c.Runtime; Backup = $c.Backup
        OriginalExisted = ($null -ne (Get-JournalItem $engineConfigPath)); OriginalHash = (Get-HashOrNull $engineConfigPath)
        StorageExisted = ($null -ne (Get-JournalItem $storageDll)); Phase = 'deploying'
        Operations = @(); Reused = @()
    }
    Save-DeploymentJournal
}

function Set-DeploymentPhase {
    param([string]$Phase)
    if ($null -ne $script:deploymentJournal) { $script:deploymentJournal.Phase = $Phase; Save-DeploymentJournal }
}

function Test-DeploymentInputs {
    # Reuse non-mutating conflict and format checks before recording/deploying any resource.
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try { $administrator = ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) }
    finally { $identity.Dispose() }
    if (-not $administrator) { throw '请通过管理员 BAT 启动；创建文件软链接需要管理员权限。' }
    if ($null -ne (Get-JournalItem $diagnosticIniPath)) { throw "临时诊断配置路径已存在，拒绝覆盖：$diagnosticIniPath" }
    $previousDryRun = $DryRun
    try {
        $DryRun = $true
        foreach ($spec in $linkSpecs) { [void](Assert-LinkOrAbsent -Path $spec.Path -ExpectedTarget $spec.Target) }
        Prepare-EngineConfig
    }
    finally { $DryRun = $previousDryRun }
}

function Get-JournalTextHash {
    param([string]$Text)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text)))).Replace('-', '') }
    finally { $sha.Dispose() }
}

function Resolve-LegacyEngineDeployment {
    $leftovers = @($engineRuntimePath, $engineBackupPath) | Where-Object { $null -ne (Get-JournalItem $_) }
    if (@($leftovers).Count -eq 0) { return }
    if ($DryRun) { throw 'DryRun: legacy Engine.ini deployment exists; restore it first.' }
    $answer = Read-Host '检测到旧版画面配置遗留，是否先恢复？[y/N]'
    if ($answer.Trim() -notmatch '^(?i:y|yes|是|同意)$') { throw '已取消，旧配置备份保持不变。' }
    Assert-RecoveryProcessesStopped
    Restore-EngineConfig
}

function Start-DeploymentOperation {
    param([string]$Kind, [string]$Path, [string]$Target = '', [string]$Hash = '')
    if ($null -eq $script:deploymentJournal) { throw 'Deployment journal not initialized.' }
    $op = [pscustomobject]@{ Kind = $Kind; Path = [IO.Path]::GetFullPath($Path); Target = $Target; Hash = $Hash; State = 'intent' }
    $script:deploymentJournal.Operations = @($script:deploymentJournal.Operations) + @($op)
    Save-DeploymentJournal
    return $op
}

function Complete-DeploymentOperation {
    param($Operation)
    $Operation.State = 'done'
    Save-DeploymentJournal
}

function Register-ReusedDeploymentLink {
    param([string]$Path, [string]$Target)
    if ($null -ne $script:deploymentJournal) {
        $script:deploymentJournal.Reused = @($script:deploymentJournal.Reused) + @([pscustomobject]@{ Path=$Path; Target=$Target })
        Save-DeploymentJournal
    }
}

function Assert-RecoveryProcessesStopped {
    $allProcesses = @(Get-Process -ErrorAction Stop)
    foreach ($name in @($script:journalContext.GameProcess, $script:journalContext.LauncherProcess)) {
        $running = @($allProcesses | Where-Object ProcessName -eq $name)
        if ($running.Count) { throw "Close $name before recovery." }
    }
}

function Restore-RecordedDeployment {
    param([switch]$EngineOnly, [switch]$Preview, [switch]$Automatic)
    if ($null -eq $script:deploymentJournal) { return }
    Assert-DeploymentJournal
    # Normal exit may leave the official launcher open. Explicit recovery requires both stopped.
    if ($Automatic) {
        if (@(Get-Process -ErrorAction Stop | Where-Object ProcessName -eq $script:journalContext.GameProcess).Count) { throw 'Game still running; deployment preserved.' }
    } else { Assert-RecoveryProcessesStopped }
    if (-not $Preview) { Set-DeploymentPhase 'recovering' }
    $ops = @($script:deploymentJournal.Operations)
    # Restore Engine.ini first; unrelated resource conflicts must not prevent original restoration.
    $ordered = @()
    foreach ($kind in @('engineLink','backup','runtime','storage','link','diagnostic','directory')) {
        $group = @($ops | Where-Object Kind -eq $kind)
        [array]::Reverse($group)
        $ordered += $group
    }
    $conflicts = [Collections.Generic.List[string]]::new()
    foreach ($op in $ordered) {
        if ($op.State -eq 'restored') { continue }
        if ($EngineOnly -and $op.Kind -notin @('backup','runtime','engineLink')) { continue }
        if ($Preview) { Write-Host "Recovery: $($op.Kind) $($op.Path)"; continue }
        try {
        if ($op.Kind -in @('backup','runtime')) {
            if (@($ops | Where-Object { $_.Kind -eq 'engineLink' -and $_.State -ne 'restored' }).Count) { throw 'Engine link unresolved; dependent files preserved.' }
            if ($op.Kind -eq 'runtime' -and @($ops | Where-Object { $_.Kind -eq 'backup' -and $_.State -ne 'restored' }).Count) { throw 'Backup unresolved; runtime preserved.' }
        }
        if ($op.Kind -eq 'diagnostic' -and @($ops | Where-Object { $_.Kind -eq 'link' -and $_.State -ne 'restored' -and (Get-NormalizedPath $_.Target) -eq (Get-NormalizedPath $op.Path) }).Count) { throw 'Diagnostic link unresolved; target preserved.' }
        $item = Get-JournalItem $op.Path
        switch ($op.Kind) {
            { $_ -in @('link','engineLink') } {
                if ($op.Kind -eq 'engineLink') {
                    $backupOperation = @($ops | Where-Object Kind -eq 'backup')
                    if ($backupOperation.Count -gt 0) {
                        $b = Get-JournalItem $backupOperation[0].Target
                        if ($null -eq $b -or $b.PSIsContainer -or ($b.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
                            (Get-HashOrNull $backupOperation[0].Target) -ne $backupOperation[0].Hash) {
                            # The engine link may already have been removed/restored before interruption.
                            if ($null -ne $item) { throw 'Backup missing/changed; Engine.ini preserved.' }
                        }
                    }
                }
                if ($null -ne $item) {
                    if ($item.LinkType -ne 'SymbolicLink' -or (Get-NormalizedPath ([string]($item.Target -join ''))) -ne (Get-NormalizedPath $op.Target)) { throw "Link changed; preserved: $($op.Path)" }
                    if ($item.PSIsContainer) { [IO.Directory]::Delete($op.Path) }
                    else { Remove-Item -LiteralPath $op.Path }
                }
            }
            'backup' {
                $backup = Get-JournalItem $op.Target
                if ($null -ne $backup) {
                    if ($backup.PSIsContainer -or ($backup.Attributes -band [IO.FileAttributes]::ReparsePoint) -or (Get-HashOrNull $op.Target) -ne $op.Hash) { throw "Backup changed; preserved: $($op.Target)" }
                    if ($null -ne $item) {
                        if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or (Get-HashOrNull $op.Path) -ne $op.Hash) { throw "Original path occupied: $($op.Path)" }
                        # Do not discard a second copy automatically.
                        throw "Both original and backup exist; preserved: $($op.Path)"
                    }
                    Move-Item -LiteralPath $op.Target -Destination $op.Path
                }
                elseif ($null -eq $item -or $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or (Get-HashOrNull $op.Path) -ne $op.Hash) { throw "Original/backup missing or changed: $($op.Path)" }
            }
            { $_ -in @('runtime','diagnostic','storage') } {
                if ($null -ne $item) {
                    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Temporary file replaced; preserved: $($op.Path)" }
                    if ($op.Kind -ne 'runtime' -and (Get-HashOrNull $op.Path) -ne $op.Hash) { throw "File hash changed; preserved: $($op.Path)" }
                    Remove-Item -LiteralPath $op.Path
                }
            }
            'directory' {
                if ($null -ne $item) {
                    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Resource directory changed; preserved: $($op.Path)" }
                    # Keep game logs or unrelated contents; never recursively delete the resource root.
                    if (@(Get-ChildItem -LiteralPath $op.Path -Force -ErrorAction Stop).Count -eq 0) { [IO.Directory]::Delete($op.Path) }
                }
            }
        }
        }
        catch { $conflicts.Add($_.Exception.Message); Write-Warning $_; continue }
        $op.State = 'restored'
        Save-DeploymentJournal
    }
    if ($conflicts.Count) { throw ('Recovery incomplete; journal retained: ' + ($conflicts -join '; ')) }
    if (-not $Preview -and @($ops | Where-Object State -ne 'restored').Count -eq 0) {
        [IO.File]::Delete($script:journalPath)
        $script:deploymentJournal = $null
        Write-Host 'Deployment recovery complete.'
    }
}

function Resolve-PreviousDeployment {
    if ($null -eq $script:deploymentJournal) { return }
    if ($DryRun) { Restore-RecordedDeployment -Preview; throw 'DryRun: unfinished deployment exists; recovery is required before deployment.' }
    $answer = Read-Host '检测到未完成部署，是否恢复后继续？[y/N]'
    if ($answer.Trim() -notmatch '^(?i:y|yes|是|同意)$') { throw '已取消，部署记录与现场保持不变。' }
    Restore-RecordedDeployment
}
