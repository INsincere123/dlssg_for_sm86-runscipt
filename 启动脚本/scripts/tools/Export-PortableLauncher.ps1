param([string]$Destination, [switch]$Zip, [ValidateSet('1','2','3')][string]$PackageType='2', [switch]$Interactive, [switch]$b)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
function Remove-PortableStagingDirectory {
    param([string]$Path)
    $resolved=[IO.Path]::GetFullPath($Path).TrimEnd('\')
    if ($resolved -eq [IO.Path]::GetPathRoot($resolved).TrimEnd('\')) { throw '拒绝清理磁盘根目录。' }
    # Inspect each directory before descending; never recursively delete through links.
    $pending=New-Object 'System.Collections.Generic.Stack[string]'
    $pending.Push($resolved)
    while ($pending.Count) {
        $current=$pending.Pop()
        $attributes=[IO.File]::GetAttributes($current)
        if ($attributes -band [IO.FileAttributes]::ReparsePoint) { throw "导出目录出现链接，已保留：$current" }
        if ($attributes -band [IO.FileAttributes]::Directory) {
            foreach ($entry in [IO.Directory]::GetFileSystemEntries($current)) { $pending.Push($entry) }
        }
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction Stop
}
. (Join-Path $PSScriptRoot '..\common\LauncherConfig.ps1')
$root=Get-LauncherRoot
$project=Split-Path -Parent $root
if ($b) {
    $Zip=$true
    $bandizip='C:\Bandizip\bz.exe'
    if (-not [IO.File]::Exists($bandizip)) { throw "找不到 Bandizip 命令行工具：$bandizip" }
}
if ($Interactive -and -not $PSBoundParameters.ContainsKey('PackageType')) {
    Write-Host '选择打包类型：'
    Write-Host '  1 完整（启动器 + 鸣潮 + 异环）'
    Write-Host '  2 启动器（Steam / 直接 EXE）'
    Write-Host '  3 鸣潮+异环'
    do { $choice=Read-Host '输入数字 1、2 或 3' } while ($choice -notin @('1','2','3'))
    $PackageType=$choice
}
$includeLauncher=$PackageType -in @('1','2')
$includeGames=$PackageType -in @('1','3')
$packageLabel=@{'1'='Full';'2'='Launcher';'3'='WuWa-NTE'}[$PackageType]
Write-Output ('打包类型：'+@{'1'='完整';'2'='启动器';'3'='鸣潮+异环'}[$PackageType])
if ([string]::IsNullOrWhiteSpace($Destination)) {
    $Destination=Join-Path $project ('dist\DLSSG-'+$packageLabel+'-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
}
$destinationPath=[IO.Path]::GetFullPath($Destination)
$folderName=if ($PackageType -eq '3') { '鸣潮异环帧生成解锁' } else { '帧生成启动器' }
$packagePath=Join-Path $destinationPath $folderName
if (Test-Path -LiteralPath $destinationPath) { throw '导出位置已存在，请换一个新目录；不会覆盖原包。' }
$extension=if ($b) { '.7z' } else { '.zip' }
$archive=$destinationPath+$extension
if ($Zip -and (Test-Path -LiteralPath $archive)) { throw '目标压缩包已存在，请换一个新名字。' }
$files=@('启动脚本\使用说明.md','version.dll','dlssg_sm86.ini','THIRD_PARTY_NOTICES.txt',
    'alternatives\winmm.dll',
    'alternatives\README.md',
    'dlssg_sm86\runtime\nvngx_dlssg.dll','dlssg_sm86\backend\sm86_backend.dll','dlssg_sm86\kernels\fg_kernels.dll',
    'dlssg_sm86\xefg\xefg_kernels.dll','dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll')
$modules=@('LauncherConfig','ScriptLogging','LauncherConsole','LauncherResources','DeploymentJournal')
if ($includeLauncher) {
    $files+=@('启动脚本\启动器.bat',
    'alternatives\dbghelp.dll','alternatives\dinput8.dll','alternatives\dxgi.dll','alternatives\d3d12.dll',
    '启动脚本\presets\SteamUE5.json','启动脚本\配置\SteamUE5\Engine.ini',
    '启动脚本\scripts\ui\Show-SteamLauncher.ps1','启动脚本\scripts\games\Run-SteamUE5-DLSSG.ps1')
    foreach ($engine in @('UE4','UE5')) {
        foreach ($tier in @('01-高性能均衡','02-主流均衡','03-性能优先','04-入门流畅','05-自定义')) {
            $files+='启动脚本\配置\虚幻通用分类\'+$engine+'\'+$tier+'\Engine.ini'
        }
    }
    $modules+=@('SteamLauncher','PortableLauncher','DirectLauncher','UnrealQuality')
}
if ($includeGames) {
    $files+=@('启动脚本\启动鸣潮.bat','启动脚本\启动异环.bat','启动脚本\切换显卡.bat',
        '启动脚本\scripts\games\Run-WutheringWaves-DLSSG.ps1',
        '启动脚本\scripts\games\Run-NevernessToEverness-DLSSG.ps1',
        '启动脚本\scripts\tools\Toggle-NvidiaDeviceName.ps1',
        '启动脚本\配置\鸣潮\Engine.ini','启动脚本\配置\异环\Engine.ini')
    $modules+=@('LaunchFromBat','LauncherSession')
}
if ($PackageType -eq '1') { $files+='启动脚本\启动SteamUE5.bat' }
# Explicit runtime closure; future common tools are not exported automatically.
foreach ($module in $modules) {
    $files+='启动脚本\scripts\common\'+$module+'.ps1'
}
foreach ($relative in $files) {
    $source=Join-Path $project $relative
    $item=Get-Item -LiteralPath $source -Force -ErrorAction Stop
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "导出源不能是链接或目录：$relative" }
}
# Active journals/backups must remain on this device. Never copy a working tree wholesale.
[void][IO.Directory]::CreateDirectory($packagePath)
foreach ($relative in $files) {
    $target=Join-Path $packagePath $relative
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $target))
    Copy-Item -LiteralPath (Join-Path $project $relative) -Destination $target
}
$guide=@'
DLSSG 便携启动器 · 开始使用

完整操作步骤见“启动脚本/使用说明.md”。

适用于 Windows 10/11 x64，硬件兼容范围沿用 0.4.0（主要为 RTX 20/30 系列）。

1. 将整个压缩包解压到一个可写文件夹。不要在压缩包内直接运行。
2. 打开“启动脚本”文件夹，双击“启动器.bat”。首版内置黑神话：悟空，会自动查找 Steam 安装位置。
3. 点击“启动游戏”，同意 Windows 管理员权限提示。
4. 启动后界面自动退出以释放内存。游戏内按需开启 DLSS 帧生成，退出后等独立脚本恢复完成。

请先从 Steam 原版启动游戏一次，让它创建配置目录。
Steam 可以一直运行。不要强行关闭运行中的脚本窗口。
发现未完成部署时点击“恢复原状”，保留现场和记录，不手动删除备份。
主界面画质按游戏保存；已有游戏保留当前配置，新增游戏默认主流均衡（第二档）。
自动识别UE4/UE5，无法确定时请在高级设置手动校准；代理和自定义INI也在高级设置。
提供四档候选及“05-自定义”；自定义文件可自行编辑，效果需游戏内核对。
“添加其他游戏”可选择Steam或直接EXE方式。
直接EXE方式不需要Steam/AppID，选择安装文件夹、启动EXE、真实渲染EXE与用户配置目录。
请先确认原版EXE能正常启动，游戏自身的平台验证仍需满足。
AppID 自动读取 Steam 清单；未适配游戏仍需自行验证兼容性。

跨设备：传递本压缩包，到新设备重新解压即可。不传旧设备使用过的整个文件夹。
本机路径偏好放在当前用户 LocalAppData；运行后生成的 .state、日志和备份不能搬作新设备配置。
有问题先点击“查看日志”，把最新日志发给配置提供者。
'@
if (-not $includeLauncher) { $guide='' }
if ($includeGames) {
    # Generate a device-neutral template; never export the provider's personal INI.
    $config=@'
; 请先填写本机游戏目录。启动器路径及游戏配置目录可留空自动发现。
; 相对路径以本文件所在目录为基准。不要复制旧电脑的 .state 或显卡备份。
[鸣潮]
游戏目录=C:\Games\Wuthering Waves Game
启动器路径=
配置源=配置\鸣潮\Engine.ini
游戏配置目录=
重启等待秒数=10

[异环]
游戏目录=C:\Games\Neverness To Everness
启动器路径=
配置源=配置\异环\Engine.ini
游戏配置目录=

[显卡]
设备实例标识=
目标显示名称=NVIDIA GeForce RTX 5070
'@
    if ($PackageType -eq '1') {
        $config+="`r`n`r`n[Steam]`r`n启动器路径=`r`n`r`n[SteamUE5:黑神话悟空]`r`n游戏名称=黑神话：悟空`r`n游戏目录=C:\Games\BlackMythWukong`r`n渲染程序=b1\Binaries\Win64\b1-Win64-Shipping.exe`r`n游戏配置目录=%LOCALAPPDATA%\b1\Saved\Config\Windows`r`n配置源=配置\SteamUE5\Engine.ini`r`n代理DLL=1`r`n"
    }
    [IO.File]::WriteAllText((Join-Path $packagePath '启动脚本\启动配置.ini'),$config,[Text.UTF8Encoding]::new($true))
    $guide+="`r`n鸣潮、异环专用入口：`r`n1. 完整解压到本地可写目录。适用于 Windows 10/11 x64、受支持的 RTX 20/30 显卡。`r`n2. 编辑 启动脚本\启动配置.ini，填写本机游戏目录。`r`n3. 关闭游戏和对应官方启动器，双击 启动鸣潮.bat 或 启动异环.bat，同意提权。`r`n4. 从官方启动器进入游戏，退出后等脚本恢复完成，不要强关脚本窗口。`r`n鸣潮默认等待热重启10秒，异环首次退出后恢复。详细恢复命令见使用说明。`r`n切换显卡.bat 为可选工具，仅修改显示名称，不改变实际硬件能力。`r`n迁移前先恢复旧设备部署，不携带旧 .state、日志或备份。`r`n"
}
[IO.File]::WriteAllText((Join-Path $packagePath '先看这里.txt'),$guide,[Text.UTF8Encoding]::new($true))
if ($Zip) {
    if (-not $b) { Add-Type -AssemblyName System.IO.Compression.FileSystem }
    # Publish only complete archives, so failed ZIPs cannot be mistaken for ready packages.
    $temporaryArchive=$archive+'.'+[guid]::NewGuid().ToString('N')+'.tmp'
    try {
        if ($b) {
            Write-Output 'Bandizip：7z / LZMA2（本机版本默认算法）/ 等级9（最高）'
            Push-Location -LiteralPath $destinationPath
            try {
                & $bandizip c -fmt:7z -l:9 -r $temporaryArchive $folderName
                if ($LASTEXITCODE -ne 0) { throw "Bandizip 压缩失败，退出代码：$LASTEXITCODE；已保留导出目录。" }
                & $bandizip t $temporaryArchive
                if ($LASTEXITCODE -ne 0) { throw "Bandizip 完整性检查失败，退出代码：$LASTEXITCODE；已保留导出目录。" }
            } finally { Pop-Location }
        } else {
            [IO.Compression.ZipFile]::CreateFromDirectory($destinationPath,$temporaryArchive,[IO.Compression.CompressionLevel]::Optimal,$false)
        }
        [IO.File]::Move($temporaryArchive,$archive)
    } finally {
        if ([IO.File]::Exists($temporaryArchive)) { [IO.File]::Delete($temporaryArchive) }
    }
    Write-Output "便携压缩包：$archive"
    Write-Output ('压缩包大小：{0:N2} MiB' -f ((Get-Item -LiteralPath $archive).Length/1MB))
    $hashStream=[IO.File]::OpenRead($archive)
    $sha=[Security.Cryptography.SHA256]::Create()
    try { $archiveHash=([BitConverter]::ToString($sha.ComputeHash($hashStream))).Replace('-','') }
    finally { $sha.Dispose(); $hashStream.Dispose() }
    Write-Output ('SHA256：'+$archiveHash)
    Remove-PortableStagingDirectory -Path $destinationPath
    Write-Output '已删除打包临时目录，仅保留压缩包。'
    Write-Output '将这个压缩包发给朋友；完整解压后阅读“先看这里.txt”。'
}
Write-Output ('运行文件：{0} 个（含说明及生成的配置）' -f ($files.Count+1+[int]$includeGames))
if (-not $Zip) { Write-Output "便携目录：$packagePath" }
