# Shared startup wait. A cancelled or expired launch returns no game process;
# callers then use their normal journal recovery path.
function Get-LauncherProcessNames {
    param([ValidateSet('Wuthering', 'Nte')][string]$GameId, [string]$LauncherPath)
    $entryName = if ($LauncherPath) { [IO.Path]::GetFileNameWithoutExtension($LauncherPath) }
    $names = if ($GameId -eq 'Nte') {
        @($entryName, 'NTELauncher', 'NTEGame', 'NTEUpdate')
    }
    else {
        @($entryName, 'launcher', 'launcher_main', 'launcher_updater')
    }
    return @($names | Where-Object { $_ } | Select-Object -Unique)
}

function Wait-ForGameProcess {
    param(
        [string]$GameProcessName,
        [string[]]$LauncherProcessName,
        [int]$TimeoutSeconds = 120,
        [int]$ClosedGraceSeconds = 3
    )

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $launcherObserved = $false
    $closedTimer = $null
    $lastLauncherSnapshot = ''
    Write-Log "等待游戏启动，监测启动器进程：$($LauncherProcessName -join '、')。"
    while ($true) {
        $processes = @(Get-Process -Name $GameProcessName -ErrorAction SilentlyContinue)
        if ($processes.Count -gt 0) {
            Write-Log "已检测到游戏进程：$GameProcessName"
            return $processes[0]
        }
        $launchers = @(Get-Process -Name $LauncherProcessName -ErrorAction SilentlyContinue)
        if ($launchers.Count -gt 0) {
            $snapshot = (@($launchers | Sort-Object Id | ForEach-Object { '{0}(PID:{1})' -f $_.ProcessName, $_.Id }) -join '、')
            if ($snapshot -ne $lastLauncherSnapshot) { Write-Log "检测到启动器进程：$snapshot" }
            $lastLauncherSnapshot = $snapshot
            $launcherObserved = $true
            $closedTimer = $null
        }
        elseif ($launcherObserved) {
            # No process before the launcher first appears is still startup,
            # not closure. Allow brief bootstrap/update process handoffs too.
            if ($null -eq $closedTimer) {
                Write-Log "暂未检测到启动器和游戏，开始 $ClosedGraceSeconds 秒关闭确认。"
                $closedTimer = [System.Diagnostics.Stopwatch]::StartNew()
                $lastLauncherSnapshot = ''
            }
            if ($closedTimer.Elapsed.TotalSeconds -ge $ClosedGraceSeconds) {
                Write-Log "启动器和游戏连续 $ClosedGraceSeconds 秒均已关闭，将自动清理部署并关闭脚本。"
                return $null
            }
        }
        if ($timer.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
            Write-Log "超过 $TimeoutSeconds 秒未检测到游戏进程，将自动清理部署并关闭脚本。"
            return $null
        }
        Start-Sleep -Seconds 1
    }
}
