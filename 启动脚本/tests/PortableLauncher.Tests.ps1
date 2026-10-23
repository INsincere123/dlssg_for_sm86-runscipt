# Isolated model/worker tests. No Steam, game, elevation or installed paths are mutated.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path -Parent $PSScriptRoot
. (Join-Path $root 'scripts\common\LauncherConfig.ps1')
. (Join-Path $root 'scripts\common\SteamLauncher.ps1')
. (Join-Path $root 'scripts\common\PortableLauncher.ps1')
function Assert-Test { param([bool]$Condition,[string]$Message) if (-not $Condition) { throw $Message } }
$testRoot=Join-Path $env:TEMP ('dlssg-portable-'+[guid]::NewGuid().ToString('N'))
$project=Join-Path $testRoot "中文 O'Brien [便携]"
$fixture=Join-Path $project '启动脚本'
$library=Join-Path $testRoot 'New Machine Library\steamapps'
$game=Join-Path $library 'common\Example Game'
$steam=Join-Path $testRoot 'Steam\steam.exe'
$config=Join-Path $testRoot 'User\Saved\Config\Windows'
foreach ($path in @($fixture,(Join-Path $fixture 'presets'),(Join-Path $fixture '配置\SteamUE5'),(Split-Path -Parent $steam),$game,$config)) { [void][IO.Directory]::CreateDirectory($path) }
[IO.File]::WriteAllText($steam,'inert')
$presets=@([pscustomobject]@{Id='example';Name='示例游戏';AppId='456';RenderExecutable='Example\Binaries\Win64\Example-Win64-Shipping.exe';ConfigDirectory=$config;EngineConfigSource='配置\SteamUE5\Engine.ini'})
[IO.File]::WriteAllText((Join-Path $fixture 'presets\SteamUE5.json'),(ConvertTo-Json -InputObject $presets),[Text.UTF8Encoding]::new($true))
[IO.File]::WriteAllText((Join-Path $fixture '配置\SteamUE5\Engine.ini'),'[SystemSettings]')
$steamApps=Join-Path (Split-Path -Parent $steam) 'steamapps'
[void][IO.Directory]::CreateDirectory($steamApps)
$escaped=(Split-Path -Parent $library).Replace('\','\\')
[IO.File]::WriteAllText((Join-Path $steamApps 'libraryfolders.vdf'),"`"libraryfolders`"`n{`n`"0`"`n{`n`"path`" `"$escaped`"`n}`n}")
[IO.File]::WriteAllText((Join-Path $library 'appmanifest_456.acf'),"`"AppState`"`n{`n`"appid`" `"456`"`n`"installdir`" `"Example Game`"`n}")
$preferences=Read-PortablePreferences -Directory (Join-Path $testRoot 'prefs')
$profiles=@(Get-PortableProfiles $preferences -Root $fixture)
Assert-Test ($profiles.Count -eq 1 -and -not $profiles[0].GameDirectory) 'Preset contains a machine-specific game path.'
$profile=$profiles[0]
$executable=Join-Path $game $profile.RenderExecutable
[void][IO.Directory]::CreateDirectory((Split-Path -Parent $executable))
[IO.File]::WriteAllText($executable,'inert')
foreach ($relative in @('version.dll','dlssg_sm86.ini','dlssg_sm86\runtime\nvngx_dlssg.dll','dlssg_sm86\backend\sm86_backend.dll',
    'dlssg_sm86\kernels\fg_kernels.dll','dlssg_sm86\xefg\xefg_kernels.dll','dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll')) {
    $file=Join-Path $project $relative
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $file)); [IO.File]::WriteAllText($file,'inert')
}
$model=Get-PortableGameModel $profile $steam -Root $fixture
Assert-Test ($model.Ready -and $model.GameDirectory -eq $game) 'New-machine library discovery failed.'
$profile.GameDirectory='Z:\Missing Old Machine\Game'
$model=Get-PortableGameModel $profile $steam -Root $fixture
Assert-Test ($model.Ready -and $model.GameDirectory -eq $game) 'Stale device path prevented discovery.'
$profile.GameDirectory=$game
Set-PortableProfile $preferences $profile
Save-PortablePreferences $preferences -Directory (Join-Path $testRoot 'prefs')
$reloaded=Read-PortablePreferences -Directory (Join-Path $testRoot 'prefs')
Assert-Test ($reloaded.Profiles[0].GameDirectory -eq $game) 'Device preference roundtrip failed.'
$merged=@(Get-PortableProfiles $reloaded -Root $fixture)
Assert-Test ($merged.Count -eq 1 -and $merged[0].Id -eq 'example' -and $merged[0].GameDirectory -eq $game) 'Reopening after saving duplicated or lost the preset.'
$custom=[pscustomobject]@{Id='custom-789';Name='Other';AppId='789';RenderExecutable='Other.exe';ConfigDirectory=$config;EngineConfigSource='配置\SteamUE5\Engine.ini';GameDirectory='';ProxyDll=2}
Set-PortableProfile $reloaded $custom
$merged=@(Get-PortableProfiles $reloaded -Root $fixture)
Assert-Test ($merged.Count -eq 2 -and @($merged | Where-Object Id -eq 'example').Count -eq 1 -and @($merged | Where-Object Id -eq 'custom-789').Count -eq 1) 'Preset/custom profile merging failed.'
$secondPreset=[pscustomobject]@{Id='another';Name='Another preset';AppId='123';RenderExecutable='Another.exe';ConfigDirectory=$config;EngineConfigSource='配置\SteamUE5\Engine.ini'}
[IO.File]::WriteAllText((Join-Path $fixture 'presets\SteamUE5.json'),(ConvertTo-Json -InputObject @($presets[0],$secondPreset)),[Text.UTF8Encoding]::new($true))
$merged=@(Get-PortableProfiles $reloaded -Root $fixture)
Assert-Test ($merged.Count -eq 3 -and @($merged | Where-Object Id -eq 'another').Count -eq 1) 'Multiple preset array normalization failed.'
Write-Host 'PASS: reopening saved preferences, no duplicate presets, custom profiles and multiple JSON presets.'
Assert-Test ((Get-PortableDataDirectory -Root $fixture) -ne (Get-PortableDataDirectory -Root ($fixture+'-moved'))) 'Different portable folders share preferences.'
$request=Write-PortableLaunchConfig $model (Join-Path $testRoot 'request')
$settings=Get-SteamUE5Settings -AppId 456 -Profile desktop -Path $request
Assert-Test ($settings.GameDirectory -eq $game -and $settings.RenderExecutable -eq $executable) 'GUI request does not roundtrip through actual core configuration reader.'
Write-Host 'PASS: path-free presets, Steam library discovery, stale-device fallback, local preferences and core INI handoff.'

