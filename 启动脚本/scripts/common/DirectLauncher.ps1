# Direct UE launch settings and shared mode identity. No Steam registry/manifest dependency.
function Initialize-PortableLaunchMode {
    param($Profile)
    if ($null -eq $Profile.PSObject.Properties['LaunchMode']) { $Profile | Add-Member NoteProperty LaunchMode Steam }
    if ($null -eq $Profile.PSObject.Properties['LaunchExecutable']) { $Profile | Add-Member NoteProperty LaunchExecutable ([string]$Profile.RenderExecutable) }
    if ($Profile.LaunchMode -notin @('Steam','Direct')) { throw '启动方式无效。' }
    if ($Profile.LaunchMode -eq 'Direct' -and $Profile.Id -notmatch '^direct-[a-f0-9]{32}$') { throw '直接启动游戏的本机标识无效。' }
}

function Get-PortableGameId {
    param($Profile)
    Initialize-PortableLaunchMode $Profile
    if ($Profile.LaunchMode -eq 'Direct') { return $Profile.Id }
    if ($Profile.AppId -notmatch '^[1-9][0-9]*$') { throw 'Steam AppID无效。' }
    return "steam-$($Profile.AppId)"
}

function Resolve-DirectExecutable {
    param([string]$GameDirectory,[string]$Relative,[string]$Description='程序')
    if ([string]::IsNullOrWhiteSpace($Relative) -or [IO.Path]::IsPathRooted($Relative)) { throw "$Description 必须是游戏目录内的相对EXE路径。" }
    $base=[IO.Path]::GetFullPath($GameDirectory).TrimEnd('\')+'\'
    $path=[IO.Path]::GetFullPath((Join-Path $base $Relative))
    if (-not $path.StartsWith($base,[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetExtension($path) -ine '.exe') { throw "$Description 必须是游戏目录内的EXE。" }
    return $path
}

function Get-DirectUESettings {
    param([Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{32}$')][string]$GameKey,
        [System.Collections.IDictionary]$Overrides=@{},[string]$Profile='desktop',
        [string]$Path=(Join-Path (Get-LauncherRoot) '启动配置.ini'))
    $group=if ($Profile -like 'DirectUE:*') { $Profile } else { "DirectUE:$Profile" }
    $section=Read-LauncherConfigSection -Name $group -Path $Path
    $values=@{Profile=$group;GameId="direct-$GameKey";LauncherPath=''}
    $keys = @('GameName','GameDirectory')
    if (-not ($Overrides.ContainsKey('DlssgOnly') -and $Overrides.DlssgOnly)) { $keys += @('ConfigDirectory','EngineConfigSource') }
    foreach ($key in $keys) {
        $value=if ($Overrides.ContainsKey($key)) { [string]$Overrides[$key] } else { Get-LauncherText $section $key $group }
        $values[$key]=if ($key -eq 'GameName') { $value } else { Resolve-LauncherConfigPath $value "$group.$key" }
    }
    $values.RenderExecutable=Resolve-DirectExecutable $values.GameDirectory (Get-LauncherText $section RenderExecutable $group) '渲染程序'
    $values.LauncherPath=Resolve-DirectExecutable $values.GameDirectory (Get-LauncherText $section LaunchExecutable $group) '启动程序'
    $proxy=if ($Overrides.ContainsKey('ProxyDll')) { [string]$Overrides.ProxyDll } else { Get-LauncherText $section ProxyDll $group -Optional }
    if (-not $proxy) { $proxy='1' }
    if ($proxy -notmatch '^[1-6]$') { throw '代理DLL / ProxyDll 只接受整数 1–6。' }
    $values.ProxyDll=[int]$proxy
    $values.ProxyName=@('version.dll','winmm.dll','dbghelp.dll','dinput8.dll','dxgi.dll','d3d12.dll')[$values.ProxyDll-1]
    return $values
}

function Start-ConfiguredUEGame {
    param([ValidateSet('Steam','Direct')][string]$LaunchMode,[string]$Executable,[string]$AppId)
    if ($LaunchMode -eq 'Direct') {
        Start-Process -FilePath $Executable -WorkingDirectory (Split-Path -Parent $Executable) -WindowStyle Normal
    } else {
        Start-Process -FilePath $Executable -WorkingDirectory (Split-Path -Parent $Executable) -ArgumentList @('-applaunch',$AppId) -WindowStyle Hidden
    }
}
