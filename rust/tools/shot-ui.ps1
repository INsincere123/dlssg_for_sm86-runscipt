#Requires -Version 5.1
<#
.SYNOPSIS
    界面截图：按标题抓一张 release 构建的窗口，然后关掉进程。

.DESCRIPTION
    开发期核对布局用（弹窗、行内按钮、设置分页）。只做人工目视检查，不做像素比对。

    三个必须的坑（2026-11-23 都踩过）：

    - 本进程要先设成 Per-Monitor V2 DPI 感知。否则 `GetWindowRect` 给的是缩放后的虚拟
      坐标，与 `CopyFromScreen` 的物理像素不一致，裁出来的窗口会偏（实测偏约 170 px，
      看起来就像"底栏没画出来"）。
    - 先抓整块主屏再按窗口矩形裁切，而不是直接把窗口左上角交给 `CopyFromScreen`。
    - 只调 `SetForegroundWindow` 可能被前台锁拒绝（截图结果是压在下面的浏览器）。改用
      `SetWindowPos(HWND_TOPMOST)` 抬到最前，抓完再取消置顶。

.PARAMETER Arguments
    传给启动器的参数（空格分隔），例如 `--ui-preview-restore ok`。默认空，只开主窗口。

.PARAMETER Output
    PNG 输出路径。默认 `rust/docs/ui-shot.png`。

.PARAMETER WaitMs
    等窗口画完的毫秒数，默认 1500。

