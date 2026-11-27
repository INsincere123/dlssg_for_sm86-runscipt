#requires -Version 5.1
<#
.SYNOPSIS
    打包全量版、轻量版或超分补丁。

.DESCRIPTION
    正式包布局（exe 在根，DLL 与 INI 进 bin\）：
        DlssgLauncher.exe
        bin\dlssg_sm86.ini
        bin\version.dll
        bin\alternatives\*.dll
        bin\dlssg_sm86\{runtime,backend,kernels,xefg}\*.dll
        bin\dlssg_sm86\sr\*.dll            ← 轻量版没有这两个
        配置\虚幻通用分类\UE{4,5}\{01..05}\Engine.ini
        配置\鸣潮\Engine.ini
        配置\异环\Engine.ini
        README.txt
        package-manifest.json

    轻量版排除 sr 两个文件（约 172 MB）。全量版和超分补丁使用启动器共用的
    超分清单核验必需文件，临时产物通过校验后再发布。

.PARAMETER Mode
    full（默认）、light 或 sr（超分补丁）。

.PARAMETER Output
    输出目录，默认 <仓库>\rust\dist。实际包目录是 <Output>\DlssgLauncher-<版本>-<模式>。

.PARAMETER Compress
    none（默认，只出目录）/ zip（deflate，逐文件校验）/ max（极限压缩）。
    max 走 Bandizip 的 7z + LZMA2 等级 9，与旧脚本 `Export-PortableLauncher.ps1 -b`
    使用相同的压缩设置，压缩后运行完整性检查。找不到 Bandizip 时依次改用
    7z.exe、tar + xz。全量版和超分补丁还会从归档提取两个 DLL 核对 SHA256。

.PARAMETER Zip
    老参数，等价于 -Compress zip（保留兼容）。

.PARAMETER Force
    校验通过后替换已有目标目录。
#>
[CmdletBinding()]
param(
    [ValidateSet('full', 'light', 'sr')]
    [string]$Mode = 'full',
    [string]$Output,
    [ValidateSet('none', 'zip', 'max')]
    [string]$Compress = 'none',
    [switch]$ArchiveOnly,
    [switch]$Zip,
    [switch]$Force
)
if ($Zip -and $Compress -eq 'none') { $Compress = 'zip' }

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$rustRoot = Join-Path $repoRoot 'rust'
$exe = Join-Path $rustRoot 'target\release\DlssgLauncher.exe'
if (-not $Output) { $Output = Join-Path $rustRoot 'dist' }

function Fail([string]$message) {
    Write-Host "打包失败：$message" -ForegroundColor Red
    exit 1
}

