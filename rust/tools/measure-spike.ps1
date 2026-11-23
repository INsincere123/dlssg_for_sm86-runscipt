#Requires -Version 5.1
<#
.SYNOPSIS
    M0 spike 实测：冷启动、常驻内存、控制台窗口、DPI、中文字体、可选截图。

.DESCRIPTION
    只启动 rust/spike 的产物，不接触游戏目录与注册表。
    截图仅在 -Screenshot 时进行，且只对 spike 自己的窗口做屏幕拷贝（OpenGL 窗口用 PrintWindow 会是空白）。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File rust\tools\measure-spike.ps1 -Screenshot
#>
[CmdletBinding()]
param(
    [string]$Exe,
    [int]$HoldMs = 6000,
    [switch]$Screenshot,
    [string[]]$ExtraArgs,
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$toolsDir = $PSScriptRoot
$rustRoot = Split-Path -Parent $toolsDir
if (-not $Exe) { $Exe = Join-Path $rustRoot 'spike\target\release\spike.exe' }
if (-not (Test-Path -LiteralPath $Exe)) { throw "找不到 spike 产物：$Exe（先 cargo build --release）" }
$Exe = (Resolve-Path $Exe).Path

Add-Type -AssemblyName System.Drawing
Add-Type -Namespace SpikeCap -Name Native -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
[DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
[StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
'@

# 让本进程成为 PerMonitorV2，否则 GetWindowRect/CopyFromScreen 在 150% 缩放下拿到的坐标与真实像素不一致。
[void][SpikeCap.Native]::SetProcessDpiAwarenessContext([IntPtr](-4))

$temporary = Join-Path $env:TEMP ('spike-measure-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
[void][IO.Directory]::CreateDirectory($temporary)

function Stop-Spike {
    Get-Process -Name 'spike' -ErrorAction SilentlyContinue | ForEach-Object {
        try { $_.Kill() } catch { }
    }
}

$extraList = @(); if ($null -ne $ExtraArgs) { $extraList = @($ExtraArgs) }

Stop-Spike
$result = [ordered]@{
    Exe = $Exe
    ExeBytes = (Get-Item -LiteralPath $Exe).Length
    MeasuredAt = (Get-Date).ToUniversalTime().ToString('o')
    Samples = @()
}

# 1. 冷启动到首帧 + 首帧内存
$reportPath = Join-Path $temporary 'basic.json'
$watch = [Diagnostics.Stopwatch]::StartNew()
$process = Start-Process -FilePath $Exe -ArgumentList (@('--report', $reportPath) + $extraList) -PassThru
if (-not $process.WaitForExit(30000)) { try { $process.Kill() } catch { } }
$watch.Stop()
Start-Sleep -Milliseconds 250
if (-not (Test-Path -LiteralPath $reportPath)) { throw '首帧报告未生成。' }
$basic = Get-Content -LiteralPath $reportPath -Raw -Encoding UTF8 | ConvertFrom-Json
$result.ColdStartWallclockMs = $watch.Elapsed.TotalMilliseconds
$result.Basic = $basic

# 2. 保持窗口期间的内存采样
$holdPath = Join-Path $temporary 'hold.json'
$process = Start-Process -FilePath $Exe -ArgumentList (@('--report', $holdPath, '--hold', $HoldMs) + $extraList) -PassThru
$samples = [Collections.Generic.List[object]]::new()
$screenshotPath = $null
$deadline = (Get-Date).AddMilliseconds($HoldMs)
$captured = $false
while (-not $process.HasExited -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 400
    $process.Refresh()
    if ($process.HasExited) { break }
    $samples.Add([pscustomobject]@{
        ElapsedMs = [int](($process.StartTime - $process.StartTime).TotalMilliseconds)
        WorkingSetMB = [math]::Round($process.WorkingSet64 / 1MB, 2)
        PrivateMB = [math]::Round($process.PrivateMemorySize64 / 1MB, 2)
    })
    if ($Screenshot -and -not $captured -and $process.MainWindowHandle -ne 0) {
        Start-Sleep -Milliseconds 400
        $process.Refresh()
        $handle = $process.MainWindowHandle
        if ($handle -ne 0) {
            [void][SpikeCap.Native]::ShowWindow($handle, 5)
            [void][SpikeCap.Native]::SetForegroundWindow($handle)
            Start-Sleep -Milliseconds 250
            $rect = New-Object SpikeCap.Native+RECT
            if ([SpikeCap.Native]::GetWindowRect($handle, [ref]$rect)) {
                $width = $rect.Right - $rect.Left
                $height = $rect.Bottom - $rect.Top
                if ($width -gt 100 -and $height -gt 100) {
                    $bitmap = New-Object Drawing.Bitmap $width, $height
                    $graphics = [Drawing.Graphics]::FromImage($bitmap)
                    try {
                        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object Drawing.Size $width, $height))
                    }
                    finally { $graphics.Dispose() }
                    $screenshotPath = Join-Path $rustRoot ('spike\target\spike-window.png')
                    $bitmap.Save($screenshotPath, [Drawing.Imaging.ImageFormat]::Png)
                    $bitmap.Dispose()
                    $captured = $true
                }
            }
        }
    }
}
if (-not $process.WaitForExit(30000)) { try { $process.Kill() } catch { } }
$result.HoldSamples = @($samples)
if ($samples.Count -gt 0) {
    $result.WorkingSetMBMin = ($samples | Measure-Object WorkingSetMB -Minimum).Minimum
    $result.WorkingSetMBMax = ($samples | Measure-Object WorkingSetMB -Maximum).Maximum
    $result.WorkingSetMBLast = $samples[$samples.Count - 1].WorkingSetMB
    $result.PrivateMBLast = $samples[$samples.Count - 1].PrivateMB
}
if (Test-Path -LiteralPath $holdPath) {
    $result.HoldReport = Get-Content -LiteralPath $holdPath -Raw -Encoding UTF8 | ConvertFrom-Json
}
$result.ScreenshotPath = $screenshotPath

# 3. 子进程模式：无控制台的进程能否启动并落盘
$childPath = Join-Path $temporary 'child.txt'
$childProcess = Start-Process -FilePath $Exe -ArgumentList @('--child', $childPath) -PassThru -Wait
Start-Sleep -Milliseconds 150
$result.ChildExitCode = $childProcess.ExitCode
$result.ChildReport = if (Test-Path -LiteralPath $childPath) { (Get-Content -LiteralPath $childPath -Raw -Encoding UTF8).Trim() } else { 'MISSING' }

Stop-Spike
Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue

if ($OutputPath) {
    [IO.File]::WriteAllText($OutputPath, (ConvertTo-Json -InputObject $result -Depth 8), [Text.UTF8Encoding]::new($false))
    Write-Host ('结果已写入：{0}' -f $OutputPath)
}

Write-Host ''
Write-Host '=== M0 spike 实测 ==='
Write-Host ('exe 体积        : {0:N0} B ({1:N2} MB)' -f $result.ExeBytes, ($result.ExeBytes / 1MB))
Write-Host ('冷启动到首帧    : {0:N1} ms（spike 自测 {1:N1} ms）' -f $basic.cold_start_ms, $basic.cold_start_ms)
Write-Host ('首帧常驻内存    : {0:N1} MB' -f ($basic.working_set_kb / 1024))
if ($samples.Count -gt 0) {
    Write-Host ('保持期内存      : 最低 {0:N1} MB / 最高 {1:N1} MB / 末次 {2:N1} MB' -f $result.WorkingSetMBMin, $result.WorkingSetMBMax, $result.WorkingSetMBLast)
}
Write-Host ('控制台窗口      : {0}' -f $(if ($basic.console_window) { '有（不符合预期）' } else { '无' }))
Write-Host ('DPI 感知        : PerMonitorV2 应用={0}；系统 DPI={1}；pixels_per_point={2}' -f $basic.dpi_awareness_applied, $basic.dpi_system, $basic.native_pixels_per_point)
Write-Host ('中文字体        : msyh.ttc {0:N1} MB，加载={1}' -f ($basic.font_msyh_bytes / 1MB), $(if ($basic.font_msyh_bytes -gt 0) { '是' } else { '否' }))
Write-Host ('子进程无控制台  : {0}' -f $result.ChildReport)
if ($screenshotPath) { Write-Host ('窗口截图        : {0}' -f $screenshotPath) }
