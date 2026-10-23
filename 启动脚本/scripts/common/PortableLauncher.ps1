. (Join-Path $PSScriptRoot 'DirectLauncher.ps1')
. (Join-Path $PSScriptRoot 'UnrealQuality.ps1')
# Data/model layer for the portable desktop launcher. Windows PowerShell 5.1.
function Get-PortableDataDirectory {
    param([string]$Root = (Get-LauncherRoot))
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $key = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes([IO.Path]::GetFullPath($Root).ToLowerInvariant())))).Replace('-','').Substring(0,16) }
    finally { $sha.Dispose() }
    return Join-Path $env:LOCALAPPDATA "DlssgSteamLauncher\$key"
}

function Read-PortablePreferences {
    param([string]$Directory = (Get-PortableDataDirectory))
    $path = Join-Path $Directory 'preferences.json'
    if (-not [IO.File]::Exists($path)) { return @{ SteamPath=''; Profiles=@() } }
    $data = [IO.File]::ReadAllText($path) | ConvertFrom-Json
    if ($data.Version -ne 1) { throw '本机偏好格式不受支持，请保留 preferences.json 并联系配置提供者。' }
    return @{ SteamPath=[string]$data.SteamPath; Profiles=@($data.Profiles) }
}

function Save-PortablePreferences {
    param($Preferences, [string]$Directory = (Get-PortableDataDirectory))
    $path = Join-Path $Directory 'preferences.json'
    # Keep device paths outside the portable package. Do not export this directory.
    [void][IO.Directory]::CreateDirectory($Directory)
    foreach ($candidate in @($Directory,$path)) {
        $item = Get-Item -LiteralPath $candidate -Force -ErrorAction SilentlyContinue
        if ($null -ne $item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw '本机偏好路径不能是链接。' }
    }
    $temporary = Join-Path $Directory ([guid]::NewGuid().ToString('N') + '.tmp')
    try {
        $data = @{ Version=1; SteamPath=$Preferences.SteamPath; Profiles=@($Preferences.Profiles) }
        [IO.File]::WriteAllText($temporary, ($data | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($true))
        if ([IO.File]::Exists($path)) { [IO.File]::Replace($temporary,$path,[System.Management.Automation.Language.NullString]::Value) }
        else { [IO.File]::Move($temporary,$path) }
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}

function Get-PortableSteamPath {
    param($Preferences)
    if ($Preferences.SteamPath -and [IO.File]::Exists($Preferences.SteamPath)) { return $Preferences.SteamPath }
    $registry = Get-ItemProperty 'HKCU:\Software\Valve\Steam' -ErrorAction SilentlyContinue
    if ($null -ne $registry -and $null -ne $registry.PSObject.Properties['SteamExe']) { return [string]$registry.SteamExe }
    return ''
}

function Get-PortableProfiles {
    param($Preferences, [string]$Root = (Get-LauncherRoot))
    # PS 5.1 emits a JSON array as a single pipeline object. Assign first so
    # the array is not nested inside @(...), which breaks saved-profile merging.
    $presets = [IO.File]::ReadAllText((Join-Path $Root 'presets\SteamUE5.json')) | ConvertFrom-Json
    $presets = @($presets)
    $presetIds = @($presets | ForEach-Object { $_.Id })
    $profiles = @()
    foreach ($preset in $presets) {
        $saved = @($Preferences.Profiles | Where-Object Id -eq $preset.Id)
        if ($saved.Count -gt 1) { throw '本机存在重复游戏配置。' }
        $profiles += if ($saved.Count) { $saved[0] } else {
            [pscustomobject]@{ Id=$preset.Id; Name=$preset.Name; AppId=$preset.AppId; RenderExecutable=$preset.RenderExecutable;
                ConfigDirectory=$preset.ConfigDirectory; EngineConfigSource=$preset.EngineConfigSource; GameDirectory=''; ProxyDll=1 }
        }
    }
    $profiles += @($Preferences.Profiles | Where-Object { $_.Id -notin $presetIds })
    foreach ($profile in $profiles) {
        Initialize-PortableQuality $profile
        Initialize-PortableLaunchMode $profile
        if ($profile.Id -notmatch '^[A-Za-z0-9_-]+$' -or ($profile.LaunchMode -eq 'Steam' -and $profile.AppId -notmatch '^[1-9][0-9]*$')) { throw '游戏预设标识无效。' }
        if ([string]$profile.ProxyDll -notmatch '^[1-6]$') { throw '本机偏好的代理编号无效。' }
    }
    return $profiles
}

function Set-PortableProfile {
    param($Preferences, $Profile)
    $Preferences.Profiles = @($Preferences.Profiles | Where-Object Id -ne $Profile.Id) + @($Profile)
}

function Get-PortableGameModel {
    param($Profile, [string]$SteamPath, [string]$Root = (Get-LauncherRoot), $EngineDetection=$null)
    $gameId=Get-PortableGameId $Profile
    $model = [pscustomobject]@{ Profile=$Profile;GameId=$gameId;LaunchExecutable=''; SteamPath=$SteamPath; GameDirectory=[string]$Profile.GameDirectory;
        ConfigDirectory=(Resolve-LauncherConfigPath $Profile.ConfigDirectory '游戏配置目录' -Root $Root);
        EngineConfigSource=(Resolve-LauncherConfigPath $Profile.EngineConfigSource '配置源' -Root $Root);
        Executable=''; EngineMajor=0; EngineSource='无法确定'; Ready=$false; Recovery=$false; Status='正在查找游戏…'; Journal=(Join-Path $Root ".state\$gameId\active.json") }
    try {
        if ([IO.File]::Exists($model.Journal)) {
            $journal = [IO.File]::ReadAllText($model.Journal) | ConvertFrom-Json
            if ($journal.GameId -ne $gameId -or $journal.Version -ne 1) { throw '恢复记录格式不受支持。' }
            $suffix = Split-Path -Parent $Profile.RenderExecutable
            $render = [IO.Path]::GetFullPath($journal.Render).TrimEnd('\')
            if ($suffix) {
                if (-not $render.EndsWith('\'+$suffix, [StringComparison]::OrdinalIgnoreCase)) { throw '原部署路径与游戏预设不同，请使用原预设恢复。' }
                $model.GameDirectory = $render.Substring(0,$render.Length-$suffix.Length-1)
            } else { $model.GameDirectory=$render }
            if ([IO.Path]::GetFileName($journal.Engine) -ine 'Engine.ini') { throw '恢复记录配置路径无效。' }
            $model.ConfigDirectory = Split-Path -Parent $journal.Engine
            $model.Recovery = $true
            $model.Status = '发现上次未完成的部署，请先点击“恢复原状”。'
            return $model
        }
        if ($Profile.LaunchMode -eq 'Steam') {
            if (-not $SteamPath -or -not [IO.File]::Exists($SteamPath)) { throw '未找到 Steam，请在高级设置中选择 steam.exe。' }
            if ($model.GameDirectory -and [IO.Directory]::Exists($model.GameDirectory)) {
                $match = Resolve-SteamAppId $model.GameDirectory $SteamPath
                if ($match.AppId -ne $Profile.AppId) { throw '选择的文件夹属于另一款游戏。' }
            } else {
                $games = @(Find-SteamGameDirectory $SteamPath $Profile.AppId)
                if ($games.Count -ne 1) { throw '未找到唯一安装位置，请点击“选择游戏文件夹”。' }
                $model.GameDirectory = $games[0]
            }
        } elseif (-not [IO.Directory]::Exists($model.GameDirectory)) { throw '未找到游戏目录，请选择当前游戏文件夹。' }
        $executable = Join-Path $model.GameDirectory $Profile.RenderExecutable
        if (-not [IO.File]::Exists($executable)) { throw '游戏程序与预设不符，请选择正确的游戏文件夹。' }
        $executable=Resolve-DirectExecutable $model.GameDirectory $Profile.RenderExecutable '渲染程序'
        $model.Executable=$executable
        if ($Profile.LaunchMode -eq 'Direct') {
            $model.LaunchExecutable=Resolve-DirectExecutable $model.GameDirectory $Profile.LaunchExecutable '启动程序'
            if (-not [IO.File]::Exists($model.LaunchExecutable)) { throw '启动EXE缺失，请选择正确的游戏文件夹。' }
        }
        if ($null -eq $EngineDetection) { $EngineDetection=Get-PortableSavedEngine $Profile }
        if ($null -ne $EngineDetection) { $model.EngineMajor=[int]$EngineDetection.Major; $model.EngineSource=[string]$EngineDetection.Source }
        Initialize-PortableQuality $Profile
        if ($Profile.EngineOverride -ne 'Auto') { $model.EngineMajor=[int]$Profile.EngineOverride.Substring(2); $model.EngineSource='手动校准' }
        $quality=Resolve-PortableQuality $Profile $EngineDetection -Root $Root
        $model.EngineConfigSource=$quality.Path
        if (@(Get-SteamGameProcesses -Executable $executable).Count) { throw '游戏正在运行，请先退出游戏，再使用启动器。' }
        if (-not [IO.Directory]::Exists($model.ConfigDirectory)) { throw '请先原版启动游戏一次，然后点击“重新查找”。' }
        if (-not [IO.File]::Exists($model.EngineConfigSource)) { throw '画质配置源缺失，请重新解压完整启动器包。' }
        $project = Split-Path -Parent $Root
        $proxy = @('version.dll','alternatives\winmm.dll','alternatives\dbghelp.dll','alternatives\dinput8.dll','alternatives\dxgi.dll','alternatives\d3d12.dll')[[int]$Profile.ProxyDll-1]
        foreach ($relative in @($proxy,'dlssg_sm86.ini','dlssg_sm86\runtime\nvngx_dlssg.dll','dlssg_sm86\backend\sm86_backend.dll',
            'dlssg_sm86\kernels\fg_kernels.dll','dlssg_sm86\xefg\xefg_kernels.dll','dlssg_sm86\sr\sr_kernels.dll','dlssg_sm86\sr\nvngx_dlss.dll')) {
            if (-not [IO.File]::Exists((Join-Path $project $relative))) { throw "启动器组件缺失，请重新解压完整包：$relative" }
        }
        $model.Ready = $true
        $model.Status = '可以启动 · 游戏退出后自动恢复'
    } catch { $model.Status = $_.Exception.Message }
    return $model
}

function Write-PortableLaunchConfig {
    param($Model, [string]$Directory)
    Initialize-PortableLaunchMode $Model.Profile
    if ($Model.Profile.LaunchMode -eq 'Steam' -and -not $Model.Recovery -and [string]::IsNullOrWhiteSpace($Model.SteamPath)) { throw '请先选择 Steam。' }
    if ($Model.SteamPath -match '[\r\n]') { throw 'Steam 路径无效。' }
    foreach ($value in @($Model.Profile.Name,$Model.GameDirectory,$Model.Profile.RenderExecutable,$Model.ConfigDirectory,$Model.EngineConfigSource)) {
        if ([string]::IsNullOrWhiteSpace($value) -or $value -match '[\r\n]') { throw '游戏设置为空或含换行，请重新选择路径。' }
    }
    if ([string]$Model.Profile.ProxyDll -notmatch '^[1-6]$') { throw '代理编号必须为 1–6。' }
    [void][IO.Directory]::CreateDirectory($Directory)
    $path = Join-Path $Directory 'launch.ini'
    $section=if ($Model.Profile.LaunchMode -eq 'Direct') { 'DirectUE:desktop' } else { 'SteamUE5:desktop' }
    $text = @"
[Steam]
启动器路径=$($Model.SteamPath)
[$section]
游戏名称=$($Model.Profile.Name)
游戏目录=$($Model.GameDirectory)
渲染程序=$($Model.Profile.RenderExecutable)
游戏配置目录=$($Model.ConfigDirectory)
配置源=$($Model.EngineConfigSource)
代理DLL=$($Model.Profile.ProxyDll)
"@
    if ($Model.Profile.LaunchMode -eq 'Direct') {
        [void](Resolve-DirectExecutable $Model.GameDirectory $Model.Profile.LaunchExecutable '启动程序')
        if ($Model.Profile.LaunchExecutable -match '[\r\n]') { throw '启动程序路径含换行。' }
        $text+="`r`n启动程序=$($Model.Profile.LaunchExecutable)`r`n"
    }
    [IO.File]::WriteAllText($path,$text,[Text.UTF8Encoding]::new($true))
    return $path
}

function New-PortableWorkerCommand {
    param([string]$ScriptPath, [string]$ConfigPath, [string]$AppId, [string]$ResultPath,
        [ValidateSet('Launch','Restore','Check')][string]$Action,
        [ValidateSet('Steam','Direct')][string]$LaunchMode='Steam',[string]$GameKey)
    if ($LaunchMode -eq 'Steam' -and $AppId -notmatch '^[1-9][0-9]*$') { throw 'Invalid AppID.' }
    if ($LaunchMode -eq 'Direct' -and $GameKey -notmatch '^[a-f0-9]{32}$') { throw 'Invalid direct game key.' }
    $identity=if ($LaunchMode -eq 'Direct') { "-LaunchMode Direct -GameKey '$GameKey'" } else { "-AppId '$AppId'" }
    $flag = switch ($Action) { 'Restore' { '-RestoreDeployment' } 'Check' { '-DryRun' } default { '' } }
    $command = @'
$global:LASTEXITCODE = 0
$code = 1
$failure = ''
try {
    & '__SCRIPT__' -LauncherConfigPath '__CONFIG__' -Profile desktop __IDENTITY__ __FLAG__
    $succeeded = $?
    $code = $global:LASTEXITCODE
    if (-not $succeeded -and $code -eq 0) { $code = 1 }
} catch { $failure = $_.Exception.Message; $code = 1 }
finally {
    $result = @{ ExitCode=$code; Error=$failure } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText('__RESULT__', $result, [Text.UTF8Encoding]::new($true))
}
if ($code -ne 0 -and __DETACHED__) {
    # The desktop UI has exited after handing off launch; report failures here.
    Add-Type -AssemblyName System.Windows.Forms
    [void][System.Windows.Forms.MessageBox]::Show(('启动或恢复未完成，请重新打开启动器查看日志或恢复原状。'+"`r`n"+$failure),'DLSSG 启动未完成','OK','Warning')
}
exit $code
'@
    $detached = if ($Action -eq 'Launch') { '$true' } else { '$false' }
    return $command.Replace('__SCRIPT__',$ScriptPath.Replace("'","''")).Replace('__CONFIG__',$ConfigPath.Replace("'","''")).Replace('__IDENTITY__',$identity).Replace('__RESULT__',$ResultPath.Replace("'","''")).Replace('__FLAG__',$flag).Replace('__DETACHED__',$detached)
}
