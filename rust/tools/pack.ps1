#requires -Version 5.1
<#
.SYNOPSIS
    打 v0.1 的正式包：全量版 / 轻量版（同一个 exe）。

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

    轻量版只是把 sr 两个内核（约 172 MB）排除掉，exe 完全相同；程序按
    "bin\dlssg_sm86\sr 在不在"自行判断，开超分却没有内核时会在部署前报错。

.PARAMETER Mode
    full（默认）或 light。

.PARAMETER Output
    输出目录，默认 <仓库>\rust\dist。实际包目录是 <Output>\DlssgLauncher-<版本>-<模式>。

.PARAMETER Zip
    打完再压成同名 .zip。

.PARAMETER Force
    目标目录已存在时先删掉重建。
#>
[CmdletBinding()]
param(
    [ValidateSet('full', 'light')]
    [string]$Mode = 'full',
    [string]$Output,
    [switch]$Zip,
    [switch]$Force
)

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

if (-not (Test-Path -LiteralPath $exe)) { Fail "没有发布版 exe：$exe（先 cargo build -p launcher --release）" }

# 版本：优先问 exe 自己，问不到再看 Cargo.toml
$version = $null
$probe = Join-Path ([System.IO.Path]::GetTempPath()) ("dlssg-pack-version-{0}.json" -f $PID)
try {
    $process = Start-Process -FilePath $exe -ArgumentList @('--version', '--report', $probe) -PassThru -Wait
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
$display = 'v' + ($version -replace '\.0$', '')

$name = "DlssgLauncher-$display-$Mode"
$packageRoot = Join-Path $Output $name
if (Test-Path -LiteralPath $packageRoot) {
    if (-not $Force) { Fail "目标已存在：$packageRoot（要覆盖请加 -Force）" }
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null

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
$srSource = Join-Path $repoRoot 'dlssg_sm86\sr'
if ($Mode -eq 'full') {
    if (-not (Test-Path -LiteralPath (Join-Path $srSource 'sr_kernels.dll'))) {
        Fail "全量版需要 sr 内核：$srSource（这份是本地文件，不在 Git 里）"
    }
    Copy-Tree $srSource (Join-Path $bin 'dlssg_sm86\sr') '*.dll'
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

# 5) 清单：每个文件的大小与 SHA256
$files = Get-ChildItem -LiteralPath $packageRoot -File -Recurse | Sort-Object FullName
$rows = @()
$total = 0
foreach ($file in $files) {
    $relative = $file.FullName.Substring($packageRoot.Length).TrimStart('\')
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $rows += [pscustomobject]@{ path = $relative; bytes = $file.Length; sha256 = $hash }
    $total += $file.Length
}
$manifest = [pscustomobject]@{
    package = $name
    version = $version
    mode    = $Mode
    layout  = 'exe at root, payload under bin/'
    created = (Get-Date).ToString('yyyy-MM-ddTHH:mm:ssK')
    totals  = [pscustomobject]@{ files = $rows.Count; bytes = $total }
    files   = $rows
}
$manifestPath = Join-Path $packageRoot 'package-manifest.json'
$manifestJson = $manifest | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText($manifestPath, $manifestJson, (New-Object System.Text.UTF8Encoding($false)))

Write-Host ("包目录：{0}" -f $packageRoot) -ForegroundColor Green
Write-Host ("  文件 {0} 个，解压后 {1:N1} MB，版本 {2}，模式 {3}" -f $rows.Count, ($total / 1MB), $version, $Mode)

if ($Zip) {
    $zipPath = Join-Path $Output ($name + '.zip')
    if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }
    Compress-Archive -Path (Join-Path $packageRoot '*') -DestinationPath $zipPath -CompressionLevel Optimal
    Write-Host ("压缩包：{0}（{1:N1} MB）" -f $zipPath, ((Get-Item -LiteralPath $zipPath).Length / 1MB)) -ForegroundColor Green
}
