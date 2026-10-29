# Isolated discovery and actual UI runspace tests. Never deploys or starts a game.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path -Parent $PSScriptRoot
. (Join-Path $root 'scripts\common\LauncherConfig.ps1')
. (Join-Path $root 'scripts\common\SteamLauncher.ps1')
. (Join-Path $root 'scripts\common\PortableLauncher.ps1')
function Assert-Test($Condition,[string]$Message) { if (-not $Condition) { throw $Message } }
function Write-Fixture([string]$Path,[string]$Text='fixture') {
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    [IO.File]::WriteAllText($Path,$Text,[Text.UTF8Encoding]::new($true))
}
function Write-Exe([string]$Path) {
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    $bytes=[byte[]]::new(512); $bytes[0]=0x4D; $bytes[1]=0x5A
    [BitConverter]::GetBytes([int]128).CopyTo($bytes,60)
    [BitConverter]::GetBytes([int]0x4550).CopyTo($bytes,128)
    [BitConverter]::GetBytes([uint16]0x8664).CopyTo($bytes,132)
    [BitConverter]::GetBytes([uint16]2).CopyTo($bytes,150)
    [IO.File]::WriteAllBytes($Path,$bytes)
}
$testRoot=Join-Path $env:TEMP ('unreal-discovery-'+[guid]::NewGuid().ToString('N'))
$game=Join-Path $testRoot '游戏安装'; $local=Join-Path $testRoot 'Local'; $documents=Join-Path $testRoot 'Documents'
$render=Join-Path $game 'Project\Binaries\Win64\Other-Win64-Shipping.exe'
Write-Exe $render
$issues=[Collections.Generic.List[string]]::new()
$args=@{GameDirectory=$game;RenderExecutable=$render;LocalAppData=$local;Documents=$documents;Issues=$issues}
Assert-Test (@(Get-UnrealConfigDirectoryCandidates @args).Count -eq 0) 'Empty scan should return no directories.'
$config=Join-Path $local 'Project\Saved\Config\WindowsNoEditor'
Write-Fixture (Join-Path $config 'GameUserSettings.ini')
$rows=@(Get-UnrealConfigDirectoryCandidates @args)
Assert-Test ($rows.Count -eq 1 -and $rows[0].Path -eq $config -and $rows[0].Reasons -match 'GameUserSettings') 'Project name differing from EXE or settings-only directory missed.'
$second=Join-Path $documents 'My Games\Other\Saved\Config\WindowsClient'
Write-Fixture (Join-Path $second 'Engine.ini')
$third=Join-Path $game 'Saved\Config\Windows'; Write-Fixture (Join-Path $third 'Engine.ini')
$fourth=Join-Path $game 'Project\Saved\Config\Windows'; Write-Fixture (Join-Path $fourth 'Engine.ini')
Assert-Test (@(Get-UnrealConfigDirectoryCandidates @args).Count -eq 4) 'Multiple locations were discarded.'
$dedup=@{GameDirectory=$game;RenderExecutable=$render;LocalAppData=$game;Documents='';Issues=$issues}
Assert-Test (@(Get-UnrealConfigDirectoryCandidates @dedup).Count -eq 2) 'Equivalent project/install candidates were not deduplicated.'
$cts=[Threading.CancellationTokenSource]::new(); $cts.Cancel()
Assert-Test (@(Get-UnrealConfigDirectoryCandidates @args -Cancellation $cts).Count -eq 0) 'Cancellation ignored.'
$cts.Dispose()
$outside=Join-Path $testRoot 'Outside'; Write-Fixture (Join-Path $outside 'Saved\Config\Windows\Engine.ini')
$link=Join-Path $local 'Other'
New-Item -ItemType Junction -Path $link -Target $outside | Out-Null
try {
    $rows=@(Get-UnrealConfigDirectoryCandidates @args)
    Assert-Test ($rows.Count -eq 4 -and $issues.Count -gt 0) 'Scanner traversed linked ancestor or failed to report it.'
} finally { [IO.Directory]::Delete($link) }
Assert-Test ([IO.File]::Exists((Join-Path $outside 'Saved\Config\Windows\Engine.ini'))) 'Link test changed target.'
# Deterministically simulate denied enumeration without changing machine ACLs.
function Get-ChildItem {
    param($LiteralPath,[switch]$Force,$ErrorAction)
    if ($LiteralPath -eq $config) { throw [UnauthorizedAccessException]::new('fixture denied') }
    Microsoft.PowerShell.Management\Get-ChildItem @PSBoundParameters
}
try {
    $rows=@(Get-UnrealConfigDirectoryCandidates @args)
    Assert-Test ($rows.Count -eq 3 -and @($issues | Where-Object { $_ -match 'fixture denied' }).Count) 'Access failure did not preserve other results and details.'
} finally { Remove-Item Function:\Get-ChildItem }
Write-Host 'PASS: exact config discovery, all roots/platforms, settings-only, deduplication, cancellation, junction and access failure.'

