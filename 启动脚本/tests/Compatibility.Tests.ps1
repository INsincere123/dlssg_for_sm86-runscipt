# Isolated relocation checks on the CURRENT OS. This does not emulate Windows 10/11.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$launcherRoot=Split-Path -Parent $PSScriptRoot
function Assert-Test { param([bool]$Condition,[string]$Message) if (-not $Condition) { throw $Message } }
$testRoot=Join-Path $env:TEMP ('dlssg-compat-' + [guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testRoot)
foreach ($scenario in @('English User','中文 用户 O''Brien [迁移]')) {
    $project=Join-Path $testRoot $scenario
    $copyRoot=Join-Path $project '启动脚本'
    [void][IO.Directory]::CreateDirectory($copyRoot)
    Copy-Item -LiteralPath (Join-Path $launcherRoot 'scripts') -Destination $copyRoot -Recurse
    $resources=@('version.dll','alternatives\winmm.dll','dlssg_sm86.ini',
        'dlssg_sm86\runtime\nvngx_dlssg.dll','dlssg_sm86\backend\sm86_backend.dll',
        'dlssg_sm86\kernels\fg_kernels.dll','dlssg_sm86\xefg\xefg_kernels.dll',
        'dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll')
    foreach ($relative in $resources) {
        $file=Join-Path $project $relative
        [void][IO.Directory]::CreateDirectory((Split-Path -Parent $file))
        [IO.File]::WriteAllText($file,'inert fixture; never load')
    }
    $userData=Join-Path $project 'UserData'
    $files=@('Games\WuWa\Client\Binaries\Win64\Client-Win64-Shipping.exe',
        'Games\launcher.exe','Games\Nte\Client\WindowsNoEditor\HT\Binaries\Win64\HTGame.exe',
        'Games\Nte\NTELauncher.exe','配置\鸣潮\Engine.ini','配置\异环\Engine.ini',
        'Games\WuWa\Client\Saved\Config\WindowsNoEditor\GameUserSettings.ini')
    foreach ($relative in $files) {
        $file=Join-Path $copyRoot $relative
        [void][IO.Directory]::CreateDirectory((Split-Path -Parent $file))
        [IO.File]::WriteAllText($file,'[/Script/Engine.GameUserSettings]')
    }
    [void][IO.Directory]::CreateDirectory((Join-Path $userData 'HT\Saved\Config\Windows'))
    $ini=@'
[鸣潮]
游戏目录=Games/WuWa
启动器路径=
配置源=配置/鸣潮/Engine.ini
游戏配置目录=
重启等待秒数=10
[异环]
游戏目录=Games/Nte
启动器路径=
配置源=配置/异环/Engine.ini
游戏配置目录=
[显卡]
设备实例标识=
目标显示名称=NVIDIA GeForce RTX 5070
'@
    $iniPath=Join-Path $copyRoot '启动配置.ini'
    [IO.File]::WriteAllText($iniPath,$ini,[Text.UTF8Encoding]::new($false))
    foreach ($game in @('Run-WutheringWaves-DLSSG.ps1','Run-NevernessToEverness-DLSSG.ps1')) {
        $scriptPath=Join-Path $copyRoot "scripts\games\$game"
        $command=@"
`$env:LOCALAPPDATA='$($userData.Replace("'","''"))'
`$env:ProgramFiles='$($project.Replace("'","''"))\NoDefaultLauncher'
`$env:ProgramW6432=`$env:ProgramFiles
`${env:ProgramFiles(x86)}=`$env:ProgramFiles
& '$($scriptPath.Replace("'","''"))' -DryRun
exit `$LASTEXITCODE
"@
        $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
        $output=& "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -EncodedCommand $encoded 2>&1
        if ($LASTEXITCODE -ne 0) { throw "Relocated DryRun failed ($scenario, $game): $($output | Out-String)" }
        Assert-Test (($output | Out-String) -match '演练模式：环境检查通过') 'DryRun did not reach resource checks.'
    }
    Assert-Test ([IO.File]::ReadAllText($iniPath) -ceq $ini) 'DryRun rewrote configuration.'
    Assert-Test (-not [IO.File]::Exists((Join-Path $copyRoot 'Games\WuWa\Client\Saved\Config\WindowsNoEditor\Engine.ini'))) 'DryRun deployed a game configuration.'
    Assert-Test (-not [IO.File]::Exists((Join-Path $userData 'HT\Saved\Config\Windows\Engine.ini'))) 'DryRun deployed NTE configuration.'
    Write-Host "PASS: both actual game entry scripts relocated under $scenario, with isolated user profile and inert resources."
}

# Import only the GPU functions; never invoke the tool's top-level registry mutations or UI.
$tool=Join-Path $launcherRoot 'scripts\tools\Toggle-NvidiaDeviceName.ps1'
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($tool,[ref]$tokens,[ref]$errors)
foreach ($node in $ast.FindAll({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst]},$true)) {
    if ($node.Name -in @('Split-DeviceDescription','Restore-FromState','Get-FallbackOriginalDescription')) { . ([scriptblock]::Create($node.Extent.Text)) }
}
$plain=Split-DeviceDescription 'NVIDIA GeForce RTX 3060'
Assert-Test ($plain.Prefix -eq '' -and $plain.DisplayName -eq 'NVIDIA GeForce RTX 3060') 'Plain device name unsupported.'
$indirect=Split-DeviceDescription '@oem123.inf,%device%;NVIDIA GeForce RTX 3080 Ti Laptop GPU'
Assert-Test ($indirect.Prefix -eq '@oem123.inf,%device%;') 'Indirect prefix lost.'
foreach ($invalid in @('','@unresolved.inf','@oem.inf;')) {
    $failed=$false; try { Split-DeviceDescription $invalid | Out-Null } catch { $failed=$true }
    Assert-Test $failed 'Malformed device description accepted.'
}
$deviceKeyPath='HKLM:\SYSTEM\CurrentControlSet\Enum\PCI\VEN_10DE&DEV_FAKE\1'
$statePath=Join-Path $testRoot '中文状态.json'
$capture=@{}
function Set-DeviceDescription { param([string]$Value) $capture.Value=$Value }
function Write-Log { param([string]$Message) }
$state=@{KeyPath=$deviceKeyPath;OriginalValue='NVIDIA 原始显示名称';ExpectedValue='测试目标 显卡'}
[IO.File]::WriteAllText($statePath,($state | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
Restore-FromState -CurrentValue '测试目标 显卡'
Assert-Test ($capture.Value -ceq 'NVIDIA 原始显示名称') 'Unicode backup corrupted.'
$state.KeyPath += '-different-device'
[IO.File]::WriteAllText($statePath,($state | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$capture.Clear();$failed=$false
try { Restore-FromState -CurrentValue '测试目标 显卡' } catch { $failed=$true }
Assert-Test ($failed -and -not $capture.ContainsKey('Value') -and [IO.File]::Exists($statePath)) 'Other-device backup was not preserved.'
$classKeyRoot='HKLM:\SYSTEM\CurrentControlSet\Control\Class'
function Get-DeviceProperties { [pscustomobject]@{Driver='{4d36e968-e325-11ce-bfc1-08002be10318}\0042'} }
function Get-ItemProperty { param($LiteralPath,$Name,$ErrorAction) $capture.DriverPath=$LiteralPath; [pscustomobject]@{DriverDesc='NVIDIA actual model'} }
Assert-Test ((Get-FallbackOriginalDescription) -eq 'NVIDIA actual model') 'Driver fallback failed.'
Assert-Test ($capture.DriverPath.EndsWith('{4d36e968-e325-11ce-bfc1-08002be10318}\0042')) 'Driver instance index not followed.'
Write-Host 'PASS: plain/indirect NVIDIA names, Unicode state recovery, changed-device protection and driver instance selection; registry mocked.'
Write-Host "All compatibility checks passed on the current OS. Artifacts: $testRoot"
