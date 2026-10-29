# Steam-specific configuration and process polling. Windows PowerShell 5.1+.
function Get-SteamLibraryDirectories {
    param([string]$SteamExecutable)
    $steamApps = Join-Path (Split-Path -Parent $SteamExecutable) 'steamapps'
    $directories = @($steamApps)
    $folders = Join-Path $steamApps 'libraryfolders.vdf'
    if (Test-Path -LiteralPath $folders -PathType Leaf) {
        foreach ($match in [regex]::Matches([IO.File]::ReadAllText($folders), '(?m)^\s*"path"\s+"((?:\\.|[^"\\])*)"\s*$')) {
            $library = $match.Groups[1].Value.Replace('\\','\').Replace('\"','"')
            if ([IO.Path]::IsPathRooted($library)) { $directories += Join-Path $library 'steamapps' }
        }
    }
    return @($directories | ForEach-Object { [IO.Path]::GetFullPath($_).TrimEnd('\') } | Sort-Object -Unique)
}

function Find-SteamGameDirectory {
    param([string]$SteamExecutable, [ValidatePattern('^[1-9][0-9]*$')][string]$AppId)
    $found = @()
    foreach ($apps in @(Get-SteamLibraryDirectories $SteamExecutable)) {
        $manifest = Join-Path $apps "appmanifest_$AppId.acf"
        if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { continue }
        $dirs = [regex]::Matches([IO.File]::ReadAllText($manifest), '(?m)^\s*"installdir"\s+"((?:\\.|[^"\\])*)"\s*$')
        if ($dirs.Count -ne 1) { continue }
        $install = $dirs[0].Groups[1].Value.Replace('\\','\').Replace('\"','"')
        if ([IO.Path]::IsPathRooted($install)) { continue }
        $common = Join-Path $apps 'common'
        $game = [IO.Path]::GetFullPath((Join-Path $common $install))
        if (-not $game.StartsWith($common.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)) { continue }
        if (Test-Path -LiteralPath $game -PathType Container) {
            $detected = Resolve-SteamAppId -GameDirectory $game -SteamExecutable $SteamExecutable
            if ($detected.AppId -eq $AppId) { $found += $game }
        }
    }
    return @($found | Sort-Object -Unique)
}

function Resolve-SteamAppId {
    param([string]$GameDirectory, [string]$SteamExecutable)
    $game = [IO.Path]::GetFullPath($GameDirectory).TrimEnd('\')
    $libraries = @{}
    # The game's own library is authoritative, including libraries not currently registered.
    $parent = Split-Path -Parent $game
    if ((Split-Path -Leaf $parent) -ieq 'common') {
        $apps = Split-Path -Parent $parent
        if ((Split-Path -Leaf $apps) -ieq 'steamapps') { $libraries[$apps] = $true }
    }
    foreach ($apps in @(Get-SteamLibraryDirectories $SteamExecutable)) { $libraries[$apps] = $true }
    $matchesFound = @()
    foreach ($apps in $libraries.Keys) {
        if (-not (Test-Path -LiteralPath $apps -PathType Container)) { continue }
        foreach ($manifest in @(Get-ChildItem -LiteralPath $apps -Filter 'appmanifest_*.acf' -File -ErrorAction Stop)) {
            $text = [IO.File]::ReadAllText($manifest.FullName)
            $dirs = [regex]::Matches($text, '(?m)^\s*"installdir"\s+"((?:\\.|[^"\\])*)"\s*$')
            if ($dirs.Count -ne 1) { continue }
            $install = $dirs[0].Groups[1].Value.Replace('\\','\').Replace('\"','"')
            if ([IO.Path]::IsPathRooted($install)) { continue }
            $common = Join-Path $apps 'common'
            $candidate = [IO.Path]::GetFullPath((Join-Path $common $install)).TrimEnd('\')
            if (-not $candidate.StartsWith($common.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -or $candidate -ine $game) { continue }
            $ids = [regex]::Matches($text, '(?m)^\s*"appid"\s+"([1-9][0-9]*)"\s*$')
            if ($ids.Count -ne 1 -or $manifest.Name -ine ("appmanifest_{0}.acf" -f $ids[0].Groups[1].Value)) {
                throw "匹配游戏的 Steam 清单无效，请检查：$($manifest.FullName)"
            }
            $matchesFound += [pscustomobject]@{ AppId=$ids[0].Groups[1].Value; Manifest=$manifest.FullName }
        }
    }
    # Duplicate references to the same library are harmless; distinct matching files are ambiguous.
    $matchesFound = @($matchesFound | Sort-Object Manifest -Unique)
    if ($matchesFound.Count -ne 1) {
        $details = ($matchesFound | ForEach-Object { "$($_.AppId): $($_.Manifest)" }) -join '; '
        throw "无法唯一识别游戏 AppID（匹配 $($matchesFound.Count) 项）：$game。请通过 Steam 安装/识别该目录，或显式填写 -AppId。$details"
    }
    return $matchesFound[0]
}

function Get-SteamUE5Settings {
    param([string]$AppId, [System.Collections.IDictionary]$Overrides = @{},
        [string]$Profile,
        [string]$Path = (Join-Path (Get-LauncherRoot) '启动配置.ini'))
    if ($AppId -and $AppId -notmatch '^[1-9][0-9]*$') { throw 'AppId 必须是正整数。' }
    $groups = @((Read-LauncherConfigText $Path) -split '\r\n|\n|\r' | ForEach-Object {
        if ($_.Trim() -match '^\[(SteamUE5(?::[^\[\]]+)?)\]$') { $Matches[1].Trim() }
    })
    if ($Profile) {
        $group = if ($Profile -ieq 'SteamUE5' -or $Profile -like 'SteamUE5:*') { $Profile } else { "SteamUE5:$Profile" }
    }
    elseif ($AppId -and "SteamUE5:$AppId" -in $groups) { $group = "SteamUE5:$AppId" }
    elseif ('SteamUE5' -in $groups) { $group = 'SteamUE5' }
    elseif ($groups.Count -eq 1) { $group = $groups[0] }
    else { throw "请用 -Profile 选择游戏配置分组：$($groups -join '、')。" }
    $section = Read-LauncherConfigSection -Name $group -Path $Path
    $steam = Read-LauncherConfigSection -Name Steam -Path $Path
    $values = @{}
    $keys = @('GameName','GameDirectory','RenderExecutable')
    if (-not ($Overrides.ContainsKey('DlssgOnly') -and $Overrides.DlssgOnly)) { $keys += @('ConfigDirectory','EngineConfigSource') }
    foreach ($key in $keys) {
        $value = if ($Overrides.ContainsKey($key)) { [string]$Overrides[$key] } else { Get-LauncherText $section $key $group }
        $values[$key] = if ($key -in @('GameName','RenderExecutable')) { $value } else { Resolve-LauncherConfigPath -Value $value -Name "$group.$key" }
    }
    # RenderExecutable is relative to the game root, unlike other INI paths.
    $relative = Get-LauncherText $section RenderExecutable $group
    if ([IO.Path]::IsPathRooted($relative)) { throw '渲染程序必须是游戏目录内的相对 EXE 路径。' }
    $values.RenderExecutable = [IO.Path]::GetFullPath((Join-Path $values.GameDirectory $relative))
    if (-not $values.RenderExecutable.StartsWith($values.GameDirectory.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetExtension($values.RenderExecutable) -ine '.exe') { throw '渲染程序必须是游戏目录内的 EXE。' }
    $proxy = if ($Overrides.ContainsKey('ProxyDll')) { [string]$Overrides['ProxyDll'] } else { Get-LauncherText $section ProxyDll $group -Optional }
    if (-not $proxy) { $proxy = '1' }
    if ($proxy -notmatch '^[1-6]$') { throw '代理DLL / ProxyDll 只接受整数 1–6。' }
    $values.ProxyDll = [int]$proxy
    $values.ProxyName = @('version.dll','winmm.dll','dbghelp.dll','dinput8.dll','dxgi.dll','d3d12.dll')[$values.ProxyDll - 1]
    $steamPath = Get-LauncherText $steam LauncherPath Steam -Optional
    if (-not $steamPath) {
        $registry = Get-ItemProperty 'HKCU:\Software\Valve\Steam' -ErrorAction SilentlyContinue
        if ($null -ne $registry -and $null -ne $registry.PSObject.Properties['SteamExe']) { $steamPath = $registry.SteamExe }
    }
    $restoring = ($Overrides.ContainsKey('RestoreDeployment') -and $Overrides['RestoreDeployment']) -or
        ($Overrides.ContainsKey('RestoreEngineConfig') -and $Overrides['RestoreEngineConfig'])
    if (-not $steamPath -and -not ($restoring -and $AppId)) { throw '未找到 Steam，请填写 [Steam] 启动器路径。' }
    $values.LauncherPath = Resolve-LauncherConfigPath -Value $steamPath -Name 'Steam.LauncherPath'
    $values.Profile = $group
    if ($AppId) { $values.AppId = $AppId; $values.AppIdSource = '命令行指定' }
    else {
        $detected = Resolve-SteamAppId -GameDirectory $values.GameDirectory -SteamExecutable $values.LauncherPath
        $values.AppId = $detected.AppId
        $values.AppIdSource = "自动读取 $($detected.Manifest)"
    }
    return $values
}

function Get-SteamGameProcesses {
    param([string]$Executable)
    $name = [IO.Path]::GetFileNameWithoutExtension($Executable)
    foreach ($process in @(Get-Process -Name $name -ErrorAction SilentlyContinue)) {
        try { $path = $process.Path }
        catch { throw "无法核实游戏进程路径（PID $($process.Id)），保留部署：$_" }
        if (-not $path) {
            # Processes can disappear between enumeration and inspection.
            if ($process.HasExited) { continue }
            throw "无法核实游戏进程路径（PID $($process.Id)），保留部署。"
        }
        if ([IO.Path]::GetFullPath($path) -ieq [IO.Path]::GetFullPath($Executable)) { $process }
    }
}

function Assert-SteamGameStopped {
    param([string]$Executable)
    if (@(Get-SteamGameProcesses -Executable $Executable).Count) { throw '目标游戏仍在运行，请先退出游戏。' }
}

function Wait-SteamGameSession {
    param([string]$Executable, [int]$TimeoutSeconds = 120)
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while (@(Get-SteamGameProcesses -Executable $Executable).Count -eq 0) {
        if ($timer.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
            Write-Log "超过 $TimeoutSeconds 秒未检测到游戏，将自动清理部署并关闭脚本。"
            return
        }
        Start-Sleep -Seconds 1
    }
    Write-Log "已检测到游戏：$Executable"
    while (@(Get-SteamGameProcesses -Executable $Executable).Count -gt 0) { Start-Sleep -Seconds 1 }
    Write-Log '目标游戏全部退出，开始恢复部署。'
}
