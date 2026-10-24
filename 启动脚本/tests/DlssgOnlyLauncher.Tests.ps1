# Isolated discovery/model/worker/recovery tests. Never starts an installed game.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path -Parent $PSScriptRoot
. (Join-Path $root 'scripts\common\LauncherConfig.ps1')
. (Join-Path $root 'scripts\common\SteamLauncher.ps1')
. (Join-Path $root 'scripts\common\DlssgOnlyLauncher.ps1')
function Assert-Test($Condition,[string]$Message) { if (-not $Condition) { throw $Message } }
function Expect-Failure([scriptblock]$Action) { $failed=$false; try { & $Action } catch { $failed=$true }; Assert-Test $failed 'Expected rejection.' }
$testRoot=Join-Path $env:TEMP ('dlssg-only-tests-'+[guid]::NewGuid().ToString('N'))
$project=Join-Path $testRoot 'project'; $fixture=Join-Path $project '启动脚本'
function Write-Fixture([string]$Path,[string]$Text='fixture') {
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    [IO.File]::WriteAllText($Path,$Text,[Text.UTF8Encoding]::new($true))
}
function Write-Exe([string]$Path,[int]$Machine=0x8664) {
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    $bytes=[byte[]]::new(512); $bytes[0]=0x4D; $bytes[1]=0x5A
    [BitConverter]::GetBytes([int]128).CopyTo($bytes,60)
    [BitConverter]::GetBytes([int]0x4550).CopyTo($bytes,128)
    [BitConverter]::GetBytes([uint16]$Machine).CopyTo($bytes,132)
    [BitConverter]::GetBytes([uint16]2).CopyTo($bytes,150)
    [IO.File]::WriteAllBytes($Path,$bytes)
}
$resources=@('version.dll','alternatives\winmm.dll','alternatives\dbghelp.dll','alternatives\dinput8.dll','alternatives\dxgi.dll','alternatives\d3d12.dll','dlssg_sm86.ini',
    'dlssg_sm86\runtime\nvngx_dlssg.dll','dlssg_sm86\backend\sm86_backend.dll','dlssg_sm86\kernels\fg_kernels.dll','dlssg_sm86\xefg\xefg_kernels.dll','dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll')
