#Requires -Version 5.1
<#
.SYNOPSIS
    生成 PARITY 的函数级清单：20 个 PS1 的每个函数、行号、行数与 Rust 归宿，并列出跨文件重复。

.DESCRIPTION
    只读源码，只写一个 Markdown 文件。Rust 侧模块名按本文件内的映射表决定，
    函数级的特殊归宿走 $overrides。改动 PS 后重跑本脚本即可刷新清单。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File rust\tools\gen-parity-inventory.ps1
#>
[CmdletBinding()]
param(
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$toolsDir = $PSScriptRoot
$rustRoot = Split-Path -Parent $toolsDir
$repoRoot = Split-Path -Parent $rustRoot
$launcherRoot = Join-Path $repoRoot '启动脚本'
if (-not $OutputPath) { $OutputPath = Join-Path $rustRoot 'docs\parity-functions.md' }

# 文件 → Rust 归属（函数级例外见 $overrides）
$fileMap = [ordered]@{
    'scripts\common\DeploymentJournal.ps1' = 'core::journal / core::restore'
    'scripts\common\LauncherConfig.ps1'    = 'core::config / core::settings'
    'scripts\common\LauncherResources.ps1' = 'core::resources'
    'scripts\common\PortableLauncher.ps1'  = 'core::prefs / core::session'
    'scripts\common\GameDiscovery.ps1'     = 'core::discovery'
    'scripts\common\SteamLauncher.ps1'     = 'core::steam'
    'scripts\common\DirectLauncher.ps1'    = 'core::direct'
    'scripts\common\DlssgOnlyLauncher.ps1' = 'core::session'
    'scripts\common\LauncherSession.ps1'   = 'core::session'
    'scripts\common\UnrealQuality.ps1'     = 'core::quality（重写：删探测/校准）'
    'scripts\common\ScriptLogging.ps1'     = 'core::log'
    'scripts\common\LauncherConsole.ps1'   = '删除（无控制台）'
    'scripts\common\LaunchFromBat.ps1'     = '由 EXE 自提权替代'
    'scripts\games\Run-WutheringWaves-DLSSG.ps1'      = 'core::session + games::wuthering'
    'scripts\games\Run-NevernessToEverness-DLSSG.ps1' = 'core::session + games::neverness'
    'scripts\games\Run-SteamUE5-DLSSG.ps1'            = 'core::session + games::steam'
    'scripts\tools\Export-PortableLauncher.ps1' = 'core::export'
    'scripts\tools\Toggle-NvidiaDeviceName.ps1' = 'core::gpu'
    'scripts\ui\Show-DlssgLauncher.ps1'    = 'launcher::ui'
    'scripts\ui\Show-SteamLauncher.ps1'    = 'launcher::ui'
}

# 函数级例外
$overrides = @{
    'Get-NormalizedPath'                = 'core::path（三处重复，合并）'
    'Get-HashOrNull'                    = 'core::path'
    'Assert-LinkOrAbsent'               = 'core::resources'
    'Ensure-Link'                       = 'core::resources'
    'Set-LauncherDiagnosticOptions'     = 'core::settings::diagnostic'
    'Test-LauncherDiagnosticOptionsText'= 'core::settings::diagnostic'
    'Resolve-LauncherAdvancedValue'     = 'core::settings'
    'Get-LauncherAdvancedChoices'       = 'core::settings'
    'Get-LauncherAdvancedDefault'       = 'core::settings'
    'Register-ReusedDeploymentLink'     = 'core::journal'
    'Restore-RecordedDeployment'        = 'core::restore'
    'Resolve-PortableQuality'           = 'core::quality（改写：EngineHost）'
    'Initialize-PortableQuality'        = 'core::quality（改写：EngineHost）'
    'Save-PortableEngineDetection'      = '删除'
    'Get-PortableSavedEngine'           = '删除'
    'Get-PortableEngineDetection'       = '删除'
    'Invoke-PortableEngineDetection'    = '删除'
    'Get-PortableEngineEvidence'        = '删除'
}

$rows = [Collections.Generic.List[object]]::new()
$duplicateIndex = @{}

foreach ($relative in $fileMap.Keys) {
    $path = Join-Path $launcherRoot $relative
    if (-not (Test-Path -LiteralPath $path)) { throw "缺少文件：$path" }
    $tokens = $null; $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
    if ($errors.Count -gt 0) { throw "语法错误：$relative（$($errors[0].Message)）" }
    $functions = @($ast.FindAll({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] }, $true) |
        Where-Object { $_.Parent -isnot [Management.Automation.Language.FunctionDefinitionAst] })
    foreach ($function in ($functions | Sort-Object { $_.Extent.StartLineNumber })) {
        $name = $function.Name
        $text = $function.Extent.Text
        $hash = [BitConverter]::ToString(
            [Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($text))
        ).Replace('-', '').Substring(0, 12)
        $target = if ($overrides.ContainsKey($name)) { $overrides[$name] } else { $fileMap[$relative] }
        $rows.Add([pscustomobject]@{
            File = $relative
            Function = $name
            Start = $function.Extent.StartLineNumber
            Lines = $function.Extent.EndLineNumber - $function.Extent.StartLineNumber + 1
            BodyHash = $hash
            Target = $target
        })
        if (-not $duplicateIndex.ContainsKey($name)) { $duplicateIndex[$name] = [Collections.Generic.List[object]]::new() }
        $duplicateIndex[$name].Add([pscustomobject]@{ File = $relative; Hash = $hash; Lines = $function.Extent.EndLineNumber - $function.Extent.StartLineNumber + 1 })
    }
}

