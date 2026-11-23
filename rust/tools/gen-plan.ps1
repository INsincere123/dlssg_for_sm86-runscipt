#Requires -Version 5.1
<#
.SYNOPSIS
    由 rust/plan/plan.json 生成 rust/plan/plan.md（只读视图，不要手改 md）。

.DESCRIPTION
    plan.json 是唯一事实来源。本脚本把环境、命令、规则、关卡、任务渲染成 Markdown 表格，
    便于 grep 与快速查看；写回一律改 plan.json 再重新生成。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File D:\Games\dlssg_for_sm86\rust\tools\gen-plan.ps1
#>
[CmdletBinding()]
param(
    [string]$PlanPath,
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$toolsDir = $PSScriptRoot
$rustRoot = Split-Path -Parent $toolsDir
if (-not $PlanPath) { $PlanPath = Join-Path $rustRoot 'plan\plan.json' }
if (-not $OutputPath) { $OutputPath = Join-Path $rustRoot 'plan\plan.md' }

$plan = Get-Content -LiteralPath $PlanPath -Raw -Encoding UTF8 | ConvertFrom-Json

$builder = [Text.StringBuilder]::new()
function Add-Line { param([string]$Text = '') [void]$builder.AppendLine($Text) }

# Markdown 表格里不能出现裸竖线：单元内容统一转义
function Format-Cell { param([string]$Text) return ($Text -replace '\|', '\|') }

Add-Line ('# 开发计划（生成物，源：plan.json）')
Add-Line ''
Add-Line ('- 生成时间（UTC）：{0}' -f (Get-Date).ToUniversalTime().ToString('o'))
Add-Line ('- 版本：{0}　受众：{1}' -f $plan.version, $plan.audience)
Add-Line ('- 说明：{0}' -f $plan.note)
Add-Line ''

Add-Line '## 入口'
Add-Line ''
Add-Line '| 键 | 路径 |'
Add-Line '|---|---|'
foreach ($entry in $plan.entrypoints.PSObject.Properties) {
    Add-Line ('| `{0}` | `{1}` |' -f $entry.Name, $entry.Value)
}
Add-Line ''

Add-Line '## 环境'
Add-Line ''
Add-Line '| 项 | 值 |'
Add-Line '|---|---|'
foreach ($entry in $plan.environment.PSObject.Properties) {
    Add-Line ('| {0} | {1} |' -f $entry.Name, $entry.Value)
}
Add-Line ''

Add-Line '## 命令'
Add-Line ''
Add-Line '| 名称 | 命令 |'
Add-Line '|---|---|'
foreach ($entry in $plan.commands.PSObject.Properties) {
    Add-Line ('| `{0}` | `{1}` |' -f $entry.Name, $entry.Value)
}
Add-Line ''

Add-Line '## 规则（违反即为缺陷）'
Add-Line ''
Add-Line '| ID | 规则 | 强制方式 |'
Add-Line '|---|---|---|'
foreach ($rule in $plan.rules) {
    Add-Line ('| {0} | {1} | {2} |' -f $rule.id, (Format-Cell $rule.rule), (Format-Cell $rule.enforce))
}
Add-Line ''

Add-Line '## 关卡'
Add-Line ''
Add-Line '| 关卡 | 状态 | 范围 | 证据 |'
Add-Line '|---|---|---|---|'
foreach ($gate in $plan.gates) {
    Add-Line ('| {0} | {1} | {2} | {3} |' -f $gate.id, $gate.status, (Format-Cell $gate.scope), (Format-Cell $gate.evidence))
}
Add-Line ''

$done = @($plan.tasks | Where-Object status -eq 'done').Count
$todo = @($plan.tasks | Where-Object status -eq 'todo').Count
Add-Line ('## 任务（done {0} / todo {1}）' -f $done, $todo)
Add-Line ''

$milestones = $plan.tasks | Group-Object milestone | Sort-Object Name
foreach ($milestone in $milestones) {
    Add-Line ('### {0}' -f $milestone.Name)
    Add-Line ''
    Add-Line '| ID | 任务 | 状态 | 依赖 | 验收 | 产物 |'
    Add-Line '|---|---|---|---|---|---|'
    foreach ($task in ($milestone.Group | Sort-Object id)) {
        $deps = if (@($task.deps).Count) { (@($task.deps) -join ', ') } else { '—' }
        $acceptance = (@($task.acceptance) -join '；')
        $artifacts = (@($task.artifacts) -join '、')
        Add-Line ('| {0} | {1} | {2} | {3} | {4} | {5} |' -f $task.id, (Format-Cell $task.title), $task.status, (Format-Cell $deps), (Format-Cell $acceptance), (Format-Cell $artifacts))
    }
    Add-Line ''
}

[IO.Directory]::CreateDirectory((Split-Path -Parent $OutputPath)) | Out-Null
[IO.File]::WriteAllText($OutputPath, $builder.ToString(), [Text.UTF8Encoding]::new($false))
Write-Host ('已生成：{0}（done {1} / todo {2}）' -f $OutputPath, $done, $todo)
