param([Parameter(Mandatory=$true)][string]$Destination, [switch]$Zip)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot '..\common\LauncherConfig.ps1')
$root=Get-LauncherRoot
$project=Split-Path -Parent $root
$destinationPath=[IO.Path]::GetFullPath($Destination)
if (Test-Path -LiteralPath $destinationPath) { throw '导出位置已存在，请换一个新目录；不会覆盖原包。' }
$archive=$destinationPath+'.zip'
if ($Zip -and (Test-Path -LiteralPath $archive)) { throw '目标 ZIP 已存在，请换一个新名字。' }
$files=@('启动脚本\启动器.bat','version.dll','dlssg_sm86.ini','THIRD_PARTY_NOTICES.txt',
    'alternatives\winmm.dll','alternatives\dbghelp.dll','alternatives\dinput8.dll','alternatives\dxgi.dll','alternatives\d3d12.dll',
    'alternatives\README.md',
    'dlssg_sm86\runtime\nvngx_dlssg.dll','dlssg_sm86\backend\sm86_backend.dll','dlssg_sm86\kernels\fg_kernels.dll',
    'dlssg_sm86\xefg\xefg_kernels.dll','dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll',
    '启动脚本\presets\SteamUE5.json','启动脚本\配置\SteamUE5\Engine.ini',
    '启动脚本\scripts\ui\Show-SteamLauncher.ps1','启动脚本\scripts\games\Run-SteamUE5-DLSSG.ps1')
foreach ($engine in @('UE4','UE5')) {
    foreach ($tier in @('01-高性能均衡','02-主流均衡','03-性能优先','04-入门流畅')) {
        $files+='启动脚本\配置\虚幻通用分类\'+$engine+'\'+$tier+'\Engine.ini'
    }
}
$files+=@(Get-ChildItem -LiteralPath (Join-Path $root 'scripts\common') -File -Filter '*.ps1' | ForEach-Object { '启动脚本\scripts\common\'+$_.Name })
foreach ($relative in $files) {
    $source=Join-Path $project $relative
    $item=Get-Item -LiteralPath $source -Force -ErrorAction Stop
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "导出源不能是链接或目录：$relative" }
}
# Active journals/backups must remain on this device. Never copy a working tree wholesale.
[void][IO.Directory]::CreateDirectory($destinationPath)
foreach ($relative in $files) {
    $target=Join-Path $destinationPath $relative
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $target))
    Copy-Item -LiteralPath (Join-Path $project $relative) -Destination $target
}
$guide=@'
DLSSG 便携启动器 · 开始使用

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
四档均为候选，效果需游戏内核对。
“添加其他游戏”需要选择安装文件夹、真实渲染 EXE 和用户配置目录。
AppID 自动读取 Steam 清单；未适配游戏仍需自行验证兼容性。

跨设备：传递本压缩包，到新设备重新解压即可。不传旧设备使用过的整个文件夹。
本机路径偏好放在当前用户 LocalAppData；运行后生成的 .state、日志和备份不能搬作新设备配置。
有问题先点击“查看日志”，把最新日志发给配置提供者。
'@
[IO.File]::WriteAllText((Join-Path $destinationPath '先看这里.txt'),$guide,[Text.UTF8Encoding]::new($true))
if ($Zip) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($destinationPath,$archive,[IO.Compression.CompressionLevel]::Optimal,$true)
    Write-Output "便携 ZIP：$archive"
}
Write-Output "便携目录：$destinationPath"
