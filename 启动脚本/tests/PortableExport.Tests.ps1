# Exercise the real exporter using tiny, inert resources in an isolated fixture.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path -Parent $PSScriptRoot
$testRoot=Join-Path $env:TEMP ('dlssg-export-tests-'+[guid]::NewGuid().ToString('N'))
$project=Join-Path $testRoot '源目录 O''Brien [中文]'
$fixture=Join-Path $project '启动脚本'
function Assert-Test($Condition,[string]$Message) { if (-not $Condition) { throw $Message } }
function Write-Fixture([string]$Path,[string]$Text='inert fixture') {
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    [IO.File]::WriteAllText($Path,$Text,[Text.UTF8Encoding]::new($true))
}
function Expect-Failure([scriptblock]$Action) { $failed=$false; try { & $Action | Out-Null } catch { $failed=$true }; Assert-Test $failed 'Expected export failure.' }
[void][IO.Directory]::CreateDirectory($fixture)
Copy-Item -LiteralPath (Join-Path $root 'scripts') -Destination $fixture -Recurse
foreach ($entry in @('通用启动器.bat','虚幻启动器.bat','启动鸣潮.bat','启动异环.bat','切换显卡.bat','启动SteamUE5.bat')) {
    Copy-Item -LiteralPath (Join-Path $root $entry) -Destination $fixture
}
$resources=@('version.dll','alternatives\winmm.dll','alternatives\dbghelp.dll','alternatives\dinput8.dll','alternatives\dxgi.dll','alternatives\d3d12.dll',
    'alternatives\README.md','dlssg_sm86.ini','THIRD_PARTY_NOTICES.txt','dlssg_sm86\runtime\nvngx_dlssg.dll','dlssg_sm86\backend\sm86_backend.dll',
    'dlssg_sm86\kernels\fg_kernels.dll','dlssg_sm86\xefg\xefg_kernels.dll','dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll')