foreach ($r in $resources) { Write-Fixture (Join-Path $project $r) }
[void][IO.Directory]::CreateDirectory((Join-Path $fixture 'scripts\games'))
[void][IO.Directory]::CreateDirectory((Join-Path $fixture 'scripts\common'))
Copy-Item -LiteralPath (Join-Path $root 'scripts\games\Run-SteamUE5-DLSSG.ps1') -Destination (Join-Path $fixture 'scripts\games')
Get-ChildItem -LiteralPath (Join-Path $root 'scripts\common') -Filter '*.ps1' | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $fixture 'scripts\common') }
$steam=Join-Path $testRoot 'Steam\steam.exe'; Write-Exe $steam
$apps=Join-Path (Split-Path -Parent $steam) 'steamapps'
$game=Join-Path $apps 'common\测试游戏'; $render=Join-Path $game 'Project\Binaries\Win64\Game-Win64-Shipping.exe'
Write-Exe $render; Write-Exe (Join-Path $game 'Start.exe'); Write-Exe (Join-Path $game 'CrashReportClient.exe')
Write-Exe (Join-Path $game 'GameLauncher.exe'); Write-Exe (Join-Path $game 'Observer.exe'); Write-Exe (Join-Path $game 'x86.exe') 0x14c
Write-Fixture (Join-Path $game 'tools\Compiler.exe')
Write-Fixture (Join-Path (Split-Path -Parent $render) 'nvngx_dlssg.dll')
Write-Fixture (Join-Path $apps 'appmanifest_123.acf') "`"AppState`"`n{`n`"appid`" `"123`"`n`"name`" `"测试游戏`"`n`"installdir`" `"测试游戏`"`n}"
$issues=[Collections.Generic.List[string]]::new()
$candidates=@(Get-GameRendererCandidates $game (Join-Path $game 'Start.exe') -Issues $issues)
Assert-Test ($candidates[0].Path -eq $render) 'Nested renderer did not outrank platform bootstrap.'
Assert-Test (@($candidates | Where-Object Relative -match 'CrashReport|Launcher|x86|Compiler').Count -eq 0) 'Tool/x86 included as renderer.'
Assert-Test (@($candidates | Where-Object Relative -eq 'Observer.exe').Count -eq 1) 'Game title falsely classified as server.'
$unity=Join-Path $testRoot 'Unity'; Write-Exe (Join-Path $unity 'UnityGame.exe'); Write-Fixture (Join-Path $unity 'UnityPlayer.dll')
[void][IO.Directory]::CreateDirectory((Join-Path $unity 'UnityGame_Data'))
$unityRows=@(Get-GameRendererCandidates $unity)
Assert-Test ($unityRows[0].Score -ge 60) 'Unity pairing not recognized.'
$cts=[Threading.CancellationTokenSource]::new(); $cts.Cancel()
Assert-Test (@(Get-GameRendererCandidates $game -Cancellation $cts).Count -eq 0) 'Cancelled scan returned candidates.'
$cts.Dispose()
$outside=Join-Path $testRoot 'Outside'; Write-Exe (Join-Path $outside 'Outside.exe')
$junction=Join-Path $game 'LinkedOutside'
New-Item -ItemType Junction -Path $junction -Target $outside | Out-Null
try {
    $linkedRows=@(Get-GameRendererCandidates $game)
    Assert-Test (@($linkedRows | Where-Object Relative -match 'Outside').Count -eq 0) 'Scanner traversed directory link.'
    Expect-Failure { Get-GameRendererCandidates $junction }
} finally { [IO.Directory]::Delete($junction) }
Assert-Test ([IO.File]::Exists((Join-Path $outside 'Outside.exe'))) 'Junction cleanup touched target.'
Write-Host 'PASS: x64 PE, nested renderer, bootstrap separation, helper exclusion, Unity and cancellation.'

$epic=Join-Path $testRoot 'EpicManifests'; $epicGame=Join-Path $testRoot 'EpicGame'; Write-Exe (Join-Path $epicGame 'Game.exe')
Write-Fixture (Join-Path $epic 'game.item') (@{AppName='Game';MainGameAppName='Game';DisplayName='Epic测试';InstallLocation=$epicGame;LaunchExecutable='Game.exe'}|ConvertTo-Json)
Write-Fixture (Join-Path $epic 'dlc.item') (@{AppName='DLC';MainGameAppName='Game';InstallLocation=$epicGame}|ConvertTo-Json)
Write-Fixture (Join-Path $epic 'broken.item') '{broken'
Write-Fixture (Join-Path $epic 'escape.item') (@{AppName='Escape';InstallLocation=$epicGame;LaunchExecutable='..\outside.exe'}|ConvertTo-Json)
$gogGame=Join-Path $testRoot 'GogGame'; Write-Exe (Join-Path $gogGame 'Gog.exe')
Write-Fixture (Join-Path $gogGame 'goggame-42.info') (@{gameId='42';rootGameId='42';name='GOG测试';playTasks=@(@{isPrimary=$true;type='FileTask';path='Gog.exe'})}|ConvertTo-Json -Depth 5)
$gogKeys=Join-Path $testRoot 'GogKeys'; Write-Fixture (Join-Path $gogKeys '42.key')
function Get-ItemProperty { param($LiteralPath,$ErrorAction) if ($LiteralPath -like '*42.key') { return [pscustomobject]@{path=$gogGame;gameName='GOG'} }; Microsoft.PowerShell.Management\Get-ItemProperty @PSBoundParameters }
$records=@(Get-InstalledGameRecords -SteamExecutable $steam -EpicManifestDirectory $epic -GogRegistryRoots @($gogKeys) -Issues $issues)
Assert-Test ($records.Count -eq 3) "Expected three platforms, got $($records.Count)"
Assert-Test (@($records | Where-Object Platform -eq 'Steam').Count -eq 1) 'Steam missing.'
Assert-Test (@($records | Where-Object Platform -eq 'Epic').Count -eq 1) 'Epic DLC/escape not excluded.'
Assert-Test (($records | Where-Object Platform -eq 'GOG').DeclaredExecutable -eq (Join-Path $gogGame 'Gog.exe')) 'GOG primary task missing.'
Assert-Test ($issues.Count -ge 2) 'Malformed/unsafe records not reported.'
Remove-Item Function:\Get-ItemProperty
Write-Host 'PASS: Steam, Epic, GOG local records; DLC, malformed JSON and path escape rejection.'

$record=[pscustomobject]@{Platform='Epic';Name="游戏 O'Brien";GameDirectory=$game;AppId='';DeclaredExecutable=(Join-Path $game 'Start.exe')}
$profile=New-DlssgOnlyProfile $record 'Project\Binaries\Win64\Game-Win64-Shipping.exe' 'Start.exe'
$model=Get-DlssgOnlyModel $profile '' -Root $fixture
Assert-Test $model.Ready $model.Status
Assert-Test ($profile.Id -match '^direct-[a-f0-9]{32}$') 'Missing stable direct identity.'
$request=Write-DlssgOnlyLaunchConfig $model (Join-Path $testRoot 'request')
Assert-Test (-not ([IO.File]::ReadAllText($request) -match 'Engine|配置源|游戏配置目录')) 'Request contains Engine.ini settings.'
$settings=Get-DirectUESettings -GameKey $profile.Id.Substring(7) -Path $request -Overrides @{DlssgOnly=$true}
Assert-Test (-not $settings.ContainsKey('EngineConfigSource') -and $settings.RenderExecutable -eq $render) 'Direct settings require Engine.ini.'
$steamRecord=$records | Where-Object Platform -eq 'Steam'
$steamProfile=New-DlssgOnlyProfile $steamRecord $profile.RenderExecutable 'Start.exe'
$steamModel=Get-DlssgOnlyModel $steamProfile $steam -Root $fixture
Assert-Test $steamModel.Ready $steamModel.Status
$steamRequest=Write-DlssgOnlyLaunchConfig $steamModel (Join-Path $testRoot 'steam-request')
$steamSettings=Get-SteamUE5Settings -AppId 123 -Path $steamRequest -Overrides @{DlssgOnly=$true}
Assert-Test (-not $steamSettings.ContainsKey('ConfigDirectory')) 'Steam settings require config directory.'
$prefs=@{SteamPath='';Profiles=@($profile,$steamProfile)}; $prefsDirectory=Join-Path $testRoot 'preferences'
Save-PortablePreferences $prefs -Directory $prefsDirectory
$read=Read-PortablePreferences -Directory $prefsDirectory
Assert-Test ($read.Profiles.Count -eq 2 -and $read.Profiles[0].Id -eq $profile.Id) 'Saved identity or array merge changed.'
Assert-Test ((Get-DlssgOnlyDataDirectory) -ne (Get-PortableDataDirectory)) 'Preferences overlap original launcher.'
Write-Host 'PASS: independent preferences, stable IDs, Steam/EXE requests without Engine.ini or directory requirements.'

$worker=Join-Path $fixture 'scripts\games\Run-SteamUE5-DLSSG.ps1'
$engine=Join-Path $game 'Engine.ini'
foreach ($state in @('absent','ordinary','readonly')) {
    if ($state -ne 'absent') { Write-Fixture $engine 'original Engine bytes' }
    if ($state -eq 'readonly') { [IO.File]::SetAttributes($engine,[IO.FileAttributes]::ReadOnly) }
    $before=if ([IO.File]::Exists($engine)) { (Get-FileHash -LiteralPath $engine).Hash } else { '' }
    $log=Join-Path $testRoot "dry-$state.txt"
    & powershell.exe -NoProfile -File $worker -DlssgOnly -LaunchMode Direct -GameKey $profile.Id.Substring(7) -LauncherConfigPath $request -Profile desktop -DryRun *> $log
    Assert-Test ($LASTEXITCODE -eq 0) "DryRun failed: $log"
    if ($before) { Assert-Test ((Get-FileHash -LiteralPath $engine).Hash -eq $before) 'Engine bytes changed.'; [IO.File]::SetAttributes($engine,[IO.FileAttributes]::Normal); [IO.File]::Delete($engine) }
    else { Assert-Test (-not [IO.File]::Exists($engine)) 'Absent Engine created.' }
}
$log=Join-Path $testRoot 'steam-dry.txt'
& powershell.exe -NoProfile -File $worker -DlssgOnly -AppId 123 -LauncherConfigPath $steamRequest -Profile desktop -DryRun *> $log
Assert-Test ($LASTEXITCODE -eq 0) "Steam DryRun failed: $log"
Write-Host 'PASS: real isolated worker DryRun for Steam and EXE; absent, ordinary and read-only Engine.ini unchanged.'

. (Join-Path $root 'scripts\common\LauncherResources.ps1')
. (Join-Path $root 'scripts\common\DeploymentJournal.ps1')
function Get-NormalizedPath([string]$Path) { [IO.Path]::GetFullPath($Path).TrimEnd('\').ToLowerInvariant() }
function Get-HashOrNull([string]$Path) { if ([IO.File]::Exists($Path)) { (Get-FileHash -LiteralPath $Path).Hash } }
$renderDir=Split-Path -Parent $render
$engineConfigPath=$null; $engineRuntimePath=$null; $engineBackupPath=$null
$diagnosticIniPath=Join-Path $env:TEMP ("dlssg_sm86-$($profile.Id)-$PID.ini")
$linkSpecs=@(@{},@{Path=(Join-Path $renderDir 'dlssg_sm86.ini');Target=$diagnosticIniPath})
Select-DlssgProxy $project $renderDir 'version.dll'
$script:journalContext=@{GameId=$profile.Id;GameProcess='NoSuchDlssgGame';LauncherProcess=@();Render=$renderDir;Engine='';Runtime='';Backup='';DlssgOnly=$true;DiagnosticPrefix="dlssg_sm86-$($profile.Id)-";SelectableProxy=$true}
$script:journalPath=Join-Path $testRoot 'journal\active.json'
$DryRun=$false
New-DeploymentJournal
Assert-Test ($script:deploymentJournal.DeploymentMode -eq 'DlssgOnly' -and -not $script:deploymentJournal.Engine) 'Resource journal contains config.'
Expect-Failure { Start-DeploymentOperation runtime (Join-Path $testRoot 'forbidden.ini') }
Write-Fixture $diagnosticIniPath 'diagnostic'
$op=Start-DeploymentOperation diagnostic $diagnosticIniPath '' (Get-HashOrNull $diagnosticIniPath); Complete-DeploymentOperation $op
$uncreated=Start-DeploymentOperation link $linkSpecs[0].Path $linkSpecs[0].Target
Restore-RecordedDeployment
Assert-Test (-not [IO.File]::Exists($diagnosticIniPath) -and -not [IO.File]::Exists($script:journalPath)) 'Partial resource deployment did not recover.'
Restore-RecordedDeployment
New-DeploymentJournal
$script:deploymentJournal.DeploymentMode='EngineConfig'
Expect-Failure { Assert-DeploymentJournal }
$script:deploymentJournal.DeploymentMode='DlssgOnly'
Expect-Failure { Restore-RecordedDeployment -EngineOnly }
Restore-RecordedDeployment
Write-Host 'PASS: resource-only journal, forbidden engine operations, interrupted intent, repeat recovery and mode mismatch.'

New-DeploymentJournal
$op=Start-DeploymentOperation storage $storageDll '' (Get-HashOrNull $linkSpecs[0].Target)
Complete-DeploymentOperation $op
Write-Fixture $storageDll 'foreign DLL'
Expect-Failure { Restore-RecordedDeployment }
Assert-Test ([IO.File]::Exists($storageDll) -and [IO.File]::Exists($script:journalPath)) 'New mode discarded storage conflict.'
Copy-Item -LiteralPath $linkSpecs[0].Target -Destination $storageDll -Force
Restore-RecordedDeployment
Assert-Test (-not [IO.File]::Exists($storageDll)) 'Owned storage not recovered after retry.'
Write-Host 'PASS: only-DLSSG storage conflict preserves file and journal, then supports retry.'

$stateDir=Join-Path $fixture ('.state\'+$profile.Id)
Write-Fixture (Join-Path $stateDir 'active.json') (@{Version=1;GameId=$profile.Id;Render=$renderDir;DeploymentMode='EngineConfig'}|ConvertTo-Json)
$blocked=Get-DlssgOnlyModel $profile '' -Root $fixture
Assert-Test (-not $blocked.Ready -and -not $blocked.Recovery -and $blocked.Status -match '原启动器') 'Old deployment exposed to new recovery.'
[IO.File]::Delete((Join-Path $stateDir 'active.json'))

# A different profile ID must not bypass an older deployment in the same directory.
$otherState=Join-Path $fixture '.state\steam-999\active.json'
Write-Fixture $otherState (@{Version=1;GameId='steam-999';Render=$renderDir;Engine='old-config-path'}|ConvertTo-Json)
$before=(Get-FileHash -LiteralPath $otherState).Hash
$blocked=Get-DlssgOnlyModel $profile '' -Root $fixture
Assert-Test (-not $blocked.Ready -and $blocked.Status -match '原入口') 'New profile bypassed original deployment in GUI.'
$log=Join-Path $testRoot 'other-profile.txt'
try {
    $ErrorActionPreference='Continue'
    & powershell.exe -NoProfile -File $worker -DlssgOnly -LaunchMode Direct -GameKey $profile.Id.Substring(7) -LauncherConfigPath $request -Profile desktop -DryRun *> $log
} finally { $ErrorActionPreference='Stop' }
Assert-Test ($LASTEXITCODE -ne 0 -and (Get-FileHash -LiteralPath $otherState).Hash -eq $before) 'Worker bypassed or changed original profile record.'
[IO.File]::Delete($otherState)

# Only the render mutex is held, including across modes and processes.
. (Join-Path $fixture 'scripts\common\DeploymentJournal.ps1')
try {
    Open-DeploymentJournal -GameId $profile.Id -GameProcess NoSuchDlssgGame -LauncherProcess @() -DiagnosticPrefix "dlssg_sm86-$($profile.Id)-" -GameExecutable $render -SelectableProxy -DlssgOnly
    Assert-Test ($null -eq $script:configurationMutex) 'Only-DLSSG mode locked a nonexistent config path.'
    $log=Join-Path $testRoot 'mutex.txt'
    try {
        $ErrorActionPreference='Continue'
        & powershell.exe -NoProfile -File $worker -DlssgOnly -LaunchMode Direct -GameKey $profile.Id.Substring(7) -LauncherConfigPath $request -Profile desktop -DryRun *> $log
    } finally { $ErrorActionPreference='Stop' }
    Assert-Test ($LASTEXITCODE -ne 0 -and ([IO.File]::ReadAllText($log) -match 'Another deployment/recovery')) 'Cross-process rendering mutex failed.'
} finally { Close-DeploymentJournal }
Write-Host 'PASS: another profile cannot bypass retained deployment; rendering mutex without config mutex.'

# Cross-process handoff executes an inert worker, including the mandatory mode flag.
$inert=Join-Path $testRoot "worker O'Brien.ps1"
Write-Fixture $inert @'
param($LauncherConfigPath,$Profile,$LaunchMode,$GameKey,[switch]$DryRun,[switch]$DlssgOnly)
if (-not $DlssgOnly -or -not $DryRun -or $LaunchMode -ne 'Direct' -or $GameKey -notmatch '^[a-f0-9]{32}$') { throw 'Handoff lost mode/identity.' }
exit 0
'@
$resultPath=Join-Path $testRoot 'handoff.json'
$command=New-PortableWorkerCommand -ScriptPath $inert -ConfigPath $request -ResultPath $resultPath -Action Check -LaunchMode Direct -GameKey $profile.Id.Substring(7) -DlssgOnly
$encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
& powershell.exe -NoProfile -EncodedCommand $encoded
Assert-Test ($LASTEXITCODE -eq 0 -and ([IO.File]::ReadAllText($resultPath)|ConvertFrom-Json).ExitCode -eq 0) 'Encoded worker handoff failed.'
Write-Host "PASS: old mode blocked and encoded mode handoff. Artifacts: $testRoot"

# Exercise actual asynchronous UI scan function without a desktop dialog.
$tokens=$null; $errors=$null
$ui=Join-Path $root 'scripts\ui\Show-DlssgLauncher.ps1'
$ast=[Management.Automation.Language.Parser]::ParseFile($ui,[ref]$tokens,[ref]$errors)
Assert-Test ($errors.Count -eq 0) 'New UI syntax errors.'
foreach ($name in @('Start-Scan','Start-Action')) {
    $node=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true)
    . ([scriptblock]::Create($node.Extent.Text))
}
function Set-Working([bool]$Working) { $script:busy=$Working }
$steamBox=[pscustomobject]@{Text=''}; $cancel=[pscustomobject]@{Enabled=$false}; $status=[pscustomobject]@{Text=''}
$script:pendingRecord=$record
Start-Scan 'Renderers'
$scanResult=@($script:scan.EndInvoke($script:scanHandle))
Assert-Test (-not $script:scan.HadErrors) ($script:scan.Streams.Error | Out-String)
Assert-Test ($scanResult[0].Rows[0].Path -eq $render) 'Actual background scan lost renderer ranking.'
$script:scan.Dispose(); $script:scan=$null; $script:cancellation.Dispose(); $script:cancellation=$null
Write-Host 'PASS: actual UI background runspace returns renderer suggestions.'

# Exercise the actual Start-Action function; UAC, windows and process creation are mocked.
function Refresh-Game {}
function Save-Editors { $script:saved=$true }
function Show-Failure([string]$Message) { $script:failure=$Message }
function Start-Process {
    param($FilePath,$ArgumentList,$PassThru,$Verb,$WindowStyle)
    if ($script:cancelElevation) { throw 'UAC cancelled' }
    $script:observedVerb=$Verb
    $script:workerCommand=[Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($ArgumentList[-1]))
    $process=[pscustomobject]@{Disposed=$false;Killed=$false}
    $process | Add-Member ScriptMethod Dispose { $this.Disposed=$true }
    $process | Add-Member ScriptMethod Kill { $this.Killed=$true; throw 'Worker cannot be killed.' }
    $script:createdWorker=$process
    return $process
}
$dataDirectory=Join-Path $testRoot 'ui-requests'
$games=[pscustomobject]@{SelectedItem=$profile}
foreach ($action in @('Launch','Check','Restore','CancelledLaunch')) {
    $script:model=$model; $script:model.Recovery=$true
    $script:worker=$null; $script:failure=''; $script:busy=$false; $script:saved=$false
    $script:cancelElevation=$action -eq 'CancelledLaunch'
    $form=[pscustomobject]@{Hidden=$false;Closed=$false}
    $form | Add-Member ScriptMethod Hide { $this.Hidden=$true }
    $form | Add-Member ScriptMethod Close { $this.Closed=$true }
    Start-Action $(if ($script:cancelElevation) { 'Launch' } else { $action })
    if ($script:cancelElevation) { Assert-Test (-not $form.Hidden -and -not $form.Closed -and $script:failure -match 'cancelled') 'Cancelled UAC lost UI.' }
    elseif ($action -eq 'Launch') {
        Assert-Test ($form.Hidden -and $form.Closed -and $script:createdWorker.Disposed -and -not $script:createdWorker.Killed) 'Launch did not detach UI safely.'
        Assert-Test ($script:workerCommand -match '-DlssgOnly' -and $script:observedVerb -eq 'RunAs') 'Launch lost mode/elevation.'
    } else { Assert-Test (-not $form.Hidden -and -not $form.Closed -and $script:busy -and $null -ne $script:worker) 'Check/restore closed UI.' }
}
Write-Host 'PASS: actual new UI handoff, cancelled elevation, mode flag and retained check/recovery UI.'