$profile.ConfigDirectory=Join-Path $testRoot 'NotYetCreated'
$model=Get-PortableGameModel $profile $steam -Root $fixture
Assert-Test (-not $model.Ready -and $model.Status -match '原版启动') 'Missing game config not explained.'
$profile.ConfigDirectory=$config
$missing=Join-Path $project 'dlssg_sm86\sr\sr_kernels.dll'
Remove-Item -LiteralPath $missing
$model=Get-PortableGameModel $profile $steam -Root $fixture
Assert-Test (-not $model.Ready -and $model.Status -match '组件缺失') 'Incomplete package marked ready.'
[IO.File]::WriteAllText($missing,'inert')
$state=Join-Path $fixture '.state\steam-456'
[void][IO.Directory]::CreateDirectory($state)
$journal=@{Version=1;GameId='steam-456';Render=(Split-Path -Parent $executable);Engine=(Join-Path $config 'Engine.ini')}
[IO.File]::WriteAllText((Join-Path $state 'active.json'),($journal | ConvertTo-Json))
# A changed preference must never redirect recovery away from the recorded originals.
$profile.GameDirectory='Z:\AnotherGame'
$profile.ConfigDirectory='Z:\AnotherConfig'
$model=Get-PortableGameModel $profile '' -Root $fixture
Assert-Test ($model.Recovery -and -not $model.Ready -and $model.GameDirectory -eq $game -and $model.ConfigDirectory -eq $config) 'Recovery follows changed settings.'
$request=Write-PortableLaunchConfig $model (Join-Path $testRoot 'restore-request')
& {
    function Get-ItemProperty { param($Path,$ErrorAction) return $null }
    $restoring=Get-SteamUE5Settings -AppId 456 -Profile desktop -Path $request -Overrides @{RestoreDeployment=$true}
    Assert-Test (-not $restoring.LauncherPath) 'Recovery unexpectedly requires Steam.'
}
Write-Host 'PASS: missing config/resources, interrupted deployment lockout and original-path recovery even without Steam.'

# Exercise the actual worker wrapper without a GUI, game, UAC or real script entry.
$fakeWorker=Join-Path $project 'worker.ps1'
$workerText=@'
param($LauncherConfigPath,$Profile,$AppId,[switch]$DryRun,[switch]$RestoreDeployment)
if ($Profile -ne 'desktop' -or $AppId -ne '456') { throw 'Worker argument mismatch.' }
if (-not [IO.File]::Exists($LauncherConfigPath)) { throw 'Config path not correctly quoted.' }
exit 0
'@
[IO.File]::WriteAllText($fakeWorker,$workerText)
foreach ($action in @('Check','Restore','Launch')) {
    $resultPath=Join-Path $testRoot ("result-$action.json")
    $command=New-PortableWorkerCommand -ScriptPath $fakeWorker -ConfigPath $request -AppId 456 -ResultPath $resultPath -Action $action
    $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -EncodedCommand $encoded
    Assert-Test ($LASTEXITCODE -eq 0 -and ([IO.File]::ReadAllText($resultPath) | ConvertFrom-Json).ExitCode -eq 0) "Worker wrapper failed: $action"
}
[IO.File]::WriteAllText($fakeWorker,"throw 'simulated failure'")
$resultPath=Join-Path $testRoot 'result-failure.json'
$command=New-PortableWorkerCommand $fakeWorker $request 456 $resultPath Check
& powershell.exe -NoProfile -ExecutionPolicy Bypass -EncodedCommand ([Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command)))
$failed=[IO.File]::ReadAllText($resultPath) | ConvertFrom-Json
Assert-Test ($LASTEXITCODE -ne 0 -and $failed.ExitCode -ne 0 -and $failed.Error -match 'simulated failure') 'Worker failure reported success.'
Write-Host 'PASS: worker completion/failure and Chinese, spaces, apostrophe and bracket path quoting.'
foreach ($relative in @('scripts\common\PortableLauncher.ps1','scripts\ui\Show-SteamLauncher.ps1','scripts\tools\Export-PortableLauncher.ps1')) {
    $tokens=$null; $errors=$null
    [void][Management.Automation.Language.Parser]::ParseFile((Join-Path $root $relative),[ref]$tokens,[ref]$errors)
    Assert-Test ($errors.Count -eq 0) "Syntax errors: $relative"
}
Write-Host "All portable launcher checks passed. Artifacts: $testRoot"
