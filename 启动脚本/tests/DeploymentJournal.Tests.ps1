# Isolated filesystem recovery tests. Does not launch games or touch installed game directories.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$launcherRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $launcherRoot 'scripts\common\DeploymentJournal.ps1')
function Get-NormalizedPath { param([string]$Path) [IO.Path]::GetFullPath($Path).TrimEnd('\').ToLowerInvariant() }
function Get-HashOrNull { param([string]$Path) if ([IO.File]::Exists($Path)) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash } }
function Assert-Test { param([bool]$Condition,[string]$Message) if (-not $Condition) { throw $Message } }
function Expect-Failure { param([scriptblock]$Action) $failed=$false; try { & $Action } catch { $failed=$true }; Assert-Test $failed 'Expected failure was not raised.' }
$testRoot = Join-Path $env:TEMP ('dlssg-journal-tests-' + [guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testRoot)
$renderDir = Join-Path $testRoot 'render'
[void][IO.Directory]::CreateDirectory($renderDir)
$engineConfigPath = Join-Path $testRoot 'Engine.ini'
$engineRuntimePath = Join-Path $testRoot 'Engine.runtime.ini'
$engineBackupPath = Join-Path $testRoot 'Engine.backup.ini'
$storageDll = Join-Path $renderDir 'storage.dll'
$diagnosticIniPath = Join-Path $env:TEMP ('dlssg-journal-test-' + $PID + '.ini')
$linkSpecs = @(@{Path=(Join-Path $renderDir 'proxy.dll');Target=(Join-Path $testRoot 'source.dll')}, @{Path=(Join-Path $renderDir 'dlssg_sm86.ini');Target=$diagnosticIniPath})
$deployment = @{ Root=(Join-Path $renderDir 'dlssg_sm86'); Specs=@(@{Path=(Join-Path $renderDir 'resource');Target=(Join-Path $testRoot 'resource-source')}) }
[IO.File]::WriteAllText($linkSpecs[0].Target,'proxy')
$DryRun = $false
function Start-TestJournal {
    $script:journalPath = Join-Path $testRoot 'state\active.json'
    $script:journalContext = @{GameId='test';GameProcess='DLSSG_NoSuchGame';LauncherProcess='DLSSG_NoSuchLauncher';Render=$renderDir;Engine=$engineConfigPath;Runtime=$engineRuntimePath;Backup=$engineBackupPath;DiagnosticPrefix='dlssg-journal-test-'}
    $script:deploymentJournal=$null
    New-DeploymentJournal
}
function Reload-TestJournal { $script:deploymentJournal = [IO.File]::ReadAllText($script:journalPath) | ConvertFrom-Json }
function Add-TestEngine {
    param([switch]$NoOriginal, [switch]$BeforeMove, [switch]$AfterRestore)
    $runtime = Start-DeploymentOperation runtime $engineRuntimePath
    [IO.File]::WriteAllText($engineRuntimePath,'game rewrote temporary settings')
    Complete-DeploymentOperation $runtime
    if (-not $NoOriginal) {
        [IO.File]::WriteAllText($engineConfigPath,'original')
        $backup = Start-DeploymentOperation backup $engineConfigPath $engineBackupPath (Get-HashOrNull $engineConfigPath)
        if (-not $BeforeMove) {
            Move-Item -LiteralPath $engineConfigPath -Destination $engineBackupPath
            if ($AfterRestore) { Move-Item -LiteralPath $engineBackupPath -Destination $engineConfigPath }
        }
    }
}
try {
    foreach ($file in @('scripts\common\DeploymentJournal.ps1','scripts\common\LauncherResources.ps1','scripts\games\Run-WutheringWaves-DLSSG.ps1','scripts\games\Run-SteamUE5-DLSSG.ps1','scripts\games\Run-NevernessToEverness-DLSSG.ps1')) {
        $tokens=$null; $errors=$null
        [void][Management.Automation.Language.Parser]::ParseFile((Join-Path $launcherRoot $file),[ref]$tokens,[ref]$errors)
        Assert-Test ($errors.Count -eq 0) "Syntax errors: $file"
    }
    foreach ($phase in @('before-move','after-move','after-restore','no-original')) {
        Start-TestJournal
        Add-TestEngine -BeforeMove:($phase -eq 'before-move') -AfterRestore:($phase -eq 'after-restore') -NoOriginal:($phase -eq 'no-original')
        if ($phase -in @('after-move','no-original')) { [void](Start-DeploymentOperation engineLink $engineConfigPath $engineRuntimePath) }
        Reload-TestJournal
        Restore-RecordedDeployment
        Assert-Test (-not (Test-Path -LiteralPath $engineRuntimePath)) "Runtime survived $phase"
        Assert-Test (-not (Test-Path -LiteralPath $script:journalPath)) "Journal survived $phase"
        if ($phase -ne 'no-original') { Assert-Test ([IO.File]::ReadAllText($engineConfigPath) -eq 'original') 'Original content changed.'; Remove-Item -LiteralPath $engineConfigPath }
        Restore-RecordedDeployment
    }
    Write-Host 'PASS: recovery before/after rename, after restore, missing original, failed link creation, reload and repeated recovery.'

    Start-TestJournal; Add-TestEngine
    [IO.File]::WriteAllText($diagnosticIniPath,'diagnostic')
    $op=Start-DeploymentOperation diagnostic $diagnosticIniPath '' (Get-JournalTextHash 'diagnostic'); Complete-DeploymentOperation $op
    $before=(Get-FileHash -LiteralPath $script:journalPath).Hash
    Restore-RecordedDeployment -Preview
    Assert-Test ((Get-FileHash -LiteralPath $script:journalPath).Hash -eq $before) 'Preview changed journal.'
    Restore-RecordedDeployment -EngineOnly
    Assert-Test (Test-Path -LiteralPath $diagnosticIniPath) 'Engine-only removed unrelated file.'
    Assert-Test (Test-Path -LiteralPath $script:journalPath) 'Engine-only discarded deployment state.'
    Reload-TestJournal; Restore-RecordedDeployment
    Assert-Test (-not (Test-Path -LiteralPath $diagnosticIniPath)) 'Diagnostic not removed.'
    Remove-Item -LiteralPath $engineConfigPath
    Write-Host 'PASS: preview, engine-only recovery followed by full recovery.'

    Start-TestJournal; Add-TestEngine
    [IO.File]::WriteAllText($engineBackupPath,'changed')
    Expect-Failure { Restore-RecordedDeployment }
    Assert-Test (Test-Path -LiteralPath $engineBackupPath) 'Changed backup lost.'
    Assert-Test (Test-Path -LiteralPath $engineRuntimePath) 'Runtime lost on backup conflict.'
    [IO.File]::WriteAllText($engineBackupPath,'original'); Reload-TestJournal; Restore-RecordedDeployment
    Remove-Item -LiteralPath $engineConfigPath
    Write-Host 'PASS: changed backup retained; retry succeeds after conflict resolution.'

    foreach ($originalExists in @($true, $false)) {
        Start-TestJournal; Add-TestEngine -NoOriginal:(-not $originalExists)
        [IO.File]::SetAttributes($engineRuntimePath, [IO.FileAttributes]::ReadOnly)
        $op=Start-DeploymentOperation engineLink $engineConfigPath $engineRuntimePath
        Complete-DeploymentOperation $op
        [IO.File]::WriteAllText($engineConfigPath,'game replaced the read-only target link')
        $savedPath=Join-Path (Split-Path -Parent $script:journalPath) ('recovered\' + $script:deploymentJournal.Session + '-Engine.ini')
        Reload-TestJournal; Restore-RecordedDeployment
        Assert-Test ([IO.File]::ReadAllText($savedPath) -eq 'game replaced the read-only target link') 'Replacement contents lost.'
        Assert-Test (-not (Test-Path -LiteralPath $engineRuntimePath)) 'Read-only runtime survived replacement recovery.'
        if ($originalExists) {
            Assert-Test ([IO.File]::ReadAllText($engineConfigPath) -eq 'original') 'Original not restored after link replacement.'
            Remove-Item -LiteralPath $engineConfigPath
        }
        else { Assert-Test (-not (Test-Path -LiteralPath $engineConfigPath)) 'Originally absent config not restored to absence.' }
    }
    Start-TestJournal; Add-TestEngine
    $op=Start-DeploymentOperation engineLink $engineConfigPath $engineRuntimePath
    Complete-DeploymentOperation $op
    [IO.File]::WriteAllText($engineConfigPath,'replacement')
    $savedPath=Join-Path (Split-Path -Parent $script:journalPath) ('recovered\' + $script:deploymentJournal.Session + '-Engine.ini')
    [IO.File]::WriteAllText($savedPath,'existing saved replacement')
    Reload-TestJournal; Expect-Failure { Restore-RecordedDeployment }
    Assert-Test ([IO.File]::ReadAllText($engineConfigPath) -eq 'replacement') 'Collision overwrote replacement.'
    Assert-Test ([IO.File]::ReadAllText($savedPath) -eq 'existing saved replacement') 'Collision overwrote saved replacement.'
    Remove-Item -LiteralPath $savedPath
    Reload-TestJournal; Restore-RecordedDeployment
    Remove-Item -LiteralPath $engineConfigPath
    Write-Host 'PASS: replaced Engine.ini archived, original restored, read-only cleanup, no-original and archive collision recovery.'

    Start-TestJournal
    $op=Start-DeploymentOperation link $linkSpecs[0].Path $linkSpecs[0].Target
    [IO.File]::WriteAllText($linkSpecs[0].Path,'unrelated replacement')
    Expect-Failure { Restore-RecordedDeployment }
    Assert-Test ([IO.File]::ReadAllText($linkSpecs[0].Path) -eq 'unrelated replacement') 'Replacement overwritten.'
    Remove-Item -LiteralPath $linkSpecs[0].Path; Reload-TestJournal; Restore-RecordedDeployment
    Write-Host 'PASS: unexpected ordinary file preserved.'

    Start-TestJournal
    [void](Start-DeploymentOperation storage $storageDll '' (Get-JournalTextHash 'proxy'))
    [IO.File]::WriteAllText($storageDll,'other dll')
    Expect-Failure { Restore-RecordedDeployment }
    Assert-Test (Test-Path -LiteralPath $storageDll) 'Unrecognized storage DLL deleted.'
    [IO.File]::WriteAllText($storageDll,'proxy'); Reload-TestJournal; Restore-RecordedDeployment
    Write-Host 'PASS: storage hash ownership.'

    Start-TestJournal
    [void](Start-DeploymentOperation runtime (Join-Path $testRoot 'unexpected.ini'))
    Expect-Failure { Restore-RecordedDeployment }
    Assert-Test (Test-Path -LiteralPath $script:journalPath) 'Invalid journal deleted.'
    $script:deploymentJournal=$null; Remove-Item -LiteralPath $script:journalPath
    Start-TestJournal
    $script:deploymentJournal.Version=999; Save-DeploymentJournal; Reload-TestJournal
    Expect-Failure { Restore-RecordedDeployment }
    $script:deploymentJournal=$null; Remove-Item -LiteralPath $script:journalPath
    Write-Host 'PASS: path allowlist and unsupported journal version.'

    Start-TestJournal
    Register-ReusedDeploymentLink $linkSpecs[0].Path $linkSpecs[0].Target
    [IO.File]::WriteAllText($linkSpecs[0].Path,'preexisting')
    Reload-TestJournal; Expect-Failure { Restore-RecordedDeployment }
    Assert-Test (Test-Path -LiteralPath $linkSpecs[0].Path) 'Unrecognized replacement removed.'
    Remove-Item -LiteralPath $linkSpecs[0].Path
    Reload-TestJournal; Restore-RecordedDeployment
    Start-TestJournal
    $script:journalContext.GameProcess=(Get-Process -Id $PID).ProcessName
    Expect-Failure { Restore-RecordedDeployment }
    Assert-Test (Test-Path -LiteralPath $script:journalPath) 'Running process lost journal.'
    $script:journalContext.GameProcess='DLSSG_NoSuchGame'; Restore-RecordedDeployment
    Write-Host 'PASS: reused link replacement preserved and running process blocks recovery.'

    $source=Join-Path $launcherRoot '配置\异环\Engine.ini'

    # Real directory junctions work without file-symlink elevation on typical Windows installations.
    $target=$deployment.Specs[0].Target; [void][IO.Directory]::CreateDirectory($target)
    [IO.File]::WriteAllText((Join-Path $target 'sentinel.txt'),'preserve')
    Start-TestJournal
    $op=Start-DeploymentOperation link $deployment.Specs[0].Path $target
    New-Item -ItemType Junction -Path $deployment.Specs[0].Path -Target $target | Out-Null
    # Junction is deliberately not accepted as our SymbolicLink; it must be preserved.
    Expect-Failure { Restore-RecordedDeployment }
    Assert-Test (Test-Path -LiteralPath (Join-Path $target 'sentinel.txt')) 'Link target contents lost.'
    [IO.Directory]::Delete($deployment.Specs[0].Path)
    Reload-TestJournal; Restore-RecordedDeployment
    Write-Host 'PASS: directory junction replacement preserved; target never traversed.'

    # Failure to persist an intent must prevent the corresponding filesystem mutation.
    Start-TestJournal
    $saveImplementation = (Get-Command Save-DeploymentJournal).ScriptBlock
    function Save-DeploymentJournal { throw 'injected persistence failure' }
    try {
        Expect-Failure {
            [void](Start-DeploymentOperation runtime $engineRuntimePath)
            [IO.File]::WriteAllText($engineRuntimePath,'must not be written')
        }
        Assert-Test (-not (Test-Path -LiteralPath $engineRuntimePath)) 'Mutation proceeded after failed journal save.'
    }
    finally { Set-Item Function:\Save-DeploymentJournal $saveImplementation }
    Reload-TestJournal; Restore-RecordedDeployment
    Write-Host 'PASS: journal write failure stops filesystem mutation.'

    # Exercise each launcher's actual Prepare-EngineConfig, not a duplicate implementation.
    foreach ($launcher in @('scripts\games\Run-WutheringWaves-DLSSG.ps1','scripts\games\Run-SteamUE5-DLSSG.ps1','scripts\games\Run-NevernessToEverness-DLSSG.ps1')) {
        $tokens=$null; $errors=$null
        $ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $launcherRoot $launcher),[ref]$tokens,[ref]$errors)
        foreach ($name in @('Assert-FileExists','Write-Log','Prepare-EngineConfig')) {
            $node=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true)
            Invoke-Expression $node.Extent.Text
        }
        $engineConfigSource=$source
        $engineRuntimeMarker='; isolated test'
        $enginePrepared=$false; $engineOriginalMoved=$false; $engineRuntimeCreated=$false
        [IO.File]::WriteAllText($engineConfigPath,'original opaque configuration')
        $originalHash=Get-HashOrNull $engineConfigPath
        Start-TestJournal
        function New-Item { throw 'injected SymbolicLink creation failure' }
        try { Expect-Failure { Prepare-EngineConfig } }
        finally { Remove-Item Function:\New-Item }
        Assert-Test (-not (Test-Path -LiteralPath $engineConfigPath)) 'Original not renamed before injected link failure.'
        Assert-Test ($script:deploymentJournal.Operations.Count -eq 3) 'Launcher did not journal each Engine.ini operation.'
        Assert-Test (([IO.File]::GetAttributes($engineRuntimePath) -band [IO.FileAttributes]::ReadOnly) -ne 0) 'Launcher runtime is writable.'
        Reload-TestJournal; Restore-RecordedDeployment
        Assert-Test ((Get-HashOrNull $engineConfigPath) -eq $originalHash) 'Actual launcher rollback changed original.'
        Remove-Item -LiteralPath $engineConfigPath
    }
    Write-Host 'PASS: all three actual launcher Engine.ini preparation functions recover after injected link failure.'

    # Positive file-link path using simulated link metadata when this process is not elevated.
    # Backup movement, runtime writes, journal saves/reloads and hash checks remain real filesystem operations.
    Start-TestJournal; Add-TestEngine
    $virtualLinks=@{}
    $virtualLinks[$engineConfigPath]=$engineRuntimePath
    [void](Start-DeploymentOperation engineLink $engineConfigPath $engineRuntimePath)
    $getItemImplementation=(Get-Command Get-JournalItem).ScriptBlock
    function Get-JournalItem {
        param([string]$Path)
        if ($virtualLinks.ContainsKey($Path)) { return [pscustomobject]@{ LinkType='SymbolicLink';Target=@($virtualLinks[$Path]);PSIsContainer=$false;Attributes=[IO.FileAttributes]::ReparsePoint } }
        & $getItemImplementation $Path
    }
    function Remove-Item {
        param([string]$LiteralPath, [switch]$Force)
        if ($virtualLinks.ContainsKey($LiteralPath)) { $virtualLinks.Remove($LiteralPath); return }
        Microsoft.PowerShell.Management\Remove-Item -LiteralPath $LiteralPath -Force:$Force
    }
    try {
        Reload-TestJournal; Restore-RecordedDeployment
        Assert-Test ($virtualLinks.Count -eq 0) 'Owned link not removed.'
        Assert-Test ([IO.File]::ReadAllText($engineConfigPath) -eq 'original') 'Linked runtime restoration failed.'
        Remove-Item -LiteralPath $engineConfigPath

        # Use the actual NTE preparation function loaded above; original bytes need no decoding.
        [IO.File]::WriteAllText($engineConfigPath,'opaque encrypted-looking original')
        $originalHash=Get-HashOrNull $engineConfigPath
        $sourceHash=Get-HashOrNull $source
        $DryRun=$true
        Prepare-EngineConfig
        Assert-Test ((Get-HashOrNull $engineConfigPath) -eq $originalHash) 'DryRun changed original.'
        Assert-Test (-not (Test-Path -LiteralPath $engineRuntimePath)) 'DryRun wrote runtime.'
        $DryRun=$false
        Start-TestJournal
        function New-Item {
            param([string]$ItemType,[string]$Path,[string]$Target)
            Assert-Test ($ItemType -eq 'SymbolicLink') 'Unexpected item type.'
            $virtualLinks[$Path]=$Target
        }
        try { Prepare-EngineConfig } finally { Microsoft.PowerShell.Management\Remove-Item Function:\New-Item }
        Assert-Test ([IO.File]::ReadAllText($engineRuntimePath) -ceq ($engineRuntimeMarker + "`r`n" + [IO.File]::ReadAllText($source))) 'NTE runtime is not the unmodified plaintext source with marker.'
        Assert-Test ((Get-HashOrNull $engineBackupPath) -eq $originalHash) 'Opaque original backup changed.'
        Assert-Test (([IO.File]::GetAttributes($engineRuntimePath) -band [IO.FileAttributes]::ReadOnly) -ne 0) 'NTE runtime is writable.'
        Expect-Failure { [IO.File]::WriteAllText($engineRuntimePath,'game attempted save') }
        Reload-TestJournal; Restore-RecordedDeployment
        Assert-Test ((Get-HashOrNull $engineConfigPath) -eq $originalHash) 'Opaque original not restored byte-for-byte.'
        Assert-Test ((Get-HashOrNull $source) -eq $sourceHash) 'Source changed during deployment.'
        Remove-Item -LiteralPath $engineConfigPath
        Write-Host 'PASS: actual NTE plaintext deployment, DryRun, read-only write rejection, opaque original backup and byte-for-byte restoration.'

        Start-TestJournal
        $virtualLinks[$linkSpecs[0].Path]=$linkSpecs[0].Target
        # Old journal format: ownership used to be recorded only in Reused.
        $script:deploymentJournal.Reused=@([pscustomobject]@{Path=$linkSpecs[0].Path;Target=$linkSpecs[0].Target})
        Save-DeploymentJournal; Reload-TestJournal
        Restore-RecordedDeployment -Preview
        Assert-Test ($virtualLinks.Count -eq 1) 'Preview removed old reused link.'
        Restore-RecordedDeployment -EngineOnly
        Assert-Test (Test-Path -LiteralPath $script:journalPath) 'Engine-only discarded old reused journal.'
        Restore-RecordedDeployment
        Assert-Test ($virtualLinks.Count -eq 0) 'Old reused link survived recovery.'

        $virtualLinks[$linkSpecs[0].Path]=$linkSpecs[0].Target
        $virtualLinks[$deployment.Specs[0].Path]=$deployment.Specs[0].Target
        Restore-LegacyResourceDeployment -Preview
        Assert-Test ($virtualLinks.Count -eq 2) 'Legacy preview removed links.'
        Restore-LegacyResourceDeployment

        Start-TestJournal
        $virtualLinks[$linkSpecs[0].Path]=$linkSpecs[0].Target
        Reload-TestJournal; Restore-RecordedDeployment
        Assert-Test ($virtualLinks.Count -eq 0) 'Unregistered owned link survived partial setup recovery.'
        Assert-Test ($virtualLinks.Count -eq 0) 'Unjournaled legacy links survived.'
        Assert-Test (-not (Test-Path -LiteralPath $script:journalPath)) 'Legacy recovery retained completed journal.'
        Restore-LegacyResourceDeployment

        Start-TestJournal
        [IO.File]::WriteAllText($linkSpecs[0].Path,'unrecorded foreign file')
        Expect-Failure { Restore-RecordedDeployment }
        Assert-Test (Test-Path -LiteralPath $script:journalPath) 'Residue verification discarded journal.'
        Remove-Item -LiteralPath $linkSpecs[0].Path
        Reload-TestJournal; Restore-RecordedDeployment

        Start-TestJournal
        $script:deploymentJournal.Reused=@([pscustomobject]@{Path=(Join-Path $testRoot 'foreign');Target=$linkSpecs[0].Target})
        Save-DeploymentJournal
        Expect-Failure { Restore-RecordedDeployment }
        $script:deploymentJournal.Reused=@(); Save-DeploymentJournal; Restore-RecordedDeployment
    }
    finally {
        Set-Item Function:\Get-JournalItem $getItemImplementation
        Microsoft.PowerShell.Management\Remove-Item Function:\Remove-Item
    }
    Write-Host 'PASS: simulated links, old Reused journal migration, unjournaled legacy recovery, preview, residue detection and reused path validation.'

    [IO.File]::WriteAllText($storageDll,'proxy')
    Restore-LegacyResourceDeployment
    Assert-Test (-not (Test-Path -LiteralPath $storageDll)) 'Legacy owned storage survived.'
    [IO.File]::WriteAllText($storageDll,'foreign')
    Restore-LegacyResourceDeployment
    Assert-Test (Test-Path -LiteralPath $storageDll) 'Foreign legacy storage deleted.'
    Remove-Item -LiteralPath $storageDll
    Write-Host 'PASS: legacy storage hash ownership and unrelated DLL preservation.'

    $windowsIdentity=[Security.Principal.WindowsIdentity]::GetCurrent()
    try { $elevated=([Security.Principal.WindowsPrincipal]::new($windowsIdentity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) }
    finally { $windowsIdentity.Dispose() }
    if ($elevated) {
        Start-TestJournal; Add-TestEngine
        $op=Start-DeploymentOperation engineLink $engineConfigPath $engineRuntimePath
        New-Item -ItemType SymbolicLink -Path $engineConfigPath -Target $engineRuntimePath | Out-Null
        Complete-DeploymentOperation $op
        [IO.File]::WriteAllText($engineConfigPath,'game save through link')
        Reload-TestJournal; Restore-RecordedDeployment
        Assert-Test ([IO.File]::ReadAllText($engineConfigPath) -eq 'original') 'Native file-link restoration failed.'
        Remove-Item -LiteralPath $engineConfigPath
        Write-Host 'PASS: native file SymbolicLink, game-save simulation and recovery.'
    }
    else { Write-Host 'SKIP: native file SymbolicLink requires elevated PowerShell; simulated link and failure recovery tested.' }

    # Real separate process verifies mutex exclusion (same-process mutexes are reentrant).
    Open-DeploymentJournal -GameId test -GameProcess DLSSG_NoSuchGame -LauncherProcess DLSSG_NoSuchLauncher -DiagnosticPrefix dlssg-journal-test-
    try {
        $childFile=Join-Path $testRoot 'mutex-test.ps1'
        $moduleLiteral=(Join-Path $launcherRoot 'scripts\common\DeploymentJournal.ps1').Replace("'","''")
        $renderLiteral=$renderDir.Replace("'","''")
        $configLiteral=$engineConfigPath.Replace("'","''")
        $runtimeLiteral=$engineRuntimePath.Replace("'","''")
        $backupLiteral=$engineBackupPath.Replace("'","''")
        $child=@"
`$ErrorActionPreference='Stop'
. '$moduleLiteral'
function Get-NormalizedPath { param([string]`$Path) [IO.Path]::GetFullPath(`$Path).TrimEnd('\').ToLowerInvariant() }
`$renderDir='$renderLiteral'
`$engineConfigPath='$configLiteral'
`$engineRuntimePath='$runtimeLiteral'
`$engineBackupPath='$backupLiteral'
try { Open-DeploymentJournal -GameId test -GameProcess DLSSG_NoSuchGame -LauncherProcess DLSSG_NoSuchLauncher -DiagnosticPrefix dlssg-journal-test-; exit 2 }
catch { if (`$_.Exception.Message -notmatch 'Another deployment/recovery') { throw }; exit 0 }
finally { Close-DeploymentJournal }
"@
        [IO.File]::WriteAllText($childFile,$child,[Text.UTF8Encoding]::new($true))
        & powershell -NoProfile -ExecutionPolicy Bypass -File $childFile
        Assert-Test ($LASTEXITCODE -eq 0) 'Concurrent deployment not excluded.'
        # A different game render directory sharing the same Engine.ini must also be excluded.
        $child=$child.Replace("`$renderDir='$renderLiteral'", "`$renderDir='$renderLiteral-other'")
        [IO.File]::WriteAllText($childFile,$child,[Text.UTF8Encoding]::new($true))
        & powershell -NoProfile -ExecutionPolicy Bypass -File $childFile
        Assert-Test ($LASTEXITCODE -eq 0) 'Shared configuration directory not locked.'
    }
    finally { Close-DeploymentJournal }
    Write-Host 'PASS: cross-process game and configuration locks.'

    Start-TestJournal
    $op=Start-DeploymentOperation directory $deployment.Root
    [void][IO.Directory]::CreateDirectory($deployment.Root); Complete-DeploymentOperation $op
    Reload-TestJournal; Restore-RecordedDeployment
    Assert-Test (-not (Test-Path -LiteralPath $deployment.Root)) 'New empty resource directory survived.'
    Start-TestJournal
    $op=Start-DeploymentOperation directory $deployment.Root
    [void][IO.Directory]::CreateDirectory($deployment.Root); Complete-DeploymentOperation $op
    $sentinel=Join-Path $deployment.Root 'game-log.txt'; [IO.File]::WriteAllText($sentinel,'preserve logs')
    Reload-TestJournal; Restore-RecordedDeployment
    Assert-Test ([IO.File]::ReadAllText($sentinel) -eq 'preserve logs') 'Resource directory contents lost.'
    Write-Host 'PASS: new empty resource root removed, nonempty root and logs preserved.'

    Write-Host "All isolated checks passed. Artifacts: $testRoot"
}
finally {
    if ([IO.File]::Exists($diagnosticIniPath)) { [IO.File]::Delete($diagnosticIniPath) }
}
