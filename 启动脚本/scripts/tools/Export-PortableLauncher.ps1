param([string]$Destination, [switch]$Zip, [ValidateSet('0','1','2','3','4')][string]$PackageType='2', [switch]$Interactive, [switch]$b)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
function Get-PortableFileHash {
    param([string]$Path)
    $stream=[IO.File]::OpenRead($Path); $sha=[Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-','') }
    finally { $sha.Dispose(); $stream.Dispose() }
}

function Assert-PortableZipArchive {
    param([string]$Path,[string]$StagingDirectory)
    # Verify every decompressed file, without extracting or trusting archive paths.
    $prefix=[IO.Path]::GetFullPath($StagingDirectory).TrimEnd('\')+'\'
    $expected=@{}
    foreach ($file in @(Get-ChildItem -LiteralPath $StagingDirectory -Recurse -File -Force)) {
        $relative=$file.FullName.Substring($prefix.Length).Replace('\','/')
        $expected[$relative]=@{Length=$file.Length;SHA256=(Get-PortableFileHash $file.FullName)}
    }
    $zip=[IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $seen=@{}
        foreach ($entry in $zip.Entries) {
            $name=$entry.FullName.Replace('\','/')
            if ($name.EndsWith('/')) { throw "ZIP含未预期的目录条目：$name" }
            if (-not $expected.ContainsKey($name) -or $seen.ContainsKey($name)) { throw "ZIP条目缺失对应源或重复：$name" }
            if ($entry.Length -ne $expected[$name].Length) { throw "ZIP文件大小不符：$name" }
            $stream=$entry.Open(); $sha=[Security.Cryptography.SHA256]::Create()
            try { $hash=([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-','') }
            finally { $sha.Dispose(); $stream.Dispose() }
            if ($hash -ne $expected[$name].SHA256) { throw "ZIP文件SHA256不符：$name" }
            $seen[$name]=$true
        }
        if ($seen.Count -ne $expected.Count) { throw 'ZIP文件数量不符，有文件遗漏。' }
    } finally { $zip.Dispose() }
}

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
$files=@('version.dll','dlssg_sm86.ini','THIRD_PARTY_NOTICES.txt','dlssg_sm86ini_Configurator.html',
    'alternatives\winmm.dll',
    'alternatives\README.md',
    'dlssg_sm86\runtime\nvngx_dlssg.dll','dlssg_sm86\backend\sm86_backend.dll','dlssg_sm86\kernels\fg_kernels.dll',
    'dlssg_sm86\xefg\xefg_kernels.dll','dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll')
$modules=@('LauncherConfig','ScriptLogging','LauncherConsole','LauncherResources','DeploymentJournal')
if ($includeLauncher) {
    $files+=@('alternatives\dbghelp.dll','alternatives\dinput8.dll','alternatives\dxgi.dll','alternatives\d3d12.dll',
        '启动脚本\scripts\games\Run-SteamUE5-DLSSG.ps1')
    # PortableLauncher loads UnrealQuality; code dependencies do not require UE configuration files.
    $modules+=@('SteamLauncher','PortableLauncher','DirectLauncher','UnrealQuality','GameDiscovery')
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
    if ($relative -in @('dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll') -and -not [IO.File]::Exists($source)) {
        throw "缺少本地SR资源：$relative。Git不提供这两个大文件，请从匹配的完整发行包或本地备份补齐后重试；超分Off也保留完整资源布局。"
    }
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
DLSSG 便携启动器 · 开始使用（小白版）

这是一份完整的操作说明，照着做就行，不用懂技术。

组件基线：0.4.2；适用环境：Windows 10/11 64 位，NVIDIA RTX 20/30 系列显卡，驱动R580或更新。
包内 dlssg_sm86ini_Configurator.html 是单文件 INI 配置器，可离线查看和生成配置；不改它也不影响启动。
启动脚本默认Preset=A、超分Mode=Off；通用与虚幻启动器可在高级设置按游戏选择超分模式、A/B/C预设、Optimized和Level（均默认2），并在本机记住上次选择的游戏；Off只关闭本项目超分接管，游戏自己的DLSS超分仍在游戏内设置，无需调整英伟达控制面板。
运行后组件日志优先看游戏渲染目录下 dlssg_sm86\logs\dlssg_sm86.log，结合脚本日志和本次游戏PID排查。
包内 package-manifest.json 列出相对路径、字节数和SHA256，便于核对文件；哈希一致不等于组件已经验证兼容。
解压要求：把整个压缩包完整解压到电脑上的普通文件夹（例如 D:\游戏工具），
不要在压缩包里面直接运行；解压后整个文件夹一起用，不要单独移动里面的文件。
'@
if ($includeGeneral) {
    $guide+="`r`n`r`n■ 通用启动器（不动游戏画面设置，主要用于开 DLSS 帧生成）`r`n第一次使用：`r`n1. 双击 启动脚本\通用启动器.bat，首次游戏列表是空的，正常。`r`n2. 点「发现已安装游戏」，从 Steam、Epic、GOG 安装记录里勾选游戏；找不到就点「手动添加游戏」，选择游戏安装文件夹。`r`n3. 核对列表里的「渲染程序」：通常选游戏安装目录 Binaries\Win64 里以 Shipping.exe 结尾的那个 EXE，候选只是建议，不确定时核对游戏目录和实际程序，不要盲选。`r`n4. Steam 游戏用「Steam 启动」；Epic、GOG 和手动添加的用「EXE 启动」，仍需满足游戏自身的登录/授权。`r`n5. 点「保存设置」→ 可先点「检查环境」→ 点「启动游戏」，弹窗点「是」同意管理员权限。`r`n6. 界面会自动退出，黑色脚本窗口会留下继续工作，不要关它。`r`n7. 进游戏后，在画面设置里打开「DLSS 帧生成」。`r`n8. 退出游戏后，等黑色窗口自动清理完成并退出。`r`n提示：此入口不修改游戏 Engine.ini，不需要画质配置；游戏搬盘后重新用「选择当前游戏文件夹」指定位置。`r`n"
}
if ($includeUnreal) {
    $guide+="`r`n`r`n■ 虚幻启动器（自动套用画质配置，适合 UE4/UE5 游戏）`r`n第一次使用：`r`n1. 先原版启动一次游戏，进入后正常退出，让游戏生成配置文件。`r`n2. 双击 启动脚本\虚幻启动器.bat，内置黑神话：悟空，也可在「添加其他游戏」中扫描Steam/Epic/GOG安装记录或手动选安装目录。渲染程序和Engine.ini配置目录唯一可信时自动采用；未找到或有歧义时提示选择，可取消扫描并查看扫描详情。`r`n3. 选好游戏和画质档位，可先点「检查环境」，再点「启动游戏」，弹窗点「是」同意管理员权限。`r`n4. 界面自动退出，黑色脚本窗口继续工作，不要关它。`r`n5. 进游戏后在画面设置里打开「DLSS 帧生成」。`r`n6. 退出游戏后等脚本自动恢复原画面配置并退出。`r`n怎么选画质：新增游戏默认「主流均衡」，从这档开始试；卡顿就换「性能优先」或「入门流畅」，想更精细换「高性能均衡」；「05-自定义」用自己准备的配置。切档前先退出游戏并等恢复完成。`r`nUE4/UE5 识别不了时：打开 高级设置 → 引擎版本，手动选对后点保存。`r`n"
}
if ($includeLauncher) {
    $guide+="`r`n通用提示：同意管理员权限后界面会交接退出，请保留负责等待游戏与恢复的黑色脚本窗口；Steam 可以保持运行。识别到游戏不代表它一定支持帧生成，以游戏内是否有「DLSS 帧生成」选项为准。遗留部署先用原入口「恢复原状」，不要删除 .state 或备份记录绕过。`r`n"
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
; 超分 Off/M-INT8/L-INT8/M-FP16/L-FP16；DLSSG预设 A/B/C。
; 优化档位0原厂/1逐位一致/2快（有损）/3最快（有损）；日志0关闭/1错误/2配置与能力/3详细追踪。
SuperResolutionMode=Off
DlssgPreset=A
Optimized=2
Level=2

[异环]
游戏目录=C:\Games\Neverness To Everness
启动器路径=
配置源=配置\异环\Engine.ini
游戏配置目录=

; 超分 Off/M-INT8/L-INT8/M-FP16/L-FP16；DLSSG预设 A/B/C。
; 优化档位0原厂/1逐位一致/2快（有损）/3最快（有损）；日志0关闭/1错误/2配置与能力/3详细追踪。
SuperResolutionMode=Off
DlssgPreset=A
Optimized=2
Level=2

[显卡]
设备实例标识=
目标显示名称=NVIDIA GeForce RTX 5070
'@
    if ($PackageType -eq '1') {
        $config+="`r`n`r`n[Steam]`r`n启动器路径=`r`n`r`n[SteamUE5:黑神话悟空]`r`n游戏名称=黑神话：悟空`r`n游戏目录=C:\Games\BlackMythWukong`r`n渲染程序=b1\Binaries\Win64\b1-Win64-Shipping.exe`r`n游戏配置目录=%LOCALAPPDATA%\b1\Saved\Config\Windows`r`n配置源=配置\SteamUE5\Engine.ini`r`n代理DLL=1`r`n"
    }
    [IO.File]::WriteAllText((Join-Path $packagePath '启动脚本\启动配置.ini'),$config,[Text.UTF8Encoding]::new($true))
    $guide+="`r`n`r`n■ 鸣潮 / 异环专用入口（开 DLSS 帧生成 + 自动套用画面优化）`r`n第一次使用（只做一次）：`r`n1. 改游戏路径：用记事本打开 启动脚本\启动配置.ini，把 [鸣潮] 和 [异环] 两段里的「游戏目录」改成你自己电脑上游戏的安装位置（不知道在哪：打开官方启动器，在设置里查看安装位置）；「启动器路径」留空，脚本会自动找到，改完保存。`r`n2. 先检查游戏内是否已有DLSS帧生成选项；若仍因显卡识别门槛隐藏选项，可按需尝试 启动脚本\切换显卡.bat，同意管理员授权后重启电脑。`r`n注意：显卡名称工具是可选兼容措施，已有选项时无需使用；只改显示名称，不保证游戏一定开放选项或支持帧生成。已切换后想还原，再运行该工具并按提示重启。`r`n每次怎么玩：`r`n1. 先彻底关闭游戏和官方启动器；`r`n2. 玩鸣潮双击 启动脚本\启动鸣潮.bat，玩异环双击 启动脚本\启动异环.bat；`r`n3. 弹窗点「是」，同意管理员权限；`r`n4. 脚本会自动打开官方启动器，之后不用管脚本窗口；`r`n5. 正常开始游戏或更新，不影响更新；`r`n6. 进游戏后，在画面设置里打开「帧生成」（即 DLSS 帧生成）；`r`n7. 退出游戏后别急着关窗口，等脚本自动清理完成（鸣潮会多等约 10 秒）。`r`n出问题怎么办：`r`n- 双击没反应或报错：再双击一次；仍失败把 启动脚本\logs 里的日志发给 AI 分析。`r`n- 游戏异常退出（强退、断电）后：关闭游戏和官方启动器，在 启动脚本 文件夹打开 PowerShell，输入 & '.\启动鸣潮.bat' -RestoreDeployment（异环换成 启动异环），回车即可恢复。`r`n切换显卡.bat 是可选工具，仅修改显示名称，不改变实际硬件能力；迁移前先恢复旧设备部署，不携带旧 .state、日志或备份。`r`n"
    $guide+="`r`n高级设置：在 启动脚本\启动配置.ini 的 [鸣潮]、[异环] 内分别修改 SuperResolutionMode、DlssgPreset、Optimized、Level，默认 Off / A / 2 / 2；命令行同名参数优先于INI。超分 Off 保留游戏自身DLSS；其他模式需要配套资源。Level=0 不生成组件日志，排查问题可设为2。`r`n"
}
$guide+="`r`n`r`n■ 遇到问题怎么办`r`n- 游戏进不去或没有帧生成选项：回头核对对应入口的「第一次使用」步骤有没有漏做；显卡名称工具仅在确有识别门槛时按需使用。`r`n- 报错、闪退：把 启动脚本\logs 文件夹里的日志发给豆包等 AI 分析，这是最快的排查方式。`r`n- 上次断电、崩溃或脚本被结束：关闭游戏，重新打开对应 BAT，点「恢复原状」。`r`n`r`n■ 别动这些`r`n- 根目录的 version.dll、「dlssg_sm86」文件夹、「dlssg_sm86.ini」：脚本靠它们工作，不要删除、改名或移动。`r`n- 整个文件夹要一起复制、一起移动，不要只拷某一个 BAT。`r`n- 不要删除 .state 或备份记录来绕过恢复；恢复失败时保留现场，把最新日志交给提供者。`r`n`r`n■ 换电脑或发给朋友`r`n- 只传新解压出来的这份文件夹/压缩包；本机偏好、日志、部署状态不会打包进去。`r`n- 不要把使用过的旧目录、旧 .state 或旧电脑的运行记录带到新设备；迁移前先恢复旧部署。`r`n- 换到新电脑后重新解压，再按上面对应入口的「第一次使用」操作。`r`n"
[IO.File]::WriteAllText((Join-Path $packagePath '先看这里.txt'),$guide,[Text.UTF8Encoding]::new($true))
# Include generated guide/config as well as copied resources. The manifest excludes itself.
$manifestFiles=@()
$packagePrefix=[IO.Path]::GetFullPath($packagePath).TrimEnd('\')+'\'
foreach ($file in @(Get-ChildItem -LiteralPath $packagePath -Recurse -File | Sort-Object FullName)) {
    $manifestFiles += [pscustomobject]@{Path=$file.FullName.Substring($packagePrefix.Length).Replace('\','/');
        Length=$file.Length;SHA256=(Get-PortableFileHash $file.FullName)}
}
$manifest=[ordered]@{FormatVersion=1;PackageType=$PackageType;Files=@($manifestFiles)}
[IO.File]::WriteAllText((Join-Path $packagePath 'package-manifest.json'),($manifest | ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($true))
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
            Assert-PortableZipArchive -Path $temporaryArchive -StagingDirectory $destinationPath
            Write-Output 'ZIP逐文件大小与SHA256校验通过。'
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
Write-Output ('运行文件：{0} 个（含说明、清单及生成的配置）' -f ($files.Count+2+[int]$includeGames))
if (-not $Zip) { Write-Output "便携目录：$packagePath" }
