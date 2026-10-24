param([string]$Destination, [switch]$Zip, [ValidateSet('0','1','2','3','4')][string]$PackageType='2', [switch]$Interactive, [switch]$b)
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
    Write-Host '  1 完整（通用启动器 + 虚幻启动器 + 鸣潮 + 异环）'
    Write-Host '  2 通用启动器（保留 Engine.ini）'
    Write-Host '  3 虚幻启动器（带画质配置）'
    Write-Host '  4 鸣潮+异环'
    Write-Host '  0 全部依次打包（1 → 2 → 3 → 4）'
    do { $choice=Read-Host '输入数字 1、2、3、4 或 0' } while ($choice -notin @('1','2','3','4','0'))
    $PackageType=$choice
}
if ($PackageType -eq '0') {
    $batchTimestamp=Get-Date -Format 'yyyyMMdd-HHmmss-fff'
    $batchPlans=@()
    foreach ($type in @('1','2','3','4')) {
        $label=@{'1'='Full';'2'='General';'3'='Unreal';'4'='WuWa-NTE'}[$type]
        $path=if ([string]::IsNullOrWhiteSpace($Destination)) {
            Join-Path $project ("dist\DLSSG-$label-$batchTimestamp")
        } else { $Destination.TrimEnd('\','/')+'-'+$label }
        $batchPlans+=@{Type=$type;Destination=[IO.Path]::GetFullPath($path)}
    }
    # Validate every output before starting, so an existing later archive cannot cause a partial batch.
    foreach ($plan in $batchPlans) {
        if (Test-Path -LiteralPath $plan.Destination) { throw "导出位置已存在，整批未开始：$($plan.Destination)" }
        $batchExtension=if ($b) { '.7z' } else { '.zip' }
        if ($Zip -and (Test-Path -LiteralPath ($plan.Destination+$batchExtension))) { throw "压缩包已存在，整批未开始：$($plan.Destination+$batchExtension)" }
    }
    foreach ($plan in $batchPlans) {
        Write-Host ("正在依次打包 [{0}/4]" -f $plan.Type)
        $batchArguments=@{PackageType=$plan.Type;Destination=$plan.Destination;Zip=[bool]$Zip;b=[bool]$b}
        try { & $PSCommandPath @batchArguments }
        catch { throw "第 $($plan.Type) 类打包失败，已停止后续打包；此前成功的包及本次失败现场保留。$($_.Exception.Message)" }
    }
    Write-Output '四类打包全部完成（1 → 2 → 3 → 4）。'
    return
}
$includeGeneral=$PackageType -in @('1','2')
$includeUnreal=$PackageType -in @('1','3')
$includeLauncher=$includeGeneral -or $includeUnreal
$includeGames=$PackageType -in @('1','4')
$packageLabel=@{'1'='Full';'2'='General';'3'='Unreal';'4'='WuWa-NTE'}[$PackageType]
Write-Output ('打包类型：'+@{'1'='完整';'2'='通用启动器';'3'='虚幻启动器';'4'='鸣潮+异环'}[$PackageType])
if ([string]::IsNullOrWhiteSpace($Destination)) {
    $Destination=Join-Path $project ('dist\DLSSG-'+$packageLabel+'-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
}
$destinationPath=[IO.Path]::GetFullPath($Destination)
$folderName=@{'1'='帧生成启动器';'2'='通用启动器';'3'='虚幻启动器';'4'='鸣潮异环帧生成解锁'}[$PackageType]
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
    $files+=@('alternatives\dbghelp.dll','alternatives\dinput8.dll','alternatives\dxgi.dll','alternatives\d3d12.dll',
        '启动脚本\scripts\games\Run-SteamUE5-DLSSG.ps1')
    # PortableLauncher loads UnrealQuality; code dependencies do not require UE configuration files.
    $modules+=@('SteamLauncher','PortableLauncher','DirectLauncher','UnrealQuality')
}
if ($includeGeneral) {
    $files+=@('启动脚本\通用启动器.bat','启动脚本\scripts\ui\Show-DlssgLauncher.ps1')
    $modules+=@('DlssgOnlyLauncher','GameDiscovery')
}
if ($includeUnreal) {
    $files+=@('启动脚本\虚幻启动器.bat','启动脚本\scripts\ui\Show-SteamLauncher.ps1',
        '启动脚本\presets\SteamUE5.json','启动脚本\配置\SteamUE5\Engine.ini')
    foreach ($engine in @('UE4','UE5')) {
        foreach ($tier in @('01-高性能均衡','02-主流均衡','03-性能优先','04-入门流畅','05-自定义')) {
            $files+='启动脚本\配置\虚幻通用分类\'+$engine+'\'+$tier+'\Engine.ini'
        }
    }
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
$files=@($files | Sort-Object -Unique)
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
将整个压缩包解压到可写文件夹，不要在压缩包内运行；保留所有文件的相对位置。
'@
if ($includeGeneral) {
    $guide+="`r`n`r`n通用启动器：`r`n1. 打开 启动脚本\通用启动器.bat，首次列表为空。`r`n2. 发现Steam、Epic、GOG已安装游戏，或手动添加安装目录。`r`n3. 核实真实渲染程序，启动程序可另选；保存设置后检查环境并启动。`r`n此入口保留游戏Engine.ini，保留帧生成及现有INT8超分，不需要画质配置目录。`r`nSteam游戏使用Steam启动，Epic/GOG和手动添加使用EXE启动，仍需满足游戏自身的平台验证。`r`n"
}
if ($includeUnreal) {
    $guide+="`r`n`r`n虚幻启动器：`r`n1. 先原版启动游戏一次，让它创建配置目录，然后退出。`r`n2. 打开 启动脚本\虚幻启动器.bat，内置黑神话：悟空，也可添加Steam或EXE启动游戏。`r`n3. 选择画质档位，检查环境，再启动游戏。此入口临时替换Engine.ini，退出后恢复。`r`nUE4/UE5识别未知时可在高级设置校准；代理和配置源也在高级设置。`r`n提供四档候选及05-自定义，新增游戏默认第二档；实际效果需游戏内确认。`r`n"
}
if ($includeLauncher) {
    $guide+="`r`n同意Windows管理员权限提示后，界面交接并退出；请保留负责等待游戏与恢复的独立脚本窗口。`r`n游戏内按需开启DLSS帧生成，退出后等待恢复完成。Steam可以保持运行。`r`n遗留部署先用原入口恢复，不删除.state或备份。识别成功不代表游戏支持帧生成。`r`n"
}
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
$guide+="`r`n跨设备只传递新压缩包，到新设备重新解压；本机偏好、运行请求、日志和部署状态不随包导出。`r`n不要把使用过的工作目录或旧.state搬到其他设备；迁移前先恢复旧部署。`r`n"
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