function Copy-Tree([string]$from, [string]$to, [string]$filter) {
    if (-not (Test-Path -LiteralPath $from)) { Fail "缺少来源：$from" }
    New-Item -ItemType Directory -Path $to -Force | Out-Null
    if ($filter) { $items = @(Get-ChildItem -LiteralPath $from -File -Recurse -Filter $filter) } else { $items = @(Get-ChildItem -LiteralPath $from -File -Recurse) }
    foreach ($item in $items) {
        $relative = $item.FullName.Substring($from.Length).TrimStart('\')
        $target = Join-Path $to $relative
        $parent = Split-Path -Parent $target
        if (-not (Test-Path -LiteralPath $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
        Copy-Item -LiteralPath $item.FullName -Destination $target -Force
    }
}

$srList = Join-Path $rustRoot 'crates\launcher-core\src\resources\super-resolution-required.txt'
$srRequired = @(Get-Content -LiteralPath $srList -Encoding UTF8 | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($srRequired.Count -eq 0) { Fail "超分资源清单为空：$srList" }
$requiresSr = $Mode -ne 'light'

function Assert-SrFiles([string]$root, [string]$description) {
    $missing = @(foreach ($relative in $srRequired) {
        $path = Join-Path $root $relative
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { $path }
    })
    if ($missing.Count -gt 0) { Fail "$description 缺少超分资源：$($missing -join '、')" }
}

function Copy-SrFiles([string]$bin) {
    foreach ($relative in $srRequired) {
        $target = Join-Path $bin $relative
        [IO.Directory]::CreateDirectory((Split-Path -Parent $target)) | Out-Null
        Copy-Item -LiteralPath (Join-Path $repoRoot $relative) -Destination $target
    }
}

# 错误应发生在创建产物或替换旧包之前。
if ($requiresSr) { Assert-SrFiles $repoRoot '源目录' }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { Fail "没有发布版 exe：$exe（先 cargo build -p launcher --release）" }

# 版本：优先问 exe 自己，问不到再看 Cargo.toml
$version = $null
$probe = Join-Path ([System.IO.Path]::GetTempPath()) ("dlssg-pack-version-{0}.json" -f $PID)
try {
    $process = Start-Process -FilePath $exe -ArgumentList @('--version', '--report', ('"' + $probe + '"')) -WindowStyle Hidden -PassThru -Wait
    if (Test-Path -LiteralPath $probe) {
        $parsed = Get-Content -LiteralPath $probe -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($parsed.version) { $version = $parsed.version }
    }
}
catch {
    Write-Host "问版本失败（继续用 Cargo.toml）：$($_.Exception.Message)" -ForegroundColor Yellow
}
finally {
    Remove-Item -LiteralPath $probe -Force -ErrorAction SilentlyContinue
}
if (-not $version) {
    $cargo = Get-Content -LiteralPath (Join-Path $rustRoot 'crates\launcher\Cargo.toml') -Raw -Encoding UTF8
    if ($cargo -match '(?m)^version\s*=\s*"([^"]+)"') { $version = $Matches[1] }
}
if (-not $version) { Fail '读不到版本号' }
$Output = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
$display = 'v' + ($version -replace '\.0$', '')

if ($Mode -eq 'sr') { $name = "DlssgLauncher-$display-sr-patch" } else { $name = "DlssgLauncher-$display-$Mode" }

function Assert-OutputChild([string]$path, [string]$parent) {
    $fullPath = [IO.Path]::GetFullPath($path)
    if ([IO.Path]::GetDirectoryName($fullPath).TrimEnd('\') -ne $parent.TrimEnd('\')) {
        throw "打包路径超出预期目录：$path"
    }
    $item = Get-Item -LiteralPath $fullPath -Force -ErrorAction SilentlyContinue
    if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "打包路径是链接或联接点：$path"
    }
}

$destination = Join-Path $Output $name
Assert-OutputChild $destination $Output
if ((Test-Path -LiteralPath $destination) -and -not $Force) {
    Fail "目标已存在：$destination（要覆盖请加 -Force）"
}
$workRoot = Join-Path $Output ('.dlssg-pack-' + [guid]::NewGuid().ToString('N'))
$packageRoot = Join-Path $workRoot $name
Assert-OutputChild $workRoot $Output
Assert-OutputChild $packageRoot $workRoot
[IO.Directory]::CreateDirectory($packageRoot) | Out-Null
try {

if ($Mode -eq 'sr') {
    Copy-SrFiles (Join-Path $packageRoot 'bin')
    $baseName = "DlssgLauncher-$display-light"
    $srFileList = ($srRequired | ForEach-Object { '  bin\' + $_.Replace('/', '\') }) -join [Environment]::NewLine
    $patchReadme = @"
超分补丁（DLSSG 启动器 $display）

这是给「轻量版」补超分内核用的补丁，两个文件合计约 172 MB：
$srFileList

怎么用
  1. 先解压 DlssgLauncher-$display-light。
  2. 把本补丁里的 bin\ 目录覆盖到启动器根目录（与 DlssgLauncher.exe 同级）。
  3. 重新打开启动器，「帧生成」里的 SuperResolution.Mode 就能选 M/L 档了。
  补完之后的目录与「全量版」等价（同样 27 个文件）。

校验
  package-manifest.json 里有两个文件的 SHA256；补丁包内容与全量版的 sr 完全一致。
"@
    $patchReadmePath = Join-Path $packageRoot 'README-超分补丁.txt'
    [System.IO.File]::WriteAllText($patchReadmePath, ($patchReadme -replace "`r`n", "`n" -replace "`n", "`r`n"), (New-Object System.Text.UTF8Encoding($true)))
    Write-Host "超分补丁：按必需清单复制文件（合并目标 $baseName）" -ForegroundColor Cyan
}
else {

# 1) exe 在根
Copy-Item -LiteralPath $exe -Destination (Join-Path $packageRoot 'DlssgLauncher.exe') -Force

# 2) 负载进 bin\
$bin = Join-Path $packageRoot 'bin'
New-Item -ItemType Directory -Path $bin -Force | Out-Null
foreach ($file in 'dlssg_sm86.ini', 'version.dll') {
    $source = Join-Path $repoRoot $file
    if (-not (Test-Path -LiteralPath $source)) { Fail "缺少负载：$source" }
    Copy-Item -LiteralPath $source -Destination (Join-Path $bin $file) -Force
}
Copy-Tree (Join-Path $repoRoot 'alternatives') (Join-Path $bin 'alternatives') '*.dll'
foreach ($directory in 'runtime', 'backend', 'kernels', 'xefg') {
    Copy-Tree (Join-Path $repoRoot "dlssg_sm86\$directory") (Join-Path $bin "dlssg_sm86\$directory") '*.dll'
}
if ($Mode -eq 'full') {
    Copy-SrFiles $bin
}
else {
    Write-Host '轻量版：跳过 dlssg_sm86\sr（超分内核约 172 MB）' -ForegroundColor Yellow
}

# 3) 配置负载（只带 12 个档位 INI 与两款游戏正式源）
$configSource = Join-Path $repoRoot '启动脚本\配置'
if (-not (Test-Path -LiteralPath $configSource)) { Fail "缺少配置来源：$configSource" }
$config = Join-Path $packageRoot '配置'
foreach ($family in 'UE4', 'UE5') {
    Copy-Tree (Join-Path $configSource "虚幻通用分类\$family") (Join-Path $config "虚幻通用分类\$family") 'Engine.ini'
}
foreach ($game in '鸣潮', '异环') {
    $source = Join-Path $configSource "$game\Engine.ini"
    if (-not (Test-Path -LiteralPath $source)) { Fail "缺少配置源：$source" }
    $target = Join-Path $config "$game"
    New-Item -ItemType Directory -Path $target -Force | Out-Null
    Copy-Item -LiteralPath $source -Destination (Join-Path $target 'Engine.ini') -Force
}

# 4) 人读的说明
$readme = @"
DLSSG 启动器 $display（$Mode）

用法
  1. 双击 DlssgLauncher.exe。程序会请求管理员权限（部署要用符号链接）。
  2. 点「扫描游戏」自动找已安装的虚幻引擎游戏，或「添加游戏」手动选渲染 EXE。
  3. 选中一行后按「启动」：程序会把 dlssg_sm86 组件与代理 DLL 链接进游戏目录，
     再拉起游戏；游戏退出后自动恢复原样。
  4. 游戏在跑时按钮变成「结束并恢复」：强行结束相关进程并恢复。

目录
  bin\      组件与代理 DLL（不要移动或改名）
  配置\     画质档位与两款游戏的配置源；「画质」里选档位时从这里取
  logs\     运行日志与结果报告（超过 7 天的 .log 会自动清理）
  .state\   部署记录（游戏在跑时会存在，恢复完成后自动删除）

这一份是$([string]::Format('{0}', $(if ($Mode -eq 'light') { '轻量版' } else { '全量版' })))
$(if ($Mode -eq 'light') { @"
  不含超分内核（bin\dlssg_sm86\sr）。帧生成的其它功能都可用，
  SuperResolution.Mode 只能用 Off；要用超分请换全量版。
"@ } else { @"
  含超分内核，SuperResolution.Mode 可选 M/L 档。
"@ })

系统要求
  Windows 10/11 x64；需要 Microsoft Visual C++ 2015-2022 运行库（多数机器已有）。

出问题看哪里
  logs\ 下最新的 engine_*.log 与 ui-*.json；界面「关于」里也有日志目录入口。
"@
$readmePath = Join-Path $packageRoot 'README.txt'
[System.IO.File]::WriteAllText($readmePath, ($readme -replace "`r`n", "`n" -replace "`n", "`r`n"), (New-Object System.Text.UTF8Encoding($true)))

} # else：完整包（full / light）

$packageBin = Join-Path $packageRoot 'bin'
if ($requiresSr) { Assert-SrFiles $packageBin '包目录' }

# 5) 清单：每个文件的大小与 SHA256（补丁包也走同一套）
$files = Get-ChildItem -LiteralPath $packageRoot -File -Recurse | Sort-Object FullName
$rows = @()
$total = 0
foreach ($file in $files) {
    $relative = $file.FullName.Substring($packageRoot.Length).TrimStart('\')
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $rows += [pscustomobject]@{ path = $relative; bytes = $file.Length; sha256 = $hash }
    $total += $file.Length
}
$srEntries = @{}
if ($requiresSr) {
    foreach ($relative in $srRequired) {
        $entry = 'bin\' + $relative.Replace('/', '\')
        $listed = @($rows | Where-Object { $_.path -eq $entry })
        if ($listed.Count -ne 1) { Fail "打包清单缺少或重复列出超分资源：$entry" }
        $srEntries[$entry] = $listed[0].sha256
    }
}
$manifest = [pscustomobject]@{
    package = $name
    version = $version
    mode    = $Mode
    layout  = if ($Mode -eq 'sr') { 'patch over a light package: bin/ only' } else { 'exe at root, payload under bin/' }
    appliesTo = if ($Mode -eq 'sr') { "DlssgLauncher-$display-light" } else { $null }
    created = (Get-Date).ToString('yyyy-MM-ddTHH:mm:ssK')
    totals  = [pscustomobject]@{ files = $rows.Count; bytes = $total }
    files   = $rows
}
$manifestPath = Join-Path $packageRoot 'package-manifest.json'
$manifestJson = $manifest | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText($manifestPath, $manifestJson, (New-Object System.Text.UTF8Encoding($false)))

# 6) 压缩（可选）：先写临时档，校验通过再改名，失败的包不能看起来像成品
function Test-ZipEntries([string]$archivePath) {
    # 直接和磁盘上的包目录比对：清单里不能写自己的哈希，逐文件核对要落在目录上
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $expected = @{}
    foreach ($file in Get-ChildItem -LiteralPath $packageRoot -File -Recurse) {
        $key = $file.FullName.Substring($packageRoot.Length).TrimStart('\').Replace('\', '/')
        $expected[$key] = $file
    }
    foreach ($entry in $srEntries.Keys) {
        if (-not $expected.ContainsKey($entry.Replace('\', '/'))) { Fail "压缩包缺少超分条目：$entry" }
    }
    $reader = [System.IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        foreach ($entry in $reader.Entries) {
            if ($entry.Name -eq '') { continue }
            # 压缩包条目可能用反斜杠，统一成 / 再比
            $key = $entry.FullName.Replace('\', '/')
            if (-not $expected.ContainsKey($key)) { Fail "压缩包条目多出或重复：$key" }
            if ($entry.Length -ne $expected[$key].Length) { Fail "压缩包条目大小不符：$key" }
            $stream = $entry.Open()
            try {
                $sha = [System.Security.Cryptography.SHA256]::Create()
                try { $digest = -join ($sha.ComputeHash($stream) | ForEach-Object { $_.ToString('x2') }) }
                finally { $sha.Dispose() }
            }
            finally { $stream.Dispose() }
            $srEntry = $key.Replace('/', '\')
            if ($srEntries.ContainsKey($srEntry)) {
                if ($digest -ne $srEntries[$srEntry]) { Fail "压缩包超分条目 SHA256 不符：$key" }
            }
            else {
                $onDisk = (Get-FileHash -LiteralPath $expected[$key].FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($digest -ne $onDisk) { Fail "压缩包条目 SHA256 与目录不符：$key" }
            }
            $expected.Remove($key)
        }
    }
    finally { $reader.Dispose() }
    if ($expected.Count -gt 0) { Fail "压缩包缺少条目：$($expected.Keys -join '、')" }
}

$finalPath = $null
$tool = $null
if ($Compress -ne 'none') {
    if ($requiresSr -and $Compress -eq 'max') {
        $verifyRoot = Join-Path $workRoot 'archive-check'
        [IO.Directory]::CreateDirectory($verifyRoot) | Out-Null
        $archiveMembers = @($srRequired | ForEach-Object { "$name/bin/$_" })
    }
    $bandizip = 'C:\Bandizip\bz.exe'
    $sevenZip = @(
        'C:\Program Files\7-Zip\7z.exe',
        'C:\Program Files (x86)\7-Zip\7z.exe'
    ) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1

    if ($Compress -eq 'zip') {
        $staging = Join-Path $workRoot ($name + '.zip')
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        [System.IO.Compression.ZipFile]::CreateFromDirectory($packageRoot, $staging, [System.IO.Compression.CompressionLevel]::Optimal, $false)
        Test-ZipEntries $staging
        $finalPath = Join-Path $Output ($name + '.zip')
        $tool = 'zip / Deflate(Optimal)，逐条目大小与 SHA256 已核对'
    }
    elseif ((Test-Path -LiteralPath $bandizip)) {
        # 与旧脚本 Export-PortableLauncher.ps1 -b 完全一致：7z + LZMA2，等级 9（最高）
        $staging = Join-Path $workRoot ($name + '.7z')
        Push-Location $workRoot
        try {
            & $bandizip c -fmt:7z -l:9 -r $staging (Split-Path -Leaf $packageRoot) | Out-Null
            if ($LASTEXITCODE -ne 0) { Fail "Bandizip 压缩失败，退出代码：$LASTEXITCODE" }
            & $bandizip t $staging | Out-Null
            if ($LASTEXITCODE -ne 0) { Fail "Bandizip 完整性检查失败，退出代码：$LASTEXITCODE" }
            if ($requiresSr) {
                $windowsMembers = @($archiveMembers | ForEach-Object { $_.Replace('/', '\') })
                & $bandizip x -y "-o:$verifyRoot" $staging @windowsMembers | Out-Null
                if ($LASTEXITCODE -ne 0) { Fail "Bandizip 无法提取必需超分条目，退出代码：$LASTEXITCODE" }
            }
        }
        finally { Pop-Location }
        $finalPath = Join-Path $Output ($name + '.7z')
        $tool = '7z / LZMA2 等级 9（Bandizip 极限压缩），CRC 已校验'
    }
    elseif ($sevenZip) {
        Write-Host '找不到 Bandizip，极限压缩改用 7-Zip' -ForegroundColor Yellow
        $staging = Join-Path $workRoot ($name + '.7z')
        Push-Location $workRoot
        try {
            & $sevenZip a -t7z -mx=9 -r $staging (Split-Path -Leaf $packageRoot) | Out-Null
            if ($LASTEXITCODE -ne 0) { Fail "7-Zip 压缩失败，退出代码：$LASTEXITCODE" }
            & $sevenZip t $staging | Out-Null
            if ($LASTEXITCODE -ne 0) { Fail "7-Zip 完整性检查失败，退出代码：$LASTEXITCODE" }
            if ($requiresSr) {
                $windowsMembers = @($archiveMembers | ForEach-Object { $_.Replace('/', '\') })
                & $sevenZip x -y "-o$verifyRoot" $staging @windowsMembers | Out-Null
                if ($LASTEXITCODE -ne 0) { Fail "7-Zip 无法提取必需超分条目，退出代码：$LASTEXITCODE" }
            }
        }
        finally { Pop-Location }
        $finalPath = Join-Path $Output ($name + '.7z')
        $tool = '7z / LZMA2 -mx=9（7-Zip 命令行），CRC 已校验'
    }
    else {
        Write-Host '找不到 Bandizip / 7-Zip，极限压缩退回 tar + xz' -ForegroundColor Yellow
        $staging = Join-Path $workRoot ($name + '.tar.xz')
        & tar -c -J -f $staging -C $workRoot (Split-Path -Leaf $packageRoot)
        if ($LASTEXITCODE -ne 0) { Fail "tar 压缩失败，退出代码：$LASTEXITCODE" }
        & tar -t -f $staging | Out-Null
        if ($LASTEXITCODE -ne 0) { Fail "tar 完整性检查失败，退出代码：$LASTEXITCODE" }
        if ($requiresSr) {
            & tar -x -f $staging -C $verifyRoot @archiveMembers
            if ($LASTEXITCODE -ne 0) { Fail "tar 无法提取必需超分条目，退出代码：$LASTEXITCODE" }
        }
        $finalPath = Join-Path $Output ($name + '.tar.xz')
        $tool = 'tar + xz，条目清单已校验'
    }

    if ($requiresSr -and $Compress -eq 'max') {
        $checkedPackage = Join-Path $verifyRoot $name
        Assert-SrFiles (Join-Path $checkedPackage 'bin') '压缩包'
        foreach ($entry in $srEntries.Keys) {
            $hash = (Get-FileHash -LiteralPath (Join-Path $checkedPackage $entry) -Algorithm SHA256).Hash
            if ($hash -ne $srEntries[$entry]) { Fail "压缩包超分条目 SHA256 不符：$entry" }
        }
    }
    Assert-OutputChild $finalPath $Output
}

# 发布之前重新检查目的地，源文件或压缩校验失败时保留已有包。
Assert-OutputChild $destination $Output
Assert-OutputChild $packageRoot $workRoot
if (Test-Path -LiteralPath $destination) {
    if (-not $Force) { Fail "目标已存在：$destination（要覆盖请加 -Force）" }
    Remove-Item -LiteralPath $destination -Recurse -Force
}
if (-not $ArchiveOnly -or $Compress -eq 'none') {
    Move-Item -LiteralPath $packageRoot -Destination $destination
    Write-Host ("包目录：{0}" -f $destination) -ForegroundColor Green
    Write-Host ("  文件 {0} 个，解压后 {1:N1} MB，版本 {2}，模式 {3}" -f $rows.Count, ($total / 1MB), $version, $Mode)
}
if ($finalPath) {
    if (Test-Path -LiteralPath $finalPath) { Remove-Item -LiteralPath $finalPath -Force }
    Move-Item -LiteralPath $staging -Destination $finalPath
    $size = (Get-Item -LiteralPath $finalPath).Length
    Write-Host ("压缩：{0}" -f $tool)
    Write-Host ("压缩包：{0}（{1:N1} MB，压到 {2:P0}）" -f $finalPath, ($size / 1MB), ($size / $total)) -ForegroundColor Green
    Write-Host '  解压后可按包内 package-manifest.json 逐文件核对 sha256'
    if ($ArchiveOnly) {
        Write-Host '  已按 -ArchiveOnly 只发布压缩包'
    }
}
elseif ($ArchiveOnly) {
    Write-Host '已发布目录。仅发布压缩包需要同时指定 -Compress zip|max' -ForegroundColor Yellow
}
}
finally {
    Assert-OutputChild $workRoot $Output
    Remove-Item -LiteralPath $workRoot -Recurse -Force
}
