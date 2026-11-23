#Requires -Version 5.1
<#
.SYNOPSIS
    界面验收测量：常驻内存、首帧、DPI 缩放，并抓一张窗口截图。

.DESCRIPTION
    连续启动三次 release 构建的 `DlssgLauncher --ui-selftest`，每次在稳定后（≥900 ms）
    采样工作集与私有字节，记录首帧耗时与 `pixels_per_point`，第一次运行额外截图。

    - 采样必须在稳定之后：字体与 GL 初始化期间有明显临时峰值。
    - 截图用窗口矩形 + CopyFromScreen，只用于人工核对中文渲染与布局，不做像素比对。

.PARAMETER Runs
    测量次数，默认 3。

.PARAMETER HoldMs
    每次窗口存活时间，默认 2200 ms。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File D:\Games\dlssg_for_sm86\rust\tools\measure-ui.ps1
#>
[CmdletBinding()]
param(
    [int]$Runs = 3,
    [int]$HoldMs = 2200
)

$ErrorActionPreference = 'Continue'
try { [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false) } catch { }

$toolsDir = $PSScriptRoot
$rustRoot = Split-Path -Parent $toolsDir
$docsDir = Join-Path $rustRoot 'docs'
$exe = Join-Path $rustRoot 'target\release\DlssgLauncher.exe'
$shot = Join-Path $docsDir 'm4-ui-150.png'

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class WindowProbe {
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, IntPtr lParam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    public static IntPtr FindByTitle(string fragment) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            if (!IsWindowVisible(h)) return true;
            var sb = new StringBuilder(512);
            GetWindowText(h, sb, sb.Capacity);
            if (sb.ToString().IndexOf(fragment, StringComparison.OrdinalIgnoreCase) >= 0) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@

function Measure-Run {
    param([int]$index, [string]$report)
    Remove-Item -LiteralPath $report -Force -ErrorAction SilentlyContinue
    $process = Start-Process -FilePath $exe -ArgumentList @('--ui-selftest', "$HoldMs", $report) -PassThru
    Start-Sleep -Milliseconds 1000
    if (-not $process.HasExited) { $process.Refresh() }
    $ws = [math]::Round($process.WorkingSet64 / 1MB, 1)
    $private = [math]::Round($process.PrivateMemorySize64 / 1MB, 1)

    if ($index -eq 1) {
        $handle = [WindowProbe]::FindByTitle('DLSSG 启动器')
        if ($handle -ne [IntPtr]::Zero) {
            $rect = New-Object WindowProbe+RECT
            if ([WindowProbe]::GetWindowRect($handle, [ref]$rect)) {
                $width = $rect.Right - $rect.Left
                $height = $rect.Bottom - $rect.Top
                if ($width -gt 0 -and $height -gt 0) {
                    $bitmap = New-Object System.Drawing.Bitmap $width, $height
                    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
                    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
                    $bitmap.Save($shot, [System.Drawing.Imaging.ImageFormat]::Png)
                    $graphics.Dispose()
                    $bitmap.Dispose()
                    Write-Host ("截图：{0}（{1}×{2}）" -f $shot, $width, $height)
                }
            }
        }
        else {
            Write-Host '截图跳过：没找到窗口标题。'
        }
    }

    $process.WaitForExit()
    $first = 'n/a'
    $ppp = 'n/a'
    if (Test-Path -LiteralPath $report) {
        $data = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
        $first = $data.first_frame_ms
        $ppp = $data.pixels_per_point
    }
    [pscustomobject]@{
        Run = $index
        WorkingSetMB = $ws
        PrivateMB = $private
        FirstFrameMs = $first
        PixelsPerPoint = $ppp
        ExitCode = $process.ExitCode
    }
}

Push-Location $rustRoot
try {
    Write-Host '=== 构建 release ==='
    & cargo build --release -p launcher 2>&1 | Select-Object -Last 2 | ForEach-Object { Write-Host $_ }
    if ($LASTEXITCODE -ne 0) { Write-Host '构建失败'; exit 1 }
    if (-not (Test-Path -LiteralPath $exe)) { Write-Host "缺少可执行文件：$exe"; exit 1 }

    Write-Host ''
    Write-Host "=== 测量（$Runs 次，每次 $HoldMs ms）==="
    $results = @()
    for ($i = 1; $i -le $Runs; $i++) {
        $results += Measure-Run -index $i -report (Join-Path $env:TEMP "dlssg-ui-measure-$i.json")
    }
    $results | Format-Table -AutoSize | Out-String | Write-Host

    $wsValues = $results | ForEach-Object { $_.WorkingSetMB }
    $firstValues = $results | Where-Object { $_.FirstFrameMs -ne 'n/a' } | ForEach-Object { [int]$_.FirstFrameMs }
    if ($wsValues.Count -gt 0) {
        $avgWs = [math]::Round(($wsValues | Measure-Object -Average).Average, 1)
        $maxWs = ($wsValues | Measure-Object -Maximum).Maximum
        Write-Host ("常驻内存：平均 {0} MB / 峰值 {1} MB（目标 ≤85 MB）" -f $avgWs, $maxWs)
    }
    if ($firstValues.Count -gt 0) {
        $avgFirst = [math]::Round(($firstValues | Measure-Object -Average).Average, 0)
        $maxFirst = ($firstValues | Measure-Object -Maximum).Maximum
        Write-Host ("首帧：平均 {0} ms / 峰值 {1} ms（目标 ≤300 ms）" -f $avgFirst, $maxFirst)
    }
    $failed = $results | Where-Object { $_.ExitCode -ne 0 }
    if ($failed) { Write-Host '有运行失败，见上表'; exit 1 }
    Write-Host '测量完成。'
}
finally {
    Pop-Location
}
