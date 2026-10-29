. (Join-Path $PSScriptRoot 'PortableLauncher.ps1')
. (Join-Path $PSScriptRoot 'GameDiscovery.ps1')

function Get-DlssgOnlyDataDirectory {
    param([string]$Root=(Get-LauncherRoot))
    return Join-Path $env:LOCALAPPDATA ('DlssgOnlyLauncher\'+(Split-Path -Leaf (Get-PortableDataDirectory -Root $Root)))
}

function New-DlssgOnlyProfile {
    param($Record,[string]$RenderExecutable,[string]$LaunchExecutable)
    $mode=if ($Record.Platform -eq 'Steam') { 'Steam' } else { 'Direct' }
    $id=if ($mode -eq 'Steam') { 'steam-'+$Record.AppId } else { 'direct-'+[guid]::NewGuid().ToString('N') }
    [void](Resolve-DirectExecutable $Record.GameDirectory $RenderExecutable '渲染程序')
    if (-not $LaunchExecutable) { $LaunchExecutable=$RenderExecutable }
    [void](Resolve-DirectExecutable $Record.GameDirectory $LaunchExecutable '启动程序')
    return [pscustomobject]@{Id=$id;Name=$Record.Name;Platform=$Record.Platform;LaunchMode=$mode;AppId=$Record.AppId;
        GameDirectory=$Record.GameDirectory;RenderExecutable=$RenderExecutable;LaunchExecutable=$LaunchExecutable;ProxyDll=1}
}

function Get-DlssgOnlyModel {
    param($Profile,[string]$SteamPath='', [string]$Root=(Get-LauncherRoot))
    $model=[pscustomobject]@{Profile=$Profile;GameId='';GameDirectory=[string]$Profile.GameDirectory;SteamPath=$SteamPath;
        Executable='';Ready=$false;Recovery=$false;Status='';Journal=''}
    try {
        $model.GameId=Get-PortableGameId $Profile
        if ([string]$Profile.ProxyDll -notmatch '^[1-6]$') { throw '代理编号必须为1–6。' }
        $model.Journal=Join-Path $Root ('.state\'+$model.GameId+'\active.json')
        if ([IO.File]::Exists($model.Journal)) {
            $journal=[IO.File]::ReadAllText($model.Journal) | ConvertFrom-Json
            if ($journal.Version -ne 1 -or $journal.GameId -ne $model.GameId) { throw '部署记录格式无效，已保留。' }
            if ($null -eq $journal.PSObject.Properties['DeploymentMode'] -or $journal.DeploymentMode -ne 'DlssgOnly') { throw '发现原启动器部署，请使用原启动器恢复原状。' }
            $suffix=Split-Path -Parent $Profile.RenderExecutable
            $render=[IO.Path]::GetFullPath($journal.Render).TrimEnd('\')
            if ($suffix) {
                if (-not $render.EndsWith('\'+$suffix,[StringComparison]::OrdinalIgnoreCase)) { throw '恢复记录与原渲染程序不符，请使用原游戏设置。' }
                $model.GameDirectory=$render.Substring(0,$render.Length-$suffix.Length-1)
            } else { $model.GameDirectory=$render }
            $model.Executable=Resolve-DirectExecutable $model.GameDirectory $Profile.RenderExecutable '原渲染程序'
            $model.Recovery=$true; $model.Status='发现未完成部署，请先恢复原状。'
            return $model
        }
        if ($Profile.LaunchMode -eq 'Steam') {
            if (-not [IO.File]::Exists($SteamPath)) { throw '请设置 steam.exe。' }
            if (-not [IO.Directory]::Exists($model.GameDirectory)) {
                $paths=@(Find-SteamGameDirectory $SteamPath $Profile.AppId)
                if ($paths.Count -ne 1) { throw '未找到唯一安装位置，请重新选择游戏目录。' }
                $model.GameDirectory=$paths[0]
            }
            $match=Resolve-SteamAppId $model.GameDirectory $SteamPath
            if ($match.AppId -ne $Profile.AppId) { throw '目录的AppID与游戏设置不符。' }
        }
        $model.Executable=Resolve-DirectExecutable $model.GameDirectory $Profile.RenderExecutable '渲染程序'
        $renderDirectory=Split-Path -Parent $model.Executable
        foreach ($directory in @(Get-ChildItem -LiteralPath (Join-Path $Root '.state') -Directory -ErrorAction SilentlyContinue)) {
            if ($directory.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '部署状态目录异常，请保留现场。' }
            $otherPath=Join-Path $directory.FullName 'active.json'
            if (-not [IO.File]::Exists($otherPath)) { continue }
            $other=[IO.File]::ReadAllText($otherPath) | ConvertFrom-Json
            if ([IO.Path]::GetFullPath($other.Render).TrimEnd('\') -ieq $renderDirectory.TrimEnd('\')) { throw '该目录仍有其他游戏设置的部署，请使用原入口恢复原状。' }
        }
        Assert-GameOrdinaryExecutable $model.Executable
        if (-not (Test-GameX64Executable $model.Executable)) { throw '渲染程序缺失或不是x64游戏程序，请重新选择。' }
        if ($Profile.LaunchMode -eq 'Direct') {
            $launch=Resolve-DirectExecutable $model.GameDirectory $Profile.LaunchExecutable '启动程序'
            if (-not [IO.File]::Exists($launch)) { throw '启动程序不存在，请重新选择。' }
            Assert-GameOrdinaryExecutable $launch
        }
        if (@(Get-SteamGameProcesses $model.Executable).Count) { throw '游戏正在运行，请先退出游戏。' }
        $project=Split-Path -Parent $Root
        $proxy=@('version.dll','alternatives\winmm.dll','alternatives\dbghelp.dll','alternatives\dinput8.dll','alternatives\dxgi.dll','alternatives\d3d12.dll')[[int]$Profile.ProxyDll-1]
        foreach ($file in @($proxy,'dlssg_sm86.ini','dlssg_sm86\runtime\nvngx_dlssg.dll','dlssg_sm86\backend\sm86_backend.dll',
            'dlssg_sm86\kernels\fg_kernels.dll','dlssg_sm86\xefg\xefg_kernels.dll','dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll')) {
            if (-not [IO.File]::Exists((Join-Path $project $file))) { throw "组件缺失，请重新解压完整启动器：$file" }
        }
        $model.Ready=$true; $model.Status='可以启动；游戏退出后自动恢复。兼容性需在游戏内确认。'
    } catch { $model.Status=$_.Exception.Message }
    return $model
}

function Write-DlssgOnlyLaunchConfig {
    param($Model,[string]$Directory)
    $p=$Model.Profile
    foreach ($value in @($p.Name,$Model.GameDirectory,$p.RenderExecutable,$p.LaunchExecutable)) {
        if ([string]::IsNullOrWhiteSpace($value) -or $value -match '[\r\n]') { throw '游戏设置为空或含换行。' }
    }
    if ($Model.SteamPath -match '[\r\n]') { throw 'Steam路径含换行。' }
    [void](Get-PortableGameId $p)
    [void](Resolve-DirectExecutable $Model.GameDirectory $p.RenderExecutable '渲染程序')
    [void](Resolve-DirectExecutable $Model.GameDirectory $p.LaunchExecutable '启动程序')
    if ([string]$p.ProxyDll -notmatch '^[1-6]$') { throw '代理编号必须为1–6。' }
    $section=if ($p.LaunchMode -eq 'Steam') { 'SteamUE5:desktop' } else { 'DirectUE:desktop' }
    $text=@"
[Steam]
启动器路径=$($Model.SteamPath)
[$section]
游戏名称=$($p.Name)
游戏目录=$($Model.GameDirectory)
渲染程序=$($p.RenderExecutable)
代理DLL=$($p.ProxyDll)
"@
    if ($p.LaunchMode -eq 'Direct') { $text+="`r`n启动程序=$($p.LaunchExecutable)`r`n" }
    foreach ($path in @($Directory,(Join-Path $Directory 'launch.ini'))) {
        $item=Get-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        if ($null -ne $item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw '运行请求路径不能是链接。' }
    }
    [void][IO.Directory]::CreateDirectory($Directory)
    $path=Join-Path $Directory 'launch.ini'
    [IO.File]::WriteAllText($path,$text,[Text.UTF8Encoding]::new($true))
    return $path
}