$best=[pscustomobject]@{Score=60;Path=$render}
$weak=[pscustomobject]@{Score=30;Path='bootstrap'}
Assert-Test ((Get-TrustedGameRenderer @($best,$weak)).Path -eq $render) 'Unique evidenced winner not selected.'
Assert-Test ($null -eq (Get-TrustedGameRenderer @($best,$best))) 'Tied candidates guessed.'
Assert-Test ($null -eq (Get-TrustedGameRenderer @($weak))) 'Weak candidate guessed.'
$bootstrap=Join-Path $game 'Start.exe'; Write-Exe $bootstrap
$record=[pscustomobject]@{Platform='Epic';Name='示例';GameDirectory=$game;AppId='';DeclaredExecutable=$bootstrap}
$profile=New-PortableDiscoveredProfile $record $render
Assert-Test ($profile.LaunchMode -eq 'Direct' -and $profile.LaunchExecutable -eq 'Start.exe' -and $profile.Id -match '^direct-[a-f0-9]{32}$') 'Direct identity/bootstrap lost.'
$profile.ConfigDirectory=$config
$preferences=@{SteamPath='';Profiles=@()}; $prefsDir=Join-Path $testRoot 'preferences'
Save-PortableProfileChange $preferences $profile -Directory $prefsDir
$reopened=Read-PortablePreferences $prefsDir
Assert-Test ($reopened.Profiles.Count -eq 1 -and $reopened.Profiles[0].Id -eq $profile.Id -and $reopened.Profiles[0].ConfigDirectory -eq $config) 'Direct roundtrip changed identity/config.'
$record.Platform='Steam'; $record.AppId='123'
$steamProfile=New-PortableDiscoveredProfile $record $render
$steamProfile.ConfigDirectory=$config
Save-PortableProfileChange $preferences $steamProfile -Directory $prefsDir
$reopened=Read-PortablePreferences $prefsDir
Assert-Test ($reopened.Profiles.Count -eq 2 -and ($reopened.Profiles | Where-Object Id -eq 'custom-123').LaunchMode -eq 'Steam') 'Steam roundtrip failed.'
$record.Platform='GOG'; $record.DeclaredExecutable=Join-Path $game 'Missing.exe'
Assert-Test ((New-PortableDiscoveredProfile $record $render).LaunchExecutable -eq $profile.RenderExecutable) 'Missing platform entry did not fall back to renderer.'
$rejected=$false
try { New-PortableDiscoveredProfile $record (Join-Path $outside 'Outside.exe') } catch { $rejected=$true }
Assert-Test $rejected 'Out-of-root renderer accepted.'
Write-Host 'PASS: renderer confidence, Steam/direct profiles, bootstrap fallback, persistence and stable identity.'

