#Requires -Version 5.1
<#
.SYNOPSIS
    M6-02 差分核对：用**现在的** PowerShell 实现重算一遍样本，与仓库里的样本逐字节比较。

.DESCRIPTION
    样本（`rust/fixtures/**`）最初由 `gen-fixtures.ps1` 调 PS 生成；Rust 侧再断言与样本逐字节一致。
    本脚本把链路闭环：重新调用 PS 的 `Set-LauncherDiagnosticOptions` 与
    `Test-LauncherDiagnosticOptionsText`，和已入库的样本比对：

    - 12 份诊断 INI（3 款游戏 × 4 组参数）逐字节
    - 9 条诊断识别边界（Accepted 布尔值）

    任何不一致都会打印明细并以非零退出。只读：不写 fixtures，不碰游戏目录。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File D:\Games\dlssg_for_sm86\rust\tools\diff-parity.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false) } catch { }

$toolsDir = $PSScriptRoot
$rustRoot = Split-Path -Parent $toolsDir
$repoRoot = Split-Path -Parent $rustRoot
$launcherRoot = Join-Path $repoRoot '启动脚本'
$fixtures = Join-Path $rustRoot 'fixtures'

function Get-ScriptStringVariable {
    param([string]$Path, [string]$Name)
    $tokens = $null; $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$errors)
    if ($errors.Count -gt 0) { throw "语法错误：$Path（$($errors[0].Message)）" }
    $values = @()
    foreach ($node in $ast.FindAll({ param($n) $n -is [Management.Automation.Language.AssignmentStatementAst] }, $true)) {
        if ($node.Left -isnot [Management.Automation.Language.VariableExpressionAst]) { continue }
        if ($node.Left.VariablePath.UserPath -ne $Name) { continue }
        $right = $node.Right
        if ($right -is [Management.Automation.Language.CommandExpressionAst]) { $right = $right.Expression }
        if ($right -isnot [Management.Automation.Language.StringConstantExpressionAst]) { continue }
        $values += , $right.Value
    }
    if ($values.Count -ne 1) { throw "变量 $Name 在 $Path 中匹配到 $($values.Count) 处字面量赋值，期望 1 处。" }
    return [string]$values[0]
}

function Get-BytesHash {
    param([byte[]]$Bytes)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-', '') } finally { $sha.Dispose() }
}

. (Join-Path $launcherRoot 'scripts\common\LauncherConfig.ps1')

$games = @(
    @{ Id = 'wuthering'; Script = 'Run-WutheringWaves-DLSSG.ps1' },
    @{ Id = 'neverness'; Script = 'Run-NevernessToEverness-DLSSG.ps1' },
    @{ Id = 'steam-ue5'; Script = 'Run-SteamUE5-DLSSG.ps1' }
)
$combos = @(
    @{ Name = 'default'; Mode = 'Off'; Preset = 'A'; Optimized = '2'; Level = '2' },
    @{ Name = 'sr-m-int8'; Mode = 'M-INT8'; Preset = 'C'; Optimized = '3'; Level = '3' },
    @{ Name = 'sr-l-fp16'; Mode = 'L-FP16'; Preset = 'B'; Optimized = '0'; Level = '0' },
    @{ Name = 'fair'; Mode = 'Off'; Preset = 'B'; Optimized = '1'; Level = '1' }
)

$failures = [Collections.Generic.List[string]]::new()
$checked = 0

Write-Host '=== 1. 诊断 INI：PS 现算 vs 仓库样本（逐字节）==='
foreach ($game in $games) {
    $scriptPath = Join-Path $launcherRoot ('scripts\games\' + $game.Script)
    $template = Get-ScriptStringVariable $scriptPath 'diagnosticIni'
    foreach ($combo in $combos) {
        $text = Set-LauncherDiagnosticOptions -Template $template `
            -SuperResolutionMode $combo.Mode -DlssgPreset $combo.Preset `
            -Optimized $combo.Optimized -Level $combo.Level
        $fixturePath = Join-Path $fixtures ('diagnostic\{0}\{1}.ini' -f $game.Id, $combo.Name)
        $checked++
        if (-not (Test-Path -LiteralPath $fixturePath)) {
            $failures.Add("$($game.Id)/$($combo.Name)：样本缺失 $fixturePath")
            continue
        }
        $expected = [IO.File]::ReadAllBytes($fixturePath)
        $actual = [Text.Encoding]::UTF8.GetBytes($text)
        if ((Get-BytesHash $expected) -ne (Get-BytesHash $actual)) {
            $failures.Add("$($game.Id)/$($combo.Name)：字节不一致（样本 $($expected.Length) B，现算 $($actual.Length) B）")
        }
    }
}
Write-Host ("核对 {0} 份诊断 INI，失败 {1} 份" -f $checked, $failures.Count)

Write-Host ''
Write-Host '=== 2. 诊断识别：PS 现算 vs 仓库记录（Accepted）==='
$recognitionPath = Join-Path $fixtures 'diagnostic\recognition.json'
if (-not (Test-Path -LiteralPath $recognitionPath)) {
    $failures.Add("缺少 recognition.json：$recognitionPath")
}
else {
    $recorded = Get-Content -LiteralPath $recognitionPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $mainScript = Join-Path $launcherRoot 'scripts\games\Run-WutheringWaves-DLSSG.ps1'
    $mainTemplate = Get-ScriptStringVariable $mainScript 'diagnosticIni'
    $recognitionChecked = 0
    $recognitionBefore = $failures.Count
    foreach ($case in @($recorded.Cases)) {
        $casePath = Join-Path $fixtures ('diagnostic\recognition\{0}.ini' -f $case.Case)
        if (-not (Test-Path -LiteralPath $casePath)) {
            $failures.Add("识别用例缺样本：$($case.Case)")
            continue
        }
        $text = [IO.File]::ReadAllText($casePath)
        $accepted = [bool](Test-LauncherDiagnosticOptionsText -Contents $text -Template $mainTemplate)
        $recognitionChecked++
        if ($accepted -ne [bool]$case.Accepted) {
            $failures.Add(("识别边界 {0}：记录 Accepted={1}，现算 {2}" -f $case.Case, $case.Accepted, $accepted))
        }
    }
    $recognitionFailed = $failures.Count - $recognitionBefore
    Write-Host ("核对 {0} 条识别边界，失败 {1} 条" -f $recognitionChecked, $recognitionFailed)
    $checked += $recognitionChecked
}

Write-Host ''
if ($failures.Count -gt 0) {
    Write-Host ("=== 差异 {0} 处 ===" -f $failures.Count)
    foreach ($failure in $failures) { Write-Host ("- " + $failure) }
    exit 1
}
Write-Host ("差分核对通过：{0} 项，差异 0 处。" -f $checked)
