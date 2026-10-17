# Isolated configuration and device selection tests. No registry writes or games.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$launcherRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $launcherRoot 'scripts\common\LauncherConfig.ps1')
function Assert-Test { param([bool]$Condition,[string]$Message) if (-not $Condition) { throw $Message } }
function Expect-ConfigFailure {
    param([scriptblock]$Action,[string]$Message)
    $failed=$false
    try { & $Action | Out-Null } catch { $failed=$true; Assert-Test ($_.Exception.Message -match $Message) "Wrong failure: $_" }
    Assert-Test $failed 'Expected configuration failure.'
}
$testRoot = Join-Path $env:TEMP ('dlssg-config-tests-' + [guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testRoot)
$fixture = Join-Path $testRoot '配置 空格.ini'
function Write-Fixture {
    param([string]$Text)
    [IO.File]::WriteAllText($fixture,$Text,[Text.UTF8Encoding]::new($true))
}
$valid = @'
; 中文配置测试
[鸣潮]
游戏目录=游戏 目录
启动器路径=启动器\launcher.exe
配置源=配置\异环\Engine.ini
游戏配置目录=
重启等待秒数=25
[异环]
游戏目录=异环 游戏
启动器路径=异环 游戏\NTELauncher.exe
配置源=%TEMP%\明文 配置.ini
游戏配置目录=
[显卡]
设备实例标识=
目标显示名称=测试 显卡
'@
Write-Fixture $valid
$settings = Get-LauncherGameSettings Wuthering -Path $fixture
Assert-Test ($settings.GameDirectory -eq (Join-Path $launcherRoot '游戏 目录')) 'Relative path root changed.'
Assert-Test ($settings.RestartGraceSeconds -eq 25 -and -not $settings.ConfigDirectory) 'Defaults changed.'
$settings = Get-LauncherGameSettings Wuthering -Path $fixture -Overrides @{EngineConfigSource='候选 空格.ini';ConfigDirectory='配置 目录';RestartGraceSeconds=31}
Assert-Test ($settings.EngineConfigSource -eq (Join-Path $launcherRoot '候选 空格.ini')) 'Source override lost.'
Assert-Test ($settings.ConfigDirectory -eq (Join-Path $launcherRoot '配置 目录') -and $settings.RestartGraceSeconds -eq 31) 'Override lost.'
$settings = Get-LauncherGameSettings Nte -Path $fixture
Assert-Test ($settings.EngineConfigSource -eq (Join-Path $env:TEMP '明文 配置.ini')) 'Environment expansion failed.'
Assert-Test ($settings.ConfigDirectory -eq (Join-Path $env:LOCALAPPDATA 'HT\Saved\Config\Windows')) 'User directory fallback changed.'
Write-Fixture ($valid.Substring(0,$valid.IndexOf('[显卡]')))
Get-LauncherGameSettings Wuthering -Path $fixture | Out-Null
Write-Fixture ($valid.Replace('重启等待秒数=25','重启等待秒数=0'))
Expect-ConfigFailure { Get-LauncherGameSettings Wuthering -Path $fixture } 'RestartGraceSeconds'
Get-LauncherGameSettings Nte -Path $fixture | Out-Null
Write-Fixture ($valid.Replace('游戏目录=游戏 目录','游戏目录='))
Expect-ConfigFailure { Get-LauncherGameSettings Wuthering -Path $fixture } 'Wuthering.GameDirectory'
Write-Fixture ($valid.Replace('启动器路径=启动器\launcher.exe','错误路径=123'))
Expect-ConfigFailure { Get-LauncherGameSettings Wuthering -Path $fixture } '未知配置项'
Write-Fixture '[错误分组'
Expect-ConfigFailure { Get-LauncherGameSettings Wuthering -Path $fixture } '格式错误'
Expect-ConfigFailure { Get-LauncherGameSettings Wuthering -Path (Join-Path $testRoot 'absent.ini') } '无法读取启动配置'
Expect-ConfigFailure { Resolve-LauncherConfigPath '%DLSSG_NONEXISTENT_TEST_VARIABLE%\a' 'Test.Path' } '未定义'
Write-Host 'PASS: config parsing, section isolation, paths, environment, overrides and invalid settings.'

$encodings = @(
    [Text.UTF8Encoding]::new($false,$true), [Text.UTF8Encoding]::new($true,$true),
    [Text.UnicodeEncoding]::new($false,$true,$true), [Text.UnicodeEncoding]::new($true,$true,$true),
    [Text.UTF32Encoding]::new($false,$true,$true), [Text.UTF32Encoding]::new($true,$true,$true),
    [Text.Encoding]::GetEncoding(936,[Text.EncoderFallback]::ExceptionFallback,[Text.DecoderFallback]::ExceptionFallback)
)
foreach ($encoding in $encodings) {
    foreach ($newline in @("`r`n","`n","`r")) {
        [IO.File]::WriteAllText($fixture,($valid -replace '\r\n|\n|\r',$newline),$encoding)
        $before=(Get-FileHash -LiteralPath $fixture).Hash
        $settings = Get-LauncherGameSettings Wuthering -Path $fixture -WarningVariable decodeWarnings
        Assert-Test ($settings.GameDirectory -eq (Join-Path $launcherRoot '游戏 目录')) "Decode failed: $($encoding.WebName)"
        Assert-Test ($before -eq (Get-FileHash -LiteralPath $fixture).Hash) 'Reader rewrote the configuration.'
        if ($encoding.CodePage -eq 936) { Assert-Test (@($decodeWarnings).Count -gt 0) 'GBK fallback must warn.' }
    }
}
[IO.File]::WriteAllBytes($fixture,[byte[]]@(239,187,191,255))
Expect-ConfigFailure { Read-LauncherConfigSection Wuthering -Path $fixture } '另存为 UTF-8'
[IO.File]::WriteAllBytes($fixture,[byte[]]@(255,254,65))
Expect-ConfigFailure { Read-LauncherConfigSection Wuthering -Path $fixture } '另存为 UTF-8'
[IO.File]::WriteAllText($fixture,$valid,[Text.UnicodeEncoding]::new($false,$false,$true))
Expect-ConfigFailure { Read-LauncherConfigSection Wuthering -Path $fixture } '无效文本'
Write-Fixture ($valid + "`n[鸣潮]`n游戏目录=重复")
Expect-ConfigFailure { Get-LauncherGameSettings Wuthering -Path $fixture } '分组 Wuthering 重复'
Write-Fixture ($valid.Replace('游戏目录=游戏 目录',"游戏目录=游戏 目录`nGameDirectory=重复"))
Expect-ConfigFailure { Get-LauncherGameSettings Wuthering -Path $fixture } '配置项 Wuthering.GameDirectory 重复'
Write-Fixture ($valid.Replace('重启等待秒数=25','重启等待秒数=25.5'))
Expect-ConfigFailure { Get-LauncherGameSettings Wuthering -Path $fixture } 'RestartGraceSeconds'
Write-Fixture ($valid.Replace('游戏目录=游戏 目录','游戏目录="C:\中文 空格\#版本;=一"'))
$settings=Get-LauncherGameSettings Wuthering -Path $fixture
Assert-Test ($settings.GameDirectory -eq 'C:\中文 空格\#版本;=一') 'INI comments/equals/quoted path were truncated.'
Write-Fixture ($valid.Replace('[鸣潮]','[Wuthering]').Replace('游戏目录=游戏 目录','GameDirectory=游戏 目录'))
Get-LauncherGameSettings Wuthering -Path $fixture | Out-Null
Write-Host 'PASS: UTF-8/16/32 BOMs, GBK warnings, CRLF/LF/CR, no rewrite, malformed encodings and INI duplicates.'

$cases = @(
    @('C:/中文 空格/game.exe','C:\中文 空格\game.exe'),
    @('  "C:\中文 空格\game.exe"  ','C:\中文 空格\game.exe'),
    @("'C:\中文 空格\game.exe'",'C:\中文 空格\game.exe'),
    @('“C:\中文 空格\game.exe”','C:\中文 空格\game.exe'),
    @(([string][char]0x2018 + 'C:\中文 空格\game.exe' + [char]0x2019),'C:\中文 空格\game.exe'),
    @('.\子目录\..\配置.ini',(Join-Path $launcherRoot '配置.ini')),
    @('../外部配置.ini',(Join-Path (Split-Path -Parent $launcherRoot) '外部配置.ini')),
    @('%LOCALAPPDATA%/配置.ini',(Join-Path $env:LOCALAPPDATA '配置.ini')),
    @('$env:LOCALAPPDATA\配置.ini',(Join-Path $env:LOCALAPPDATA '配置.ini')),
    @('${env:LOCALAPPDATA}/配置.ini',(Join-Path $env:LOCALAPPDATA '配置.ini')),
    @('~/配置.ini',(Join-Path ([Environment]::GetFolderPath('UserProfile')) '配置.ini')),
    @('\\server\share\中文 空格\Engine.ini','\\server\share\中文 空格\Engine.ini'),
    @('//server/share/配置.ini','\\server\share\配置.ini'),
    @('file:///C:/%E4%B8%AD%E6%96%87%20space/Engine.ini','C:\中文 space\Engine.ini'),
    @('file://server/share/Engine.ini','\\server\share\Engine.ini'),
    @('FileSystem::C:\中文 空格\game.exe','C:\中文 空格\game.exe'),
    @('Microsoft.PowerShell.Core\FileSystem::C:\game.exe','C:\game.exe'),
    @('\\?\C:\中文 空格\game.exe','C:\中文 空格\game.exe'),
    @('\\?\UNC\server\share\Engine.ini','\\server\share\Engine.ini'),
    @('C:\目录\O''Brien\#版本;=一.ini','C:\目录\O''Brien\#版本;=一.ini')
)
Push-Location $env:TEMP
try {
    foreach ($case in $cases) {
        $resolved=Resolve-LauncherConfigPath -Value $case[0] -Name 'Test.Path'
        Assert-Test ($resolved -eq $case[1]) "Path failed: $($case[0]) => $resolved; expected $($case[1])"
    }
    $rootDrive=[IO.Path]::GetPathRoot($launcherRoot)
    Assert-Test ((Resolve-LauncherConfigPath '\根目录\配置.ini' 'Test.Path') -eq (Join-Path $rootDrive '根目录\配置.ini')) 'Drive-root path depends on current directory.'
}
finally { Pop-Location }
foreach ($invalid in @('C:relative','C:','https://example.com/file','"C:\bad','C:\bad*.ini','C:\bad?.ini','\\?\C:\bad*.ini','\\.\PhysicalDrive0')) {
    Expect-ConfigFailure { Resolve-LauncherConfigPath $invalid 'Test.Path' } 'Test.Path'
}
Expect-ConfigFailure { Resolve-LauncherConfigPath '$env:DLSSG_NONEXISTENT_TEST_VARIABLE\a' 'Test.Path' } '未定义'
Write-Fixture $valid
Write-Host 'PASS: quotes, slashes, relative/root/UNC/extended paths, file URIs, home, environment and invalid paths.'

$discoveryRoot = Join-Path $testRoot '启动器查找'
$gameRoot = Join-Path $discoveryRoot '游戏'
$defaultRoot = Join-Path $testRoot '默认安装目录'
[void][IO.Directory]::CreateDirectory($gameRoot)
[void][IO.Directory]::CreateDirectory($defaultRoot)
foreach ($game in @('Wuthering','Nte')) {
    $exe = if ($game -eq 'Wuthering') { 'launcher.exe' } else { 'NTELauncher.exe' }
    Expect-ConfigFailure { Find-LauncherExecutable $game $gameRoot } '未找到启动器'
    Assert-Test (-not (Find-LauncherExecutable $game $gameRoot -RestoreOnly)) 'Missing launcher blocked recovery.'
    $parentLauncher = Join-Path $discoveryRoot $exe
    [IO.File]::WriteAllText($parentLauncher,'fake launcher; never execute')
    Assert-Test ((Find-LauncherExecutable $game $gameRoot) -eq $parentLauncher) 'Parent launcher not found.'
    $rootLauncher = Join-Path $gameRoot $exe
    [IO.File]::WriteAllText($rootLauncher,'fake launcher; never execute')
    Expect-ConfigFailure { Find-LauncherExecutable $game $gameRoot } '多个启动器'
    Assert-Test (-not (Find-LauncherExecutable $game $gameRoot -RestoreOnly)) 'Ambiguous launchers blocked recovery.'
    Remove-Item -LiteralPath $parentLauncher
    Assert-Test ((Find-LauncherExecutable $game $gameRoot -DefaultDirectories @($gameRoot)) -eq $rootLauncher) 'Same candidate duplicated.'
    Remove-Item -LiteralPath $rootLauncher
    $defaultLauncher = Join-Path $defaultRoot $exe
    [IO.File]::WriteAllText($defaultLauncher,'fake launcher; never execute')
    Assert-Test ((Find-LauncherExecutable $game $gameRoot -DefaultDirectories @($defaultRoot)) -eq $defaultLauncher) 'Default install location not found.'
    Remove-Item -LiteralPath $defaultLauncher
}
Write-Fixture ($valid.Replace('启动器路径=启动器\launcher.exe','启动器路径='))
& {
    function Find-LauncherExecutable { param($GameId,$GameDirectory,$DefaultDirectories,[switch]$RestoreOnly) return 'C:\Detected\launcher.exe' }
    Assert-Test ((Get-LauncherGameSettings Wuthering -Path $fixture).LauncherPath -eq 'C:\Detected\launcher.exe') 'Blank setting did not discover launcher.'
    function Find-LauncherExecutable { throw 'Cached launcher must not be searched again.' }
    Assert-Test ((Get-LauncherGameSettings Wuthering -Path $fixture).LauncherPath -eq 'C:\Detected\launcher.exe') 'Detected path was not persisted.'
    Write-Fixture $valid
    Assert-Test ((Get-LauncherGameSettings Wuthering -Path $fixture).LauncherPath -eq (Join-Path $launcherRoot '启动器\launcher.exe')) 'Explicit launcher lost priority.'
}
Write-Host 'PASS: optional launcher, same/parent/default directory, deduplication, ambiguity, explicit priority and recovery without launcher.'

$emptyLauncher = $valid.Replace('启动器路径=启动器\launcher.exe','启动器路径=')
foreach ($encoding in $encodings) {
    foreach ($newline in @("`r`n","`n","`r")) {
        $original = $emptyLauncher -replace '\r\n|\n|\r',$newline
        [IO.File]::WriteAllText($fixture,$original,$encoding)
        Save-LauncherDetectedPath -Path $fixture -GameId Wuthering -LauncherPath 'C:\缓存 路径\launcher.exe' | Out-Null
        $expected = $original.Replace('启动器路径='+$newline,'启动器路径=C:\缓存 路径\launcher.exe'+$newline)
        $expectedBytes=[byte[]]($encoding.GetPreamble()+$encoding.GetBytes($expected))
        Assert-Test ([Convert]::ToBase64String([IO.File]::ReadAllBytes($fixture)) -ceq [Convert]::ToBase64String($expectedBytes)) 'Cache changed encoding, BOM, newline, comments or unrelated settings.'
    }
}
Write-Fixture ($emptyLauncher.Replace("启动器路径=`r`n",'').Replace("启动器路径=`n",''))
Save-LauncherDetectedPath -Path $fixture -GameId Wuthering -LauncherPath 'C:\Inserted\launcher.exe' | Out-Null
Assert-Test ((Read-LauncherConfigSection Wuthering -Path $fixture).LauncherPath -eq 'C:\Inserted\launcher.exe') 'Missing key was not inserted.'
Write-Fixture $emptyLauncher
& {
    function Find-LauncherExecutable { return 'C:\Detected\launcher.exe' }
    $before=(Get-FileHash -LiteralPath $fixture).Hash
    Get-LauncherGameSettings Wuthering -Path $fixture -Overrides @{DryRun=$true} | Out-Null
    Get-LauncherGameSettings Wuthering -Path $fixture -Overrides @{RestoreDeployment=$true} | Out-Null
    Assert-Test ($before -eq (Get-FileHash -LiteralPath $fixture).Hash) 'DryRun/recovery rewrote config.'
}
$before=(Get-FileHash -LiteralPath $fixture).Hash
[IO.File]::SetAttributes($fixture,[IO.FileAttributes]::ReadOnly)
try { Expect-ConfigFailure { Save-LauncherDetectedPath $fixture Wuthering 'C:\Detected\launcher.exe' } '只读' }
finally { [IO.File]::SetAttributes($fixture,[IO.FileAttributes]::Normal) }
Assert-Test ($before -eq (Get-FileHash -LiteralPath $fixture).Hash) 'Failed cache write changed original.'
Write-Fixture $valid
Write-Host 'PASS: cache persists once, preserves encodings/newlines, inserts omitted key, skips DryRun/recovery and keeps readonly original.'

$configCacheRoot = Join-Path $testRoot '已找到的配置 目录'
[void][IO.Directory]::CreateDirectory($configCacheRoot)
foreach ($encoding in $encodings) {
    foreach ($newline in @("`r`n","`n","`r")) {
        $original = $valid -replace '\r\n|\n|\r',$newline
        [IO.File]::WriteAllText($fixture,$original,$encoding)
        Save-LauncherDetectedConfigDirectory -GameId Nte -Directory $configCacheRoot -Path $fixture | Out-Null
        $at=$original.IndexOf('[异环]')
        $expected=$original.Substring(0,$at)+$original.Substring($at).Replace('游戏配置目录='+$newline,'游戏配置目录='+$configCacheRoot+$newline)
        $bytes=[byte[]]($encoding.GetPreamble()+$encoding.GetBytes($expected))
        Assert-Test ([Convert]::ToBase64String([IO.File]::ReadAllBytes($fixture)) -ceq [Convert]::ToBase64String($bytes)) 'Config directory cache changed unrelated bytes.'
        $before=(Get-FileHash -LiteralPath $fixture).Hash
        Save-LauncherDetectedConfigDirectory -GameId Nte -Directory $configCacheRoot -Path $fixture | Out-Null
        Assert-Test ($before -eq (Get-FileHash -LiteralPath $fixture).Hash) 'Repeated directory cache rewrote config.'
    }
}
foreach ($overrides in @(@{DryRun=$true},@{RestoreDeployment=$true},@{RestoreEngineConfig=$true},@{ConfigDirectory=$configCacheRoot})) {
    Write-Fixture $valid
    $before=(Get-FileHash -LiteralPath $fixture).Hash
    Save-LauncherDetectedConfigDirectory -GameId Wuthering -Directory $configCacheRoot -Path $fixture -Overrides $overrides | Out-Null
    Assert-Test ($before -eq (Get-FileHash -LiteralPath $fixture).Hash) 'Temporary/recovery settings were persisted.'
}
Expect-ConfigFailure { Save-LauncherDetectedConfigDirectory Wuthering (Join-Path $testRoot 'absent-directory') -Path $fixture } '不存在'
Write-Fixture ($valid.Replace('游戏配置目录=','ConfigDirectory=""'))
Save-LauncherDetectedConfigDirectory Wuthering $configCacheRoot -Path $fixture | Out-Null
Assert-Test ((Read-LauncherConfigSection Wuthering -Path $fixture).ConfigDirectory -eq $configCacheRoot) 'English directory alias not cached.'

# Exercise the actual WuWa finder, then prove a stored directory bypasses a later ambiguous search.
& {
    $gameDir = Join-Path $testRoot '模拟鸣潮安装'
    $found = Join-Path $gameDir 'Client\Saved\Config\WindowsNoEditor'
    [void][IO.Directory]::CreateDirectory($found)
    [IO.File]::WriteAllText((Join-Path $found 'GameUserSettings.ini'),'[/Script/Engine.GameUserSettings]')
    $tool = Join-Path $launcherRoot 'scripts\games\Run-WutheringWaves-DLSSG.ps1'
    $tokens=$null; $errors=$null
    $ast=[Management.Automation.Language.Parser]::ParseFile($tool,[ref]$tokens,[ref]$errors)
    $node=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Resolve-EngineConfigDirectory'},$true)
    . ([scriptblock]::Create($node.Extent.Text))
    function Assert-FileExists { param($Path,$Description) if (-not [IO.File]::Exists($Path)) { throw "Missing $Description" } }
    $ConfigDirectory=''
    $resolved=Resolve-EngineConfigDirectory
    Write-Fixture $valid
    Save-LauncherDetectedConfigDirectory Wuthering $resolved -Path $fixture | Out-Null
    $ConfigDirectory=(Get-LauncherGameSettings Wuthering -Path $fixture).ConfigDirectory
    $another=Join-Path $gameDir 'Client\Saved\Config\Other'
    [void][IO.Directory]::CreateDirectory($another)
    [IO.File]::WriteAllText((Join-Path $another 'GameUserSettings.ini'),'[/Script/Engine.GameUserSettings]')
    Assert-Test ((Resolve-EngineConfigDirectory) -eq $found) 'Stored directory was searched again.'
    $ConfigDirectory=''
    Expect-ConfigFailure { Resolve-EngineConfigDirectory } '2 个候选'
}
Write-Fixture $valid
Write-Host 'PASS: game directory cache, encoding preservation, repeat reads, explicit/temporary priority and actual WuWa discovery bypass.'

# A fresh project copy can read its personal sources without initializing either submodule.
$offlineRoot = Join-Path $testRoot '未初始化上游的设备 副本'
[void][IO.Directory]::CreateDirectory((Join-Path $offlineRoot 'scripts\common'))
Copy-Item -LiteralPath (Join-Path $launcherRoot 'scripts\common\LauncherConfig.ps1') -Destination (Join-Path $offlineRoot 'scripts\common\LauncherConfig.ps1')
Copy-Item -LiteralPath (Join-Path $launcherRoot '启动配置.ini') -Destination (Join-Path $offlineRoot '启动配置.ini')
foreach ($game in @('鸣潮','异环')) {
    [void][IO.Directory]::CreateDirectory((Join-Path $offlineRoot "配置\$game"))
    Copy-Item -LiteralPath (Join-Path $launcherRoot "配置\$game\Engine.ini") -Destination (Join-Path $offlineRoot "配置\$game\Engine.ini")
}
& {
    . (Join-Path $offlineRoot 'scripts\common\LauncherConfig.ps1')
    foreach ($game in @('Wuthering','Nte')) {
        $personal = Get-LauncherGameSettings $game -Overrides @{RestoreDeployment=$true}
        Assert-Test ([IO.File]::Exists($personal.EngineConfigSource)) "Personal source missing without submodules: $game"
        Assert-Test ($personal.EngineConfigSource.StartsWith($offlineRoot)) "Personal source still depends on old device: $game"
    }
}
Write-Host 'PASS: both migrated personal sources resolve in a copied project without upstream submodules.'

$first = [pscustomobject]@{Class='Display';InstanceId='PCI\VEN_10DE&DEV_1111\1';FriendlyName='NVIDIA One'}
$second = [pscustomobject]@{Class='Display';InstanceId='PCI\VEN_10DE&DEV_2222\2';FriendlyName='NVIDIA Two'}
$intel = [pscustomobject]@{Class='Display';InstanceId='PCI\VEN_8086&DEV_3333\3';FriendlyName='Intel'}
Expect-ConfigFailure { Select-LauncherNvidiaDevice @() } 'Gpu.DeviceInstanceId'
Expect-ConfigFailure { Select-LauncherNvidiaDevice @($intel) } 'Gpu.DeviceInstanceId'
Assert-Test ((Select-LauncherNvidiaDevice @($first,$intel)) -eq $first.InstanceId) 'Unique NVIDIA selection failed.'
Expect-ConfigFailure { Select-LauncherNvidiaDevice @($first,$second) } 'NVIDIA One[\s\S]*NVIDIA Two'
Assert-Test ((Select-LauncherNvidiaDevice @($first,$second) $second.InstanceId.ToLowerInvariant()) -eq $second.InstanceId) 'Explicit device selection failed.'
Expect-ConfigFailure { Select-LauncherNvidiaDevice @($first,$intel) $intel.InstanceId } 'Gpu.DeviceInstanceId'
Expect-ConfigFailure { Select-LauncherNvidiaDevice @($first) 'PCI\VEN_10DE&DEV_ABSENT' } 'Gpu.DeviceInstanceId'
Write-Fixture $valid
function Get-PnpDevice {
    param([string]$Class,[switch]$PresentOnly,[string]$ErrorAction)
    Assert-Test ($Class -eq 'Display' -and $PresentOnly) 'Enumeration must require present display devices.'
    return @($first,$intel)
}
$gpu = Get-LauncherGpuSettings -Path $fixture
Assert-Test ($gpu.DeviceInstanceId -eq $first.InstanceId -and $gpu.TargetName -eq '测试 显卡') 'GPU settings lost.'
Write-Host 'PASS: zero/one/multiple GPUs, explicit identity and mocked present-device enumeration.'

# Exercise the actual status branch and its function bodies with simulated read-only device state.
$tool = Join-Path $launcherRoot 'scripts\tools\Toggle-NvidiaDeviceName.ps1'
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($tool,[ref]$tokens,[ref]$errors)
$names=@('Show-Status','Get-DeviceProperties','Split-DeviceDescription')
foreach ($node in $ast.FindAll({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst]},$true)) {
    if ($node.Name -in $names) { . ([scriptblock]::Create($node.Extent.Text)) }
}
function Get-ItemProperty { param([string]$LiteralPath,[string]$ErrorAction) [pscustomobject]@{DeviceDesc='@driver.inf;测试 显卡'} }
function Test-Path { param([string]$LiteralPath) return $LiteralPath -eq $deviceKeyPath }
function Write-Log { param([string]$Message) Write-Host $Message }
function Assert-Administrator { }
function Show-CurrentStateDialog { }
function Set-ItemProperty { throw 'Registry mutation is forbidden in status tests.' }
$deviceKeyPath = 'HKLM:\SYSTEM\CurrentControlSet\Enum\' + $gpu.DeviceInstanceId
$targetName = $gpu.TargetName
$statePath = Join-Path $testRoot 'absent-state.json'
$Status=$true
$branch=$ast.Find({param($n) $n -is [Management.Automation.Language.IfStatementAst] -and $n.Clauses[0].Item1.Extent.Text -eq '$Status' -and $n.Clauses[0].Item2.Extent.Text -match 'Show-Status'},$true)
Assert-Test ($null -ne $branch) 'Status branch not found.'
& ([scriptblock]::Create($branch.Clauses[0].Item2.Extent.Text.Trim().TrimStart('{').TrimEnd('}').Replace('exit 0','return')))
Write-Host 'PASS: actual tool status branch, simulated read-only registry, no UI or mutation.'
Remove-Item Function:\Test-Path

