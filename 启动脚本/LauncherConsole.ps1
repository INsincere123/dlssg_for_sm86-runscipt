# 两个启动脚本共用：运行期间禁用控制台的关闭按钮，清理完成后恢复。
function Initialize-LauncherConsole {
    param([string]$ScriptPath, [System.Collections.IDictionary]$Parameters)

    if (-not ('DLSSG.LauncherConsole' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace DLSSG {
    public static class LauncherConsole {
        [DllImport("kernel32.dll")]
        public static extern IntPtr GetConsoleWindow();
        [DllImport("user32.dll")]
        public static extern bool IsWindowVisible(IntPtr window);
        [DllImport("user32.dll")]
        public static extern IntPtr GetSystemMenu(IntPtr window, bool revert);
        [DllImport("user32.dll")]
        public static extern uint EnableMenuItem(IntPtr menu, uint item, uint flags);
        [DllImport("user32.dll")]
        public static extern bool DrawMenuBar(IntPtr window);
    }
}
'@
    }

    $window = [DLSSG.LauncherConsole]::GetConsoleWindow()
    if ($window -eq [IntPtr]::Zero -or -not [DLSSG.LauncherConsole]::IsWindowVisible($window)) {
        # Windows Terminal 的伪控制台没有可操作的标题栏，转到独立的传统控制台。
        if ($env:DLSSG_LAUNCHER_CONSOLE -eq '1') {
            throw '无法创建带关闭按钮保护的控制台窗口。'
        }
        $arguments = foreach ($entry in $Parameters.GetEnumerator()) {
            if ($entry.Value -is [System.Management.Automation.SwitchParameter]) {
                if ($entry.Value.IsPresent) { '-' + $entry.Key }
            }
            else {
                '-{0} ''{1}''' -f $entry.Key, ([string]$entry.Value).Replace("'", "''")
            }
        }
        # conhost 本身不转发子进程退出代码，用临时文件带回实际脚本结果。
        $resultPath = (New-TemporaryFile).FullName
        try {
            $command = @'
$env:DLSSG_LAUNCHER_CONSOLE = '1'
$global:LASTEXITCODE = 0
$resultCode = 1
try {{
    & '{0}' {1}
    $succeeded = $?
    $resultCode = $global:LASTEXITCODE
    if (-not $succeeded -and $resultCode -eq 0) {{ $resultCode = 1 }}
}}
finally {{ [IO.File]::WriteAllText('{2}', [string]$resultCode) }}
exit $resultCode
'@ -f $ScriptPath.Replace("'", "''"), ($arguments -join ' '), $resultPath.Replace("'", "''")
            $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
            $shellExe = (Get-Process -Id $PID).Path
            Start-Process -FilePath "$env:SystemRoot\System32\conhost.exe" -ArgumentList ('"{0}" -NoLogo -NoProfile -ExecutionPolicy Bypass -EncodedCommand {1}' -f $shellExe, $encodedCommand) -Wait
            $resultCode = [int]::Parse([IO.File]::ReadAllText($resultPath))
        }
        finally { Remove-Item -LiteralPath $resultPath -ErrorAction SilentlyContinue }
        exit $resultCode
    }

    $menu = [DLSSG.LauncherConsole]::GetSystemMenu($window, $false)
    $previousState = [DLSSG.LauncherConsole]::EnableMenuItem($menu, 0xF060, 1)
    if ($previousState -eq [uint32]::MaxValue) {
        throw '无法禁用控制台关闭按钮，已取消启动。'
    }
    [void][DLSSG.LauncherConsole]::DrawMenuBar($window)
    Write-Host '运行期间已禁用窗口关闭按钮，游戏退出并完成清理后恢复。'
    return @{ Window = $window; Menu = $menu; PreviousState = $previousState }
}

function Restore-LauncherConsole {
    param($State)
    if ($null -eq $State) { return }
    [void][DLSSG.LauncherConsole]::EnableMenuItem($State.Menu, 0xF060, ($State.PreviousState -band 3))
    [void][DLSSG.LauncherConsole]::DrawMenuBar($State.Window)
}
