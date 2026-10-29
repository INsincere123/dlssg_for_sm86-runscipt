# Isolated tests; never launch Steam or access installed game directories.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$launcherRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $launcherRoot 'scripts\common\LauncherConfig.ps1')
. (Join-Path $launcherRoot 'scripts\common\SteamLauncher.ps1')
. (Join-Path $launcherRoot 'scripts\common\LauncherResources.ps1')
function Assert-Test { param([bool]$Condition, [string]$Message) if (-not $Condition) { throw $Message } }
function Expect-Failure { param([scriptblock]$Action) $failed = $false; try { & $Action } catch { $failed = $true }; Assert-Test $failed 'Expected failure.' }
function Write-Log { param([string]$Message) Write-Host $Message }
$testRoot = Join-Path $env:TEMP ('dlssg-steam-tests-' + [guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testRoot)
$config = Join-Path $testRoot '配置 空格.ini'
$gameRoot = Join-Path $testRoot "游戏 O'Brien"
$configText = @"
[Steam]
启动器路径=$testRoot\Steam\steam.exe
[SteamUE5:2358720]
游戏名称=黑神话：悟空
游戏目录=$gameRoot
渲染程序=b1\Binaries\Win64\b1-Win64-Shipping.exe
游戏配置目录=$testRoot\Config
配置源=配置\SteamUE5\Engine.ini
[SteamUE5:123]
GameName=Second Game
GameDirectory=$testRoot\other
RenderExecutable=Game.exe
ConfigDirectory=$testRoot\other-config
EngineConfigSource=配置\SteamUE5\Engine.ini
ProxyDll=6
"@
[IO.File]::WriteAllText($config, $configText, [Text.UTF8Encoding]::new($true))
$settings = Get-SteamUE5Settings 2358720 -Path $config
Assert-Test ($settings.GameName -eq '黑神话：悟空' -and $settings.ProxyName -eq 'version.dll') 'Default proxy or Chinese name lost.'
Assert-Test ($settings.RenderExecutable -eq (Join-Path $gameRoot 'b1\Binaries\Win64\b1-Win64-Shipping.exe')) 'Render path base incorrect.'
Assert-Test ((Get-SteamUE5Settings 123 -Path $config).ProxyName -eq 'd3d12.dll') 'AppID groups or English aliases failed.'
$names = @('version.dll','winmm.dll','dbghelp.dll','dinput8.dll','dxgi.dll','d3d12.dll')
foreach ($index in 1..6) {
    Assert-Test ((Get-SteamUE5Settings 2358720 -Overrides @{ ProxyDll = $index } -Path $config).ProxyName -eq $names[$index-1]) 'Proxy mapping failed.'
}
foreach ($bad in @('0','7','1.0','-1','winmm.dll','1;comment')) {
    Expect-Failure { Get-SteamUE5Settings 2358720 -Overrides @{ProxyDll=$bad} -Path $config }
}
Expect-Failure { Get-SteamUE5Settings '../2358720' -Path $config }
Expect-Failure { Get-SteamUE5Settings 999 -Path $config }
[IO.File]::WriteAllText($config, $configText.Replace('渲染程序=b1\Binaries\Win64\b1-Win64-Shipping.exe','渲染程序=..\escape.exe'))
Expect-Failure { Get-SteamUE5Settings 2358720 -Path $config }
[IO.File]::WriteAllText($config, $configText)
Assert-Test ((Get-SteamUE5Settings 2358720 -Overrides @{ConfigDirectory="$testRoot\override";EngineConfigSource="$testRoot\source.ini"} -Path $config).ConfigDirectory -eq "$testRoot\override") 'Explicit config override lost.'
Write-Host 'PASS: AppID isolation, Chinese labels, six mappings, defaults, aliases, overrides and path validation.'

# Automatic AppID detection: only inert manifests in temporary libraries are read.
$autoLibrary = Join-Path $testRoot 'Steam Library'
$autoApps = Join-Path $autoLibrary 'steamapps'
$autoGame = Join-Path $autoApps 'common\Game With Spaces'
[void][IO.Directory]::CreateDirectory($autoGame)
$manifest = Join-Path $autoApps 'appmanifest_456.acf'
$manifestText = @'
"AppState"
{
    "appid" "456"
    "name" "Game With Spaces"
    "installdir" "Game With Spaces"
}
'@
[IO.File]::WriteAllText($manifest, $manifestText)
$fakeSteam = Join-Path $testRoot 'Steam\steam.exe'
$detected = Resolve-SteamAppId $autoGame $fakeSteam
Assert-Test ($detected.AppId -eq '456' -and $detected.Manifest -eq $manifest) 'Own library manifest not detected.'
Expect-Failure { Resolve-SteamAppId (Join-Path $autoApps 'common\Unregistered Copy') $fakeSteam }
$autoConfig = $configText.Replace('[SteamUE5:2358720]','[SteamUE5:测试游戏]').Replace($gameRoot,$autoGame)
[IO.File]::WriteAllText($config,$autoConfig)
$auto = Get-SteamUE5Settings -Profile 测试游戏 -Path $config
Assert-Test ($auto.AppId -eq '456' -and $auto.Profile -eq 'SteamUE5:测试游戏') 'Named profile automatic detection failed.'
Expect-Failure { Get-SteamUE5Settings -Path $config }
Assert-Test ((Get-SteamUE5Settings -AppId 999 -Profile 测试游戏 -Path $config).AppId -eq '999') 'Explicit AppID did not override discovery.'
$singleConfig = $autoConfig.Substring(0,$autoConfig.IndexOf('[SteamUE5:123]'))
[IO.File]::WriteAllText($config,$singleConfig.Replace("`n","`r"))
Assert-Test ((Get-SteamUE5Settings -Path $config).AppId -eq '456') 'Single named profile or CR-only INI detection failed.'
[IO.File]::WriteAllText($config,$singleConfig.Replace('[SteamUE5:测试游戏]','[SteamUE5]'))
Assert-Test ((Get-SteamUE5Settings -Path $config).AppId -eq '456') 'Default generic profile not detected.'
Copy-Item -LiteralPath $manifest -Destination (Join-Path $autoApps 'appmanifest_789.acf')
[IO.File]::WriteAllText((Join-Path $autoApps 'appmanifest_789.acf'),$manifestText.Replace('456','789'))
Expect-Failure { Resolve-SteamAppId $autoGame $fakeSteam }
Remove-Item -LiteralPath (Join-Path $autoApps 'appmanifest_789.acf')
[IO.File]::WriteAllText($manifest,$manifestText.Replace('456','999'))
Expect-Failure { Resolve-SteamAppId $autoGame $fakeSteam }
[IO.File]::WriteAllText($manifest,$manifestText)
# Registered libraries are also inspected; duplicate paths must not create ambiguity.
$steamApps = Join-Path (Split-Path -Parent $fakeSteam) 'steamapps'
[void][IO.Directory]::CreateDirectory($steamApps)
$escapedLibrary = $autoLibrary.Replace('\','\\')
[IO.File]::WriteAllText((Join-Path $steamApps 'libraryfolders.vdf'), "`"libraryfolders`"`n{`n`"0`"`n{`n`"path`" `"$escapedLibrary`"`n}`n}")
Assert-Test ((Resolve-SteamAppId $autoGame $fakeSteam).AppId -eq '456') 'Duplicate registered/own library paths caused ambiguity.'
[IO.File]::WriteAllText($config,$configText)
Write-Host 'PASS: automatic manifest matching, named/default/single profiles, CR-only INI, explicit override, missing/ambiguous/invalid manifests and library deduplication.'

# Process polling uses test doubles only. Steam itself is deliberately irrelevant.
& {
    $executable = Join-Path $testRoot 'game.exe'
    $script:processList = @()
    function Get-Process { param($Name, $ErrorAction) $script:processList }
    $script:processList = @([pscustomobject]@{Id=1;Path="$testRoot\other\game.exe"})
    Assert-Test (@(Get-SteamGameProcesses $executable).Count -eq 0) 'Other installation mistaken for target.'
    $script:processList += [pscustomobject]@{Id=2;Path=$executable}
    Assert-Test (@(Get-SteamGameProcesses $executable).Count -eq 1) 'Target path not matched.'
    Expect-Failure { Assert-SteamGameStopped $executable }
    $script:processList = @([pscustomobject]@{Id=3;Path=$null;HasExited=$false})
    Expect-Failure { Get-SteamGameProcesses $executable }
    $script:processList = @([pscustomobject]@{Id=3;Path=$null;HasExited=$true})
    Assert-Test (@(Get-SteamGameProcesses $executable).Count -eq 0) 'Exited process blocked polling.'
    $script:processList = @()
    Wait-SteamGameSession $executable -TimeoutSeconds 0
    $script:poll = 0
    function Start-Sleep {
        param($Seconds)
        $script:poll++
        $script:processList = switch ($script:poll) {
            1 { @([pscustomobject]@{Id=4;Path=$executable}, [pscustomobject]@{Id=5;Path=$executable}) }
            2 { @([pscustomobject]@{Id=5;Path=$executable}) }
            3 { @() }
            default { throw 'Polling did not stop.' }
        }
    }
    Wait-SteamGameSession $executable
    Assert-Test ($script:poll -eq 3) 'Did not wait for every target process.'
}
Write-Host 'PASS: exact process paths, inaccessible/exited processes, timeout, multiple game processes and exit cleanup trigger.'

# Copy the journal module so its real .state path stays inside the fixture.
$common = Join-Path $testRoot 'launcher\scripts\common'
[void][IO.Directory]::CreateDirectory($common)
Copy-Item -LiteralPath (Join-Path $launcherRoot 'scripts\common\DeploymentJournal.ps1') -Destination $common
. (Join-Path $common 'DeploymentJournal.ps1')
function Get-NormalizedPath { param([string]$Path) [IO.Path]::GetFullPath($Path).TrimEnd('\').ToLowerInvariant() }
function Get-HashOrNull { param([string]$Path) if ([IO.File]::Exists($Path)) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash } }
$projectDir = Join-Path $testRoot 'project'
$renderDir = Join-Path $testRoot 'render'
[void][IO.Directory]::CreateDirectory((Join-Path $projectDir 'alternatives'))
[void][IO.Directory]::CreateDirectory((Join-Path $renderDir '_storage_'))
$engineConfigPath = Join-Path $testRoot 'Engine.ini'
$engineRuntimePath = Join-Path $testRoot 'Engine.runtime.ini'
$engineBackupPath = Join-Path $testRoot 'Engine.backup.ini'
$diagnosticIniPath = Join-Path $env:TEMP ("dlssg-sm86-steam-test-$PID.ini")
$linkSpecs = @(@{}, @{Path=(Join-Path $renderDir 'dlssg_sm86.ini');Target=$diagnosticIniPath})
foreach ($name in $names) {
    Select-DlssgProxy $projectDir $renderDir $name
    [IO.File]::WriteAllText($projectProxy, "source-$name")
    $expectedSource = if ($name -eq 'version.dll') { Join-Path $projectDir $name } else { Join-Path $projectDir "alternatives\$name" }
    Assert-Test ($projectProxy -eq $expectedSource) 'Wrong proxy source.'
    Open-DeploymentJournal -GameId steam-2358720 -GameProcess NoSuchGame -LauncherProcess @() -DiagnosticPrefix 'dlssg-sm86-steam-test-' -GameExecutable (Join-Path $renderDir 'game.exe') -SelectableProxy
    try {
        New-DeploymentJournal
        Assert-Test ($script:deploymentJournal.ProxyName -eq $name) 'Proxy not saved.'
        $op = Start-DeploymentOperation storage $storageDll '' (Get-HashOrNull $projectProxy)
        [IO.File]::WriteAllText($storageDll, "source-$name")
        Complete-DeploymentOperation $op
    } finally { Close-DeploymentJournal }
    $oldStorage = $storageDll
    # Simulate an INI edit selecting a different proxy before reopening.
    Select-DlssgProxy $projectDir $renderDir $names[([array]::IndexOf($names,$name)+1)%6]
    $script:deploymentJournal = $null
    Open-DeploymentJournal -GameId steam-2358720 -GameProcess NoSuchGame -LauncherProcess @() -DiagnosticPrefix 'dlssg-sm86-steam-test-' -GameExecutable (Join-Path $renderDir 'game.exe') -SelectableProxy
    try {
        Assert-Test ($deployment.Proxy -eq $name -and $storageDll -eq $oldStorage) 'Recovery followed new selection instead of record.'
        $before = [IO.File]::ReadAllBytes($script:journalPath)
        Restore-RecordedDeployment -Preview
        Assert-Test (([Convert]::ToBase64String($before)) -eq [Convert]::ToBase64String([IO.File]::ReadAllBytes($script:journalPath))) 'Preview changed journal.'
        [IO.File]::WriteAllText($storageDll, 'foreign')
        Expect-Failure { Restore-RecordedDeployment }
        Assert-Test ([IO.File]::ReadAllText($storageDll) -eq 'foreign') 'Foreign storage removed.'
        [IO.File]::WriteAllText($storageDll, "source-$name")
        Restore-RecordedDeployment -Automatic
        Assert-Test (-not (Test-Path -LiteralPath $oldStorage)) 'Recorded storage remained.'
        Restore-RecordedDeployment
    } finally { Close-DeploymentJournal }
}
Write-Host 'PASS: all six proxy sources, actual journal reload after selection change, preview, storage conflicts and repeated recovery.'

# Parse every changed executable source with the current Windows PowerShell parser.
foreach ($relative in @('scripts\games\Run-SteamUE5-DLSSG.ps1','scripts\common\SteamLauncher.ps1','scripts\common\DeploymentJournal.ps1','scripts\common\LauncherResources.ps1','scripts\common\LauncherConfig.ps1')) {
    $tokens=$null; $errors=$null
    [void][Management.Automation.Language.Parser]::ParseFile((Join-Path $launcherRoot $relative), [ref]$tokens, [ref]$errors)
    Assert-Test ($errors.Count -eq 0) "Syntax errors: $relative"
}
Write-Host "All Steam UE5 isolated checks passed. Artifacts: $testRoot"