$duplicates = foreach ($name in ($duplicateIndex.Keys | Sort-Object)) {
    $entries = $duplicateIndex[$name]
    if ($entries.Count -lt 2) { continue }
    $uniqueHashes = @($entries | ForEach-Object Hash | Sort-Object -Unique)
    [pscustomobject]@{
        Function = $name
        Copies = $entries.Count
        Identical = ($uniqueHashes.Count -eq 1)
        Lines = $entries[0].Lines
        Files = (@($entries | ForEach-Object { Split-Path -Leaf $_.File }) -join ' / ')
    }
}

$totalLines = 0
foreach ($relative in $fileMap.Keys) { $totalLines += (Get-Content (Join-Path $launcherRoot $relative) -Encoding UTF8).Count }

$builder = [Text.StringBuilder]::new()
$null = $builder.AppendLine('# PARITY 函数级清单（生成物）')
$null = $builder.AppendLine()
$null = $builder.AppendLine('> 本文件由 `rust/tools/gen-parity-inventory.ps1` 生成，**不要手工编辑**。')
$null = $builder.AppendLine('> 契约与决策见 [PARITY.md](PARITY.md)。')
$null = $builder.AppendLine()
$null = $builder.AppendLine(('- 生成时间（UTC）：{0}' -f (Get-Date).ToUniversalTime().ToString('o')))
$null = $builder.AppendLine(('- 源文件：{0} 个，合计 {1} 行，{2} 个函数' -f $fileMap.Count, $totalLines, $rows.Count))
$null = $builder.AppendLine(('- PowerShell：{0}' -f $PSVersionTable.PSVersion.ToString()))
$null = $builder.AppendLine()

$null = $builder.AppendLine('## 1. 逐文件清单')
foreach ($relative in $fileMap.Keys) {
    $fileRows = @($rows | Where-Object File -eq $relative)
    $null = $builder.AppendLine()
    $null = $builder.AppendLine(('### {0}' -f ($relative -replace '\\', '/') + "  →  " + $fileMap[$relative]))
    $null = $builder.AppendLine()
    if ($fileRows.Count -eq 0) {
        $null = $builder.AppendLine('（无函数定义；整文件行为见 PARITY.md）')
        continue
    }
    $null = $builder.AppendLine('| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |')
    $null = $builder.AppendLine('|---|---:|---:|---|---|')
    foreach ($row in $fileRows) {
        $null = $builder.AppendLine(('| `{0}` | {1} | {2} | `{3}` | {4} |' -f $row.Function, $row.Start, $row.Lines, $row.BodyHash, $row.Target))
    }
}

$null = $builder.AppendLine()
$null = $builder.AppendLine('## 2. 跨文件重复函数')
$null = $builder.AppendLine()
$null = $builder.AppendLine('正文哈希相同的副本可以只保留一份实现；哈希不同的必须逐条比对，不能默认等价。')
$null = $builder.AppendLine()
$null = $builder.AppendLine('| 函数 | 副本数 | 正文全同 | 行数 | 出现文件 |')
$null = $builder.AppendLine('|---|---:|---|---:|---|')
foreach ($duplicate in ($duplicates | Sort-Object -Property @{ Expression = 'Identical'; Descending = $false }, Function)) {
    $null = $builder.AppendLine(('| `{0}` | {1} | {2} | {3} | {4} |' -f
        $duplicate.Function, $duplicate.Copies, $(if ($duplicate.Identical) { '是' } else { '**否**' }), $duplicate.Lines, $duplicate.Files))
}

[void][IO.Directory]::CreateDirectory((Split-Path -Parent $OutputPath))
[IO.File]::WriteAllText($OutputPath, $builder.ToString(), [Text.UTF8Encoding]::new($false))
Write-Host ('清单已生成：{0}' -f $OutputPath)
Write-Host ('函数 {0} 个；重复组 {1} 个（其中正文不一致 {2} 组）' -f $rows.Count, @($duplicates).Count, @($duplicates | Where-Object { -not $_.Identical }).Count)
