# Isolated direct launch checks: no installed game, Steam, UAC or preferences changed.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path -Parent $PSScriptRoot
. (Join-Path $root 'scripts\common\LauncherConfig.ps1')
. (Join-Path $root 'scripts\common\SteamLauncher.ps1')
. (Join-Path $root 'scripts\common\PortableLauncher.ps1')
function Assert-Test {param([bool]$Condition,[string]$Message) if(-not $Condition){throw $Message}}
function Expect-Failure {param([scriptblock]$Action) $failed=$false;try{& $Action}catch{$failed=$true};Assert-Test $failed 'Expected rejection.'}
$testRoot=Join-Path $env:TEMP ('dlssg-direct-'+[guid]::NewGuid().ToString('N'))
$project=Join-Path $testRoot "中文 O'Brien [Direct]"
$fixture=Join-Path $project '启动脚本'
$game=Join-Path $testRoot 'NonSteam Game'
$config=Join-Path $testRoot 'User Config'
$render=Join-Path $game 'Example\Binaries\Win64\Example-Win64-Shipping.exe'
$launch=Join-Path $game 'Launch.exe'
foreach($dir in @($fixture,$game,$config,(Split-Path -Parent $render),(Join-Path $fixture 'presets'),(Join-Path $fixture '配置\SteamUE5'),(Join-Path $fixture 'scripts\games'),(Join-Path $fixture 'scripts\common'))){[void][IO.Directory]::CreateDirectory($dir)}
[IO.File]::WriteAllText($render,'inert');[IO.File]::WriteAllText($launch,'inert')
[IO.File]::WriteAllText((Join-Path $fixture '配置\SteamUE5\Engine.ini'),'[SystemSettings]')
[IO.File]::WriteAllText((Join-Path $config 'Engine.ini'),"original config`r`n")
[IO.File]::WriteAllText((Join-Path $fixture 'presets\SteamUE5.json'),'[]')
foreach($rel in @('version.dll','dlssg_sm86.ini','dlssg_sm86\runtime\nvngx_dlssg.dll','dlssg_sm86\backend\sm86_backend.dll','dlssg_sm86\kernels\fg_kernels.dll','dlssg_sm86\xefg\xefg_kernels.dll','dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll')){
    $file=Join-Path $project $rel;[void][IO.Directory]::CreateDirectory((Split-Path -Parent $file));[IO.File]::WriteAllText($file,'inert')
}
function Resolve-SteamAppId {throw 'Direct launch touched Steam discovery.'}
function Find-SteamGameDirectory {throw 'Direct launch touched Steam libraries.'}
function Get-PortableSteamPath {throw 'Direct launch required Steam.'}
$key=[guid]::NewGuid().ToString('N')
$profile=[pscustomobject]@{Id="direct-$key";Name='直接测试';AppId='';LaunchMode='Direct';LaunchExecutable='Launch.exe';RenderExecutable='Example\Binaries\Win64\Example-Win64-Shipping.exe';GameDirectory=$game;ConfigDirectory=$config;EngineConfigSource='配置\SteamUE5\Engine.ini';ProxyDll=1;QualityPreset='Current';EngineOverride='Auto'}
$model=Get-PortableGameModel $profile '' -Root $fixture
Assert-Test ($model.Ready -and $model.GameId -eq $profile.Id -and $model.LaunchExecutable -eq $launch) 'Direct model needs Steam or misidentified executable.'
$request=Write-PortableLaunchConfig $model (Join-Path $testRoot 'request')
$settings=Get-DirectUESettings -GameKey $key -Path $request
Assert-Test ($settings.GameId -eq $profile.Id -and $settings.LauncherPath -eq $launch -and $settings.RenderExecutable -eq $render) 'Direct INI roundtrip failed.'
Expect-Failure {Resolve-DirectExecutable $game '..\outside.exe'}
Expect-Failure {Resolve-DirectExecutable $game 'C:\outside.exe'}
Expect-Failure {Resolve-DirectExecutable $game 'bad.dll'}
$profile.LaunchExecutable='missing.exe'
Assert-Test (-not (Get-PortableGameModel $profile '' -Root $fixture).Ready) 'Missing launch EXE accepted.'
$profile.LaunchExecutable='Launch.exe'
$preferences=@{SteamPath='';Profiles=@($profile)}
Save-PortablePreferences $preferences -Directory (Join-Path $testRoot 'prefs')
$reloaded=Read-PortablePreferences -Directory (Join-Path $testRoot 'prefs')
$profiles=@(Get-PortableProfiles $reloaded -Root $fixture)
Assert-Test ($profiles.Count -eq 1 -and $profiles[0].LaunchMode -eq 'Direct' -and $profiles[0].Id -eq $profile.Id) 'Direct identity changed on reopen.'
$state=Join-Path $fixture ('.state\'+$profile.Id)
[void][IO.Directory]::CreateDirectory($state)
[IO.File]::WriteAllText((Join-Path $state 'active.json'),(@{Version=1;GameId=$profile.Id;Render=(Split-Path -Parent $render);Engine=(Join-Path $config 'Engine.ini')}|ConvertTo-Json))
$profile.GameDirectory='Z:\Missing';$profile.ConfigDirectory='Z:\MissingConfig';$profile.QualityPreset='Tier2'
$recovery=Get-PortableGameModel $profile '' -Root $fixture
Assert-Test ($recovery.Recovery -and $recovery.GameDirectory -eq $game -and $recovery.ConfigDirectory -eq $config) 'Recovery followed changed preferences.'
$restoreRequest=Write-PortableLaunchConfig $recovery (Join-Path $testRoot 'restore-request')
Assert-Test ((Get-DirectUESettings -GameKey $key -Path $restoreRequest).GameDirectory -eq $game) 'Direct restore needs Steam.'
Remove-Item -LiteralPath (Join-Path $state 'active.json')
$profile.GameDirectory=$game;$profile.ConfigDirectory=$config;$profile.QualityPreset='Current'
function Start-Process {param($FilePath,$WorkingDirectory,$ArgumentList,$WindowStyle) $script:dispatch=$PSBoundParameters}
Start-ConfiguredUEGame Direct $launch
Assert-Test ($script:dispatch.FilePath -eq $launch -and $script:dispatch.WorkingDirectory -eq $game -and -not $script:dispatch.ContainsKey('ArgumentList')) 'Direct dispatch used Steam arguments or wrong directory.'
Start-ConfiguredUEGame Steam 'C:\Steam\steam.exe' 123
Assert-Test ($script:dispatch.ArgumentList[0] -eq '-applaunch') 'Steam dispatch changed.'
Copy-Item -LiteralPath (Join-Path $root 'scripts\games\Run-SteamUE5-DLSSG.ps1') -Destination (Join-Path $fixture 'scripts\games')
Get-ChildItem -LiteralPath (Join-Path $root 'scripts\common') -Filter '*.ps1' | ForEach-Object {Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $fixture 'scripts\common')}
$before=(Get-FileHash -LiteralPath (Join-Path $config 'Engine.ini')).Hash
$dryLog=Join-Path $testRoot 'dryrun-output.txt'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $fixture 'scripts\games\Run-SteamUE5-DLSSG.ps1') -LaunchMode Direct -GameKey $key -Profile desktop -LauncherConfigPath $request -DryRun *> $dryLog
Assert-Test ($LASTEXITCODE -eq 0) "Direct worker DryRun failed, see $dryLog"
Assert-Test ((Get-FileHash -LiteralPath (Join-Path $config 'Engine.ini')).Hash -eq $before -and -not [IO.File]::Exists((Join-Path $state 'active.json'))) 'DryRun mutated config/journal.'
Assert-Test (@(Get-ChildItem -LiteralPath (Split-Path -Parent $render) -Filter '*.dll').Count -eq 0) 'DryRun deployed DLLs.'
$fakeWorker=Join-Path $testRoot "worker O'Brien.ps1"
[IO.File]::WriteAllText($fakeWorker,@'
param($LauncherConfigPath,$Profile,$LaunchMode,$GameKey,[switch]$DryRun)
if($LaunchMode -ne 'Direct' -or $GameKey -notmatch '^[a-f0-9]{32}$' -or -not $DryRun -or -not [IO.File]::Exists($LauncherConfigPath)){throw 'Direct parameters lost.'}
exit 0
'@)
$resultPath=Join-Path $testRoot 'result.json'
$command=New-PortableWorkerCommand -ScriptPath $fakeWorker -ConfigPath $request -ResultPath $resultPath -Action Check -LaunchMode Direct -GameKey $key
$encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
& powershell.exe -NoProfile -ExecutionPolicy Bypass -EncodedCommand $encoded
Assert-Test ($LASTEXITCODE -eq 0 -and ([IO.File]::ReadAllText($resultPath)|ConvertFrom-Json).ExitCode -eq 0) 'Direct worker handoff failed.'
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $root 'scripts\ui\Show-SteamLauncher.ps1'),[ref]$tokens,[ref]$errors)
Assert-Test ($errors.Count -eq 0) 'GUI syntax failed.'
$node=$ast.Find({param($n)$n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Add-LauncherGame'},$true)
. ([scriptblock]::Create($node.Extent.Text))
$script:preferences=@{SteamPath='';Profiles=@()};$games=[pscustomobject]@{Items=@()}
$script:folderCalls=0;$script:failure=''
function Select-Folder {param($Description) $script:folderCalls++;if($script:folderCalls -eq 1){return $game}else{return $config}}
function Select-File {param($Title,$Filter,$InitialDirectory) return $render}
function Get-PortableDataDirectory {return Join-Path $testRoot 'gui-prefs'}
function Load-Games {param($SelectedId) $script:addedSelectedId=$SelectedId}
function Show-Failure {param($Message) $script:failure=$Message}
Add-LauncherGame Direct
Assert-Test (-not $script:failure -and $script:preferences.Profiles.Count -eq 1) "Direct add failed: $script:failure"
$added=$script:preferences.Profiles[0]
Assert-Test ($added.LaunchMode -eq 'Direct' -and $added.QualityPreset -eq 'Tier2' -and $added.Id -eq $script:addedSelectedId -and $added.AppId -eq '') 'Direct add defaults/identity wrong.'
Write-Host "PASS: direct add, no Steam/AppID, stable recovery, path checks, dispatch, isolated worker DryRun and encoded handoff. Artifacts: $testRoot"