.PARAMETER Title
    窗口标题片段，默认 `DLSSG 启动器`。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File D:\Games\dlssg_for_sm86\rust\tools\shot-ui.ps1 `
        -Arguments '--ui-preview-restore ok' -Output D:\Games\dlssg_for_sm86\rust\docs\shot-restore-ok.png
#>
[CmdletBinding()]
param(
    [string]$Arguments = '',
    [string]$Output,
    [int]$WaitMs = 1500,
    [string]$Title = 'DLSSG 启动器',
    # 悬停位置（窗口内物理像素，左边/上边为 0）；给了就把鼠标移过去，等提示浮出。
    [string]$HoverWindow = ''
)

$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false) } catch { }

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class ShotWindow {
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, IntPtr lParam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    const uint SWP_NOSIZE = 0x0001;
    const uint SWP_NOMOVE = 0x0002;
    public static void MakeDpiAware() {
        // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
        try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
    }
    public static void BringToFront(IntPtr window) {
        ShowWindow(window, 9); // SW_RESTORE
        SetForegroundWindow(window);
        // HWND_TOPMOST = -1：前台锁生效时这是唯一可靠的办法。
        SetWindowPos(window, new IntPtr(-1), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    }
    public static void DropTopmost(IntPtr window) {
        // HWND_NOTOPMOST = -2
        SetWindowPos(window, new IntPtr(-2), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    }
    public static string Key(IntPtr window) {
        RECT r;
        if (!GetWindowRect(window, out r)) return "?";
        return r.Left + "," + r.Top + "," + r.Right + "," + r.Bottom;
    }
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
[ShotWindow]::MakeDpiAware()

$toolsDir = $PSScriptRoot
$rustRoot = Split-Path -Parent $toolsDir
$exe = Join-Path $rustRoot 'target\release\DlssgLauncher.exe'
if (-not $Output) { $Output = Join-Path $rustRoot 'docs\ui-shot.png' }

if (-not (Test-Path -LiteralPath $exe)) { Write-Host "缺少可执行文件：$exe"; exit 1 }
# 只清理"界面"实例：带 --engine 的是正在干活的引擎，杀了会把用户正在跑的游戏丢在部署状态。
$stale = @(Get-CimInstance Win32_Process -Filter "Name='DlssgLauncher.exe'" |
    Where-Object { $_.CommandLine -and $_.CommandLine -notmatch '--engine' } |
    ForEach-Object { Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue })
if ($stale.Count -gt 0) {
    Write-Host "结束残留界面进程：$(($stale | ForEach-Object { $_.Id }) -join ', ')"
    $stale | Stop-Process -Force
    Start-Sleep -Milliseconds 300
}

$argList = @()
if ($Arguments.Trim()) { $argList = $Arguments -split '\s+' }
$process = if ($argList.Count -gt 0) {
    Start-Process -FilePath $exe -ArgumentList $argList -PassThru
}
else {
    Start-Process -FilePath $exe -PassThru
}
try {
    Start-Sleep -Milliseconds $WaitMs
    $handle = [ShotWindow]::FindByTitle($Title)
    if ($handle -eq [IntPtr]::Zero) { Write-Host '截图失败：没找到窗口标题。'; exit 1 }
    [ShotWindow]::BringToFront($handle)
    Start-Sleep -Milliseconds 500

    if ($HoverWindow.Trim()) {
        $parts = $HoverWindow -split ','
        if ($parts.Count -eq 2) {
            $hoverRect = New-Object ShotWindow+RECT
            if ([ShotWindow]::GetWindowRect($handle, [ref]$hoverRect)) {
                $hx = $hoverRect.Left + [int]$parts[0]
                $hy = $hoverRect.Top + [int]$parts[1]
                # 来回抖动几下：egui 只在有输入事件时重绘，静止鼠标的提示可能永远不出现。
                foreach ($offset in @(0, 3, 0, 3, 0)) {
                    [void][ShotWindow]::SetCursorPos($hx + $offset, $hy)
                    Start-Sleep -Milliseconds 260
                }
                Write-Host ("鼠标移到窗口内 ({0},{1})" -f $parts[0], $parts[1])
                Start-Sleep -Milliseconds 700
            }
        }
    }

    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $rect = New-Object ShotWindow+RECT
    $screen = $null
    for ($attempt = 1; $attempt -le 6; $attempt++) {
        $before = [ShotWindow]::Key($handle)
        $screen = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
        $screenGraphics = [System.Drawing.Graphics]::FromImage($screen)
        $screenGraphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $screen.Size)
        $screenGraphics.Dispose()
        $after = [ShotWindow]::Key($handle)
        if ($before -eq $after) { break }
        $screen.Dispose()
        $screen = $null
    }
    if ($null -eq $screen) { Write-Host '截图失败：窗口一直在移动。'; exit 1 }
    if (-not [ShotWindow]::GetWindowRect($handle, [ref]$rect)) { Write-Host '截图失败：读不到窗口矩形。'; exit 1 }

    $left = [Math]::Max(0, $rect.Left - $bounds.Left)
    $top = [Math]::Max(0, $rect.Top - $bounds.Top)
    $width = [Math]::Min($rect.Right - $rect.Left, $bounds.Width - $left)
    $height = [Math]::Min($rect.Bottom - $rect.Top, $bounds.Height - $top)
    if ($width -le 0 -or $height -le 0) { $screen.Dispose(); Write-Host '截图失败：窗口尺寸异常。'; exit 1 }
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $source = New-Object System.Drawing.Rectangle $left, $top, $width, $height
    $target = New-Object System.Drawing.Rectangle 0, 0, $width, $height
    $graphics.DrawImage($screen, $target, $source, [System.Drawing.GraphicsUnit]::Pixel)
    $graphics.Dispose()
    $screen.Dispose()

    $parent = Split-Path -Parent $Output
    if ($parent -and -not (Test-Path -LiteralPath $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
    $bitmap.Save($Output, [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
    Write-Host ("截图：{0}（{1}×{2}）" -f $Output, $width, $height)
}
finally {
    if ($handle -and $handle -ne [IntPtr]::Zero) { [ShotWindow]::DropTopmost($handle) }
    if (-not $process.HasExited) { $process.Kill() }
    $process.WaitForExit(3000) | Out-Null
}