foreach ($file in $resources) { Write-Fixture (Join-Path $project $file) }
foreach ($file in @('配置\SteamUE5\Engine.ini','配置\鸣潮\Engine.ini','配置\异环\Engine.ini')) { Write-Fixture (Join-Path $fixture $file) }
Write-Fixture (Join-Path $fixture 'presets\SteamUE5.json') '[{"Id":"test","Name":"测试","AppId":"123","RenderExecutable":"Game.exe","ConfigDirectory":"%LOCALAPPDATA%\\test","EngineConfigSource":"配置\\SteamUE5\\Engine.ini"}]'
foreach ($engine in @('UE4','UE5')) {
    foreach ($tier in @('01-高性能均衡','02-主流均衡','03-性能优先','04-入门流畅','05-自定义')) { Write-Fixture (Join-Path $fixture "配置\虚幻通用分类\$engine\$tier\Engine.ini") }
}
foreach ($file in @('启动配置.ini','.state\old\active.json','logs\secret.log','preferences.json','backup.ini','tests\secret.ps1')) { Write-Fixture (Join-Path $fixture $file) 'PRIVATE_SENTINEL' }
Write-Fixture (Join-Path $project '.git\config') 'PRIVATE_SENTINEL'
$exporter=Join-Path $fixture 'scripts\tools\Export-PortableLauncher.ps1'
$folders=@{'1'='帧生成启动器';'2'='通用启动器';'3'='虚幻启动器';'4'='鸣潮异环帧生成解锁'}
foreach ($type in @('1','2','3','4')) {
    $destination=Join-Path $testRoot "包$type"
    & $exporter -PackageType $type -Destination $destination | Out-Null
    $package=Join-Path $destination $folders[$type]
    Assert-Test ([IO.Directory]::Exists($package)) "Wrong folder for type $type"
    $general=$type -in @('1','2'); $unreal=$type -in @('1','3'); $games=$type -in @('1','4')
    Assert-Test ([IO.File]::Exists((Join-Path $package '启动脚本\scripts\common\GameDiscovery.ps1')) -eq ($general -or $unreal)) "Shared discovery mismatch: $type"
    foreach ($relative in @('启动脚本\通用启动器.bat','启动脚本\scripts\ui\Show-DlssgLauncher.ps1')) {
        Assert-Test ([IO.File]::Exists((Join-Path $package $relative)) -eq $general) "General content mismatch: $type/$relative"
    }
    foreach ($relative in @('启动脚本\虚幻启动器.bat','启动脚本\scripts\ui\Show-SteamLauncher.ps1','启动脚本\presets\SteamUE5.json','启动脚本\配置\虚幻通用分类\UE5\05-自定义\Engine.ini')) {
        Assert-Test ([IO.File]::Exists((Join-Path $package $relative)) -eq $unreal) "Unreal content mismatch: $type/$relative"
    }
    foreach ($relative in @('启动脚本\启动鸣潮.bat','启动脚本\启动异环.bat','启动脚本\切换显卡.bat','启动脚本\配置\鸣潮\Engine.ini','启动脚本\启动配置.ini')) {
        Assert-Test ([IO.File]::Exists((Join-Path $package $relative)) -eq $games) "Specialized content mismatch: $type/$relative"
    }
    Assert-Test ([IO.File]::Exists((Join-Path $package '启动脚本\启动SteamUE5.bat')) -eq ($type -eq '1')) 'Full CLI entry missing/leaked.'
    Assert-Test ([IO.File]::Exists((Join-Path $package 'alternatives\d3d12.dll')) -eq ($general -or $unreal)) 'Proxy selection mismatch.'
    foreach ($relative in @('runtime\nvngx_dlssg.dll','backend\sm86_backend.dll','kernels\fg_kernels.dll','xefg\xefg_kernels.dll','sr\sr_kernels.dll','sr\nvngx_dlss.dll')) {
        Assert-Test ([IO.File]::Exists((Join-Path $package "dlssg_sm86\$relative"))) "Missing full resource: $type/$relative"
    }
    foreach ($relative in @('启动脚本\.state','启动脚本\logs','启动脚本\preferences.json','启动脚本\backup.ini','启动脚本\tests','启动脚本\scripts\tools\Export-PortableLauncher.ps1','.git')) {
        Assert-Test (-not (Test-Path -LiteralPath (Join-Path $package $relative))) "Private/tool content exported: $relative"
    }
    $guide=[IO.File]::ReadAllText((Join-Path $package '先看这里.txt'))
    Assert-Test ($guide.Contains('通用启动器.bat') -eq $general) "Wrong general guide for $type"
    Assert-Test ($guide.Contains('虚幻启动器.bat') -eq $unreal) "Wrong unreal guide for $type"
    Assert-Test ($guide.Contains('启动鸣潮.bat') -eq $games) "Wrong game guide for $type"
    if ($games) { Assert-Test (-not ([IO.File]::ReadAllText((Join-Path $package '启动脚本\启动配置.ini')).Contains('PRIVATE_SENTINEL'))) 'Personal config leaked.' }
    # Validate literal dot-source dependencies inside every exported script.
    $dependencyChecks=0
    foreach ($script in @(Get-ChildItem -LiteralPath (Join-Path $package '启动脚本\scripts') -Filter '*.ps1' -Recurse)) {
        foreach ($match in [regex]::Matches([IO.File]::ReadAllText($script.FullName), "(?m)^\s*\.\s+\(Join-Path\s+\`$PSScriptRoot\s+'([^']+)'\)")) {
            $dependency=[IO.Path]::GetFullPath((Join-Path $script.DirectoryName $match.Groups[1].Value))
            Assert-Test ([IO.File]::Exists($dependency)) "Exported dependency missing: $($script.Name) -> $dependency"
            $dependencyChecks++
        }
    }
    Assert-Test ($dependencyChecks -gt 0) "Dependency checks did not cover type $type"
    # Loading the model verifies indirect dependencies too, with no game discovery or launch.
    if ($general) { & { param($Path) . (Join-Path (Split-Path -Parent $Path) 'LauncherConfig.ps1'); . $Path; [void](Get-DlssgOnlyDataDirectory) } (Join-Path $package '启动脚本\scripts\common\DlssgOnlyLauncher.ps1') }
    Expect-Failure { & $exporter -PackageType $type -Destination $destination }
}
Write-Host 'PASS: all four real fixture exports, separate entries/configs, complete dependencies/resources, tailored guides and privacy exclusions.'

$default=Join-Path $testRoot '默认'
& $exporter -Destination $default | Out-Null
Assert-Test ([IO.File]::Exists((Join-Path $default '通用启动器\启动脚本\通用启动器.bat'))) 'Default must be general.'
$script:answers=[Collections.Generic.Queue[string]]::new(); $script:answers.Enqueue('invalid'); $script:answers.Enqueue('4')
function Read-Host { param($Prompt) return $answers.Dequeue() }
$interactive=Join-Path $testRoot '交互'
& $exporter -Interactive -Destination $interactive | Out-Null
Assert-Test ([IO.Directory]::Exists((Join-Path $interactive '鸣潮异环帧生成解锁'))) 'Interactive option 4 failed.'
$script:answers.Enqueue('2')
& $exporter -Interactive -PackageType 3 -Destination (Join-Path $testRoot '显式类型') | Out-Null
Assert-Test ($script:answers.Count -eq 1) 'Explicit package type did not bypass menu.'
$script:answers.Clear(); $script:answers.Enqueue('0')
$batchPrefix=Join-Path $testRoot '全部交互'
$batchOutput=@(& $exporter -Interactive -Destination $batchPrefix)
$types=@($batchOutput | Where-Object { $_ -match '^打包类型：' })
Assert-Test (($types -join '|') -eq '打包类型：完整|打包类型：通用启动器|打包类型：虚幻启动器|打包类型：鸣潮+异环') 'Option 0 did not run exactly 1,2,3,4 in order.'
foreach ($label in @('Full','General','Unreal','WuWa-NTE')) { Assert-Test ([IO.Directory]::Exists($batchPrefix+'-'+$label)) "Batch directory missing: $label" }
Remove-Item Function:\Read-Host
Expect-Failure { & $exporter -PackageType 5 -Destination (Join-Path $testRoot '非法类型') }
Write-Host 'PASS: default, all menu choices validated, retry input, option 4 and explicit type bypass.'