$targets = @{
    '启动鸣潮.bat' = 'scripts\games\Run-WutheringWaves-DLSSG.ps1'
    '启动异环.bat' = 'scripts\games\Run-NevernessToEverness-DLSSG.ps1'
    '切换显卡.bat' = 'scripts\tools\Toggle-NvidiaDeviceName.ps1'
}
foreach ($bat in $targets.Keys) {
    $text = [IO.File]::ReadAllText((Join-Path $launcherRoot $bat))
    Assert-Test ($text.Contains('%~dp0scripts\common\LaunchFromBat.ps1') -and $text.Contains('%~dp0' + $targets[$bat]) -and $text.Contains('%*')) "BAT target/forwarding changed: $bat"
    Assert-Test ([IO.File]::Exists((Join-Path $launcherRoot $targets[$bat]))) "Missing BAT target: $bat"
}
$capture = @{}
function Start-Process {
    param([string]$FilePath,[string]$Verb,[string]$ArgumentList)
    $capture.Start = @{FilePath=$FilePath;Verb=$Verb;ArgumentList=$ArgumentList}
}
$argument = "D:\中文 空格\O'Brien\Engine.ini"
& (Join-Path $launcherRoot 'scripts\common\LaunchFromBat.ps1') -ScriptPath (Join-Path $launcherRoot $targets['启动鸣潮.bat']) -EngineConfigSource $argument -DryRun
Assert-Test ($capture.Start.Verb -eq 'RunAs' -and $capture.Start.FilePath.EndsWith('\conhost.exe')) 'Elevation host changed.'
$encoded = ($capture.Start.ArgumentList -split ' ')[-1]
$command = [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($encoded))
Assert-Test ($command.Contains($argument.Replace("'","''")) -and $command.Contains('-DryRun')) 'Quoted argument forwarding lost.'
Assert-Test ($command.Contains((Join-Path $launcherRoot 'logs'))) 'Failure dialog log directory changed.'
Write-Host 'PASS: Chinese BAT targets and mocked elevation, Unicode/spaces/quote argument forwarding.'
Write-Host "All configuration checks passed. Artifacts: $testRoot"