$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $root 'scripts\ui\Show-SteamLauncher.ps1'),[ref]$tokens,[ref]$errors)
Assert-Test ($errors.Count -eq 0) ($errors | Out-String)
foreach ($name in @('Start-Discovery','Complete-Discovery')) {
    $node=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true)
    . ([scriptblock]::Create($node.Extent.Text))
}
function Set-Working([bool]$Working) { $script:busy=$Working }
$steamBox=[pscustomobject]@{Text=''}; $cancelScan=[pscustomobject]@{Enabled=$false}; $script:status=[pscustomobject]@{Text=''}
$script:pendingRecord=$record; $script:pendingProfile=$profile; $script:preferences=$preferences
$originalLocal=$env:LOCALAPPDATA
try {
    $env:LOCALAPPDATA=$local
    foreach ($mode in @('Renderers','Config')) {
        Start-Discovery $mode
        $result=@($script:discovery.EndInvoke($script:discoveryHandle))
        Assert-Test (-not $script:discovery.HadErrors) ($script:discovery.Streams.Error | Out-String)
        Assert-Test ($result.Count -eq 1 -and @($result[0].Rows).Count -gt 0) 'UI runspace did not return candidates.'
        $script:discovery.Dispose(); $script:discovery=$null; $script:cancellation.Dispose(); $script:cancellation=$null
    }
} finally { $env:LOCALAPPDATA=$originalLocal }
# Actual completion function: only ambiguous/empty results invoke selection, cancelled selection never saves.
function Select-DiscoveryRecord($Rows,$Title) { $script:selections++; return $null }
function Select-Folder($Description) { $script:selections++; return '' }
function Save-PortableProfileChange($Preferences,$Profile) { $script:saves++; $script:savedProfile=$Profile }
function Load-Games($SelectedId) {}
$script:selections=0; $script:saves=0; $script:repairConfig=$false
Complete-Discovery Config @([pscustomobject]@{Path=$config})
Assert-Test ($script:saves -eq 1 -and $script:selections -eq 0) 'Unique config prompted or was not saved.'
Complete-Discovery Config @([pscustomobject]@{Path=$config},[pscustomobject]@{Path=$second})
Complete-Discovery Config @()
Assert-Test ($script:saves -eq 1 -and $script:selections -eq 2) 'Cancelled ambiguous/missing selections saved a partial profile.'
function Get-PortableGameModel($Profile,$SteamPath) { return [pscustomobject]@{Recovery=$true} }
$script:repairConfig=$true; $rejected=$false
try { Complete-Discovery Config @([pscustomobject]@{Path=$second}) } catch { $rejected=$true }
Assert-Test ($rejected -and $script:saves -eq 1) 'Recovery failed to block config redirect.'
Write-Host "PASS: actual UI runspaces, unique/multiple/missing config completion and recovery lockout. Artifacts: $testRoot"

# Exercise the actual timer callback: cancellation after worker completion still wins.
$tickNode=$ast.Find({param($n) $n -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $n.Member.Value -eq 'Add_Tick'},$true)
$tickText=$tickNode.Arguments[0].ScriptBlock.Extent.Text
$tick=[scriptblock]::Create($tickText.Substring(1,$tickText.Length-2))
$script:discovery=[powershell]::Create()
[void]$script:discovery.AddScript('[pscustomobject]@{Rows=@();Issues=@();Cancelled=$false}')
$script:discoveryHandle=$script:discovery.BeginInvoke()
Assert-Test ($script:discoveryHandle.AsyncWaitHandle.WaitOne(5000)) 'Inert scan did not finish.'
$script:cancellation=[Threading.CancellationTokenSource]::new(); $script:cancellation.Cancel()
$script:discoveryMode='Config'
& $tick
Assert-Test ($null -eq $script:discovery -and $null -eq $script:cancellation -and $null -eq $script:pendingProfile -and $script:saves -eq 1 -and $script:status.Text -match '已取消') 'Late cancellation saved settings or leaked runspace.'

# Verify the shared dependency is exported for Unreal without generating any packages.
$exportTokens=$null; $exportErrors=$null
$exportAst=[Management.Automation.Language.Parser]::ParseFile((Join-Path $root 'scripts\tools\Export-PortableLauncher.ps1'),[ref]$exportTokens,[ref]$exportErrors)
Assert-Test ($exportErrors.Count -eq 0) 'Exporter syntax error.'
$sharedBlock=$exportAst.Find({param($n) $n -is [Management.Automation.Language.IfStatementAst] -and $n.Clauses[0].Item1.Extent.Text -eq '$includeLauncher'},$true)
Assert-Test ($null -ne $sharedBlock -and $sharedBlock.Extent.Text -match "'GameDiscovery'") 'Unreal portable dependency omitted.'
Write-Host 'PASS: actual timer late cancellation and shared export whitelist (no package generated).'