& $exporter -PackageType 0 | Out-Null
$defaultBatch=@(Get-ChildItem -LiteralPath (Join-Path $project 'dist') -Directory)
Assert-Test ($defaultBatch.Count -eq 4) 'Option 0 without Destination did not produce four default outputs.'
foreach ($label in @('Full','General','Unreal','WuWa-NTE')) {
    Assert-Test (@($defaultBatch | Where-Object Name -like "DLSSG-$label-*").Count -eq 1) "Default batch label missing: $label"
}
$allZip=Join-Path $testRoot '全部ZIP'
& $exporter -PackageType 0 -Destination $allZip -Zip | Out-Null
foreach ($label in @('Full','General','Unreal','WuWa-NTE')) {
    Assert-Test ([IO.File]::Exists($allZip+'-'+$label+'.zip') -and -not [IO.Directory]::Exists($allZip+'-'+$label)) "Batch ZIP/cleanup failed: $label"
}
$collision=Join-Path $testRoot '批量冲突'
Write-Fixture ($collision+'-General.zip') 'existing archive'
Expect-Failure { & $exporter -PackageType 0 -Destination $collision -Zip }
Assert-Test (-not [IO.File]::Exists($collision+'-Full.zip') -and -not [IO.Directory]::Exists($collision+'-Full')) 'Batch started before checking later collision.'
Assert-Test ([IO.File]::ReadAllText($collision+'-General.zip').Contains('existing archive')) 'Existing batch archive changed.'
# Inject a second-package failure only in the temporary exporter copy.
$faultExporter=Join-Path (Split-Path -Parent $exporter) 'Export-FaultFixture.ps1'
$faultText=[IO.File]::ReadAllText($exporter).Replace('$includeGeneral=', 'if ($PackageType -eq ''2'') { throw ''injected second-package failure'' }; $includeGeneral=')
Write-Fixture $faultExporter $faultText
$failedBatch=Join-Path $testRoot '批量中断'
Expect-Failure { & $faultExporter -PackageType 0 -Destination $failedBatch }
Assert-Test ([IO.Directory]::Exists($failedBatch+'-Full')) 'Batch failure discarded completed package.'
Assert-Test (-not [IO.Directory]::Exists($failedBatch+'-Unreal') -and -not [IO.Directory]::Exists($failedBatch+'-WuWa-NTE')) 'Batch continued after second-package failure.'
Write-Host 'PASS: option 0 order, inherited ZIP, four separate outputs, preflight collision and fail-stop preserving prior success.'

$zipDestination=Join-Path $testRoot 'ZIP测试'
& $exporter -PackageType 2 -Destination $zipDestination -Zip | Out-Null
Assert-Test ([IO.File]::Exists($zipDestination+'.zip') -and -not [IO.Directory]::Exists($zipDestination)) 'ZIP publish/cleanup failed.'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive=[IO.Compression.ZipFile]::OpenRead($zipDestination+'.zip')
try {
    Assert-Test (@($archive.Entries | Where-Object FullName -match '通用启动器[\\/]启动脚本[\\/]通用启动器\.bat$').Count -eq 1) 'ZIP layout wrong.'
    Assert-Test (@($archive.Entries | Where-Object { $_.FullName -notmatch '^通用启动器[\\/]' }).Count -eq 0) 'ZIP has multiple top-level folders.'
} finally { $archive.Dispose() }
Expect-Failure { & $exporter -PackageType 2 -Destination $zipDestination -Zip }
$missing=Join-Path $project 'dlssg_sm86\sr\sr_kernels.dll'
[IO.File]::Delete($missing)
$failedDestination=Join-Path $testRoot '缺资源'
Expect-Failure { & $exporter -PackageType 2 -Destination $failedDestination }
Assert-Test (-not [IO.Directory]::Exists($failedDestination)) 'Missing resource produced a partial export.'
Write-Host "PASS: actual tiny ZIP, single top-level folder, duplicate rejection, staging cleanup and preflight missing-resource failure. Fixtures: $testRoot"
