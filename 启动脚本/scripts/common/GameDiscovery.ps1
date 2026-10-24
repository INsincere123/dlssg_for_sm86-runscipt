# Local installation discovery and renderer suggestions. Never deploys or launches.
function Test-DiscoveryCancelled {
    param($Cancellation)
    return ($null -ne $Cancellation -and $Cancellation.IsCancellationRequested)
}

function Test-GameX64Executable {
    param([string]$Path)
    $stream=$null; $reader=$null
    try {
        $stream=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
        $reader=[IO.BinaryReader]::new($stream)
        if ($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5A4D) { return $false }
        $stream.Position=60; $offset=$reader.ReadInt32()
        if ($offset -lt 64 -or $offset -gt $stream.Length-26) { return $false }
        $stream.Position=$offset
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) { return $false }
        $stream.Position=$offset+22; $flags=$reader.ReadUInt16()
        return (($flags -band 2) -ne 0 -and ($flags -band 0x2000) -eq 0)
    } catch { return $false }
    finally { if ($null -ne $reader) { $reader.Dispose() } elseif ($null -ne $stream) { $stream.Dispose() } }
}

function Assert-GameOrdinaryExecutable {
    param([string]$Path)
    $current=[IO.Path]::GetFullPath($Path)
    while ($current) {
        $item=Get-Item -LiteralPath $current -Force -ErrorAction Stop
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "游戏程序和上级目录不能是链接：$current" }
        $parent=Split-Path -Parent $current
        if ($parent -eq $current) { break }; $current=$parent
    }
}

function Test-GameAuxiliaryExecutable {
    param([string]$Name)
    $role=[IO.Path]::GetFileNameWithoutExtension($Name) -replace '(?i)-(shipping|development|debuggame|debug|test)$','' -replace '(?i)-(win64|win32|x64|x86)$',''
    if ($role -match '^(?i:crashreportclient.*|unitycrashhandler\d*|crashpad_handler|crashsender\d*|unins\d*|setup\d*|install|uninstall|vulkaninfo\d*|nvidia-smi|d3d12stateobjectcompiler|d3d12warpcompiler|shadercompileworker|unrealpak|ue4editor|unrealeditor|easyanticheat.*|beservice.*|start_protected_game|steamwebhelper|steamservice|steamerrorreporter|cefsubprocess|qtwebengineprocess|gameserver|srcds(?:_win64)?)$') { return $true }
    # Boundary-sensitive roles keep titles such as Observer and Crash intact.
    return ($role -match '^(launcher|bootstrap|bootstrapper|updater|helper|crashreporter|dedicatedserver)$' -or
        $role -match '[-_ ](launcher|bootstrap|bootstrapper|updater|helper|crashreporter|dedicatedserver)$' -or
        $role -cmatch '[a-z0-9](Launcher|Bootstrap|Bootstrapper|Updater|Helper|CrashReporter|DedicatedServer)$')
}

function Get-GameRendererCandidates {
    param([string]$GameDirectory,[string]$DeclaredExecutable='', $Cancellation,
        [Collections.Generic.List[string]]$Issues)
    $root=[IO.Path]::GetFullPath($GameDirectory).TrimEnd('\')
    $rootItem=Get-Item -LiteralPath $root -Force -ErrorAction Stop
    if (-not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw '游戏目录必须是普通文件夹。' }
    $pending=[Collections.Generic.Queue[object]]::new()
    $pending.Enqueue(@{Path=$root;Depth=0}); $visited=0; $rows=@()
    $skip='^(?i:windows|\$recycle.bin|system volume information|node_modules|\.git|_commonredist|redist|redistributables|prerequisites|thirdparty|tools?|support|__installer|editors|sdk|dlssg_sm86)$'
    while ($pending.Count -and -not (Test-DiscoveryCancelled $Cancellation)) {
        if (++$visited -gt 12000) { if ($null -ne $Issues) { $Issues.Add("扫描目录达到上限：$root") }; break }
        $entry=$pending.Dequeue()
        try { $items=@(Get-ChildItem -LiteralPath $entry.Path -Force -ErrorAction Stop) }
        catch { if ($null -ne $Issues) { $Issues.Add("无法读取：$($entry.Path)") }; continue }
        $names=@($items | ForEach-Object { $_.Name.ToLowerInvariant() })
        foreach ($item in $items) {
            if (Test-DiscoveryCancelled $Cancellation) { break }
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
            if ($item.PSIsContainer) {
                if ($entry.Depth -lt 12 -and $item.Name -notmatch $skip) { $pending.Enqueue(@{Path=$item.FullName;Depth=$entry.Depth+1}) }
                elseif ($entry.Depth -ge 12 -and $null -ne $Issues) { $Issues.Add("达到扫描深度限制：$($item.FullName)") }
                continue
            }
            if ($item.Extension -ine '.exe' -or (Test-GameAuxiliaryExecutable $item.Name) -or -not (Test-GameX64Executable $item.FullName)) { continue }
            $score=0; $reasons=@()
            if ('nvngx_dlssg.dll' -in $names -or 'sl.dlss_g.dll' -in $names) { $score+=80; $reasons+='同目录有帧生成组件' }
            if ($item.Name -match '(?i)-Win64-Shipping\.exe$' -and $entry.Path -match '(?i)\\Binaries\\Win64$') { $score+=60; $reasons+='UE Shipping 游戏目录' }
            if ('unityplayer.dll' -in $names -and ($item.BaseName.ToLowerInvariant()+'_data') -in $names) { $score+=60; $reasons+='Unity 程序与数据目录对应' }
            if ($DeclaredExecutable -and $item.FullName -ieq $DeclaredExecutable) { $score+=30; $reasons+='平台声明的启动入口（需核实渲染程序）' }
            if ('nvngx_dlss.dll' -in $names) { $score+=10; $reasons+='有超分组件，不代表支持帧生成' }
            if ($item.BaseName -match '(?i)(dx12|d3d12)' -or $entry.Path -match '(?i)\\x64_dx12$') { $score+=5; $reasons+='DX12 命名线索' }
            $relative=$item.FullName.Substring($root.Length+1)
            if (-not $reasons.Count) { $reasons=@('x64 程序；请手动核实游戏本体') }
            $rows += [pscustomobject]@{Path=$item.FullName;Relative=$relative;Score=$score;Reasons=($reasons -join '；');Display=($relative+' — '+($reasons -join '；'))}
        }
    }
    return @($rows | Sort-Object @{Expression='Score';Descending=$true},Relative)
}

function Get-InstalledGameRecords {
    param([string]$SteamExecutable='', [string]$EpicManifestDirectory=(Join-Path $env:ProgramData 'Epic\EpicGamesLauncher\Data\Manifests'),
        [string[]]$GogRegistryRoots=@('HKLM:\SOFTWARE\GOG.com\Games','HKLM:\SOFTWARE\WOW6432Node\GOG.com\Games','HKCU:\SOFTWARE\GOG.com\Games'),
        $Cancellation, [Collections.Generic.List[string]]$Issues)
    $records=@()
    if ($SteamExecutable) {
        foreach ($apps in @(Get-SteamLibraryDirectories $SteamExecutable)) {
            if (Test-DiscoveryCancelled $Cancellation) { break }
            if (-not [IO.Directory]::Exists($apps)) { continue }
            try {
                foreach ($manifest in @(Get-ChildItem -LiteralPath $apps -Filter 'appmanifest_*.acf' -File -ErrorAction Stop)) {
                    if (Test-DiscoveryCancelled $Cancellation) { break }
                    try {
                        $text=[IO.File]::ReadAllText($manifest.FullName)
                        $id=[regex]::Match($text,'(?m)^\s*"appid"\s+"([1-9][0-9]*)"\s*$').Groups[1].Value
                        $install=[regex]::Match($text,'(?m)^\s*"installdir"\s+"([^"\r\n]+)"\s*$').Groups[1].Value
                        if (-not $id -or -not $install -or $manifest.Name -ine "appmanifest_$id.acf" -or [IO.Path]::IsPathRooted($install)) { throw '清单标识无效' }
                        $common=[IO.Path]::GetFullPath((Join-Path $apps 'common')).TrimEnd('\')+'\'
                        $path=[IO.Path]::GetFullPath((Join-Path $common $install))
                        if (-not $path.StartsWith($common,[StringComparison]::OrdinalIgnoreCase)) { throw '目录越界' }
                        $match=Resolve-SteamAppId $path $SteamExecutable
                        if ($match.AppId -ne $id) { throw 'AppID不匹配' }
                        $name=[regex]::Match($text,'(?m)^\s*"name"\s+"([^"\r\n]+)"\s*$').Groups[1].Value
                        $records += [pscustomobject]@{Platform='Steam';Name=$name;GameDirectory=$path;AppId=$id;DeclaredExecutable=''}
                    } catch { if ($null -ne $Issues) { $Issues.Add("Steam清单跳过：$($manifest.FullName)：$($_.Exception.Message)") } }
                }
            } catch { if ($null -ne $Issues) { $Issues.Add("Steam库无法读取：$apps") } }
        }
    }
    if ([IO.Directory]::Exists($EpicManifestDirectory)) {
        foreach ($file in @(Get-ChildItem -LiteralPath $EpicManifestDirectory -Filter '*.item' -File -ErrorAction SilentlyContinue)) {
            if (Test-DiscoveryCancelled $Cancellation) { break }
            try {
                $record=[IO.File]::ReadAllText($file.FullName) | ConvertFrom-Json
                $fields=@{}; foreach ($p in $record.PSObject.Properties) { $fields[$p.Name]=$p.Value }
                if (($fields.ContainsKey('bIsApplication') -and -not $fields.bIsApplication) -or ($fields.ContainsKey('bIsExecutable') -and -not $fields.bIsExecutable)) { continue }
                if ($fields.MainGameAppName -and $fields.MainGameAppName -ne $fields.AppName) { continue }
                if (-not $fields.InstallLocation -or -not [IO.Path]::IsPathRooted($fields.InstallLocation)) { throw '缺少安装路径' }
                $declared=''
                if ($fields.LaunchExecutable) { $declared=Resolve-DirectExecutable $fields.InstallLocation $fields.LaunchExecutable '平台入口' }
                $records += [pscustomobject]@{Platform='Epic';Name=[string]$fields.DisplayName;GameDirectory=[string]$fields.InstallLocation;AppId='';DeclaredExecutable=$declared}
            } catch { if ($null -ne $Issues) { $Issues.Add("Epic清单跳过：$($file.Name)：$($_.Exception.Message)") } }
        }
    }
    foreach ($registryRoot in $GogRegistryRoots) {
        if (Test-DiscoveryCancelled $Cancellation) { break }
        if (-not (Test-Path -LiteralPath $registryRoot)) { continue }
        foreach ($key in @(Get-ChildItem -LiteralPath $registryRoot -ErrorAction SilentlyContinue)) {
            try {
                $properties=Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction Stop
                $fields=@{}; foreach ($p in $properties.PSObject.Properties) { $fields[$p.Name]=$p.Value }
                if (-not $fields.path -or -not [IO.Path]::IsPathRooted($fields.path)) { continue }
                $declared=''; $name=[string]$fields.gameName
                foreach ($info in @(Get-ChildItem -LiteralPath $fields.path -Filter 'goggame-*.info' -File -ErrorAction Stop)) {
                    $gog=[IO.File]::ReadAllText($info.FullName) | ConvertFrom-Json
                    $g=@{}; foreach ($p in $gog.PSObject.Properties) { $g[$p.Name]=$p.Value }
                    if ($g.rootGameId -and [string]$g.rootGameId -ne [string]$g.gameId) { continue }
                    if ($g.name) { $name=[string]$g.name }
                    foreach ($task in @($g.playTasks)) {
                        if ($null -eq $task) { continue }
                        $t=@{}; foreach ($p in $task.PSObject.Properties) { $t[$p.Name]=$p.Value }
                        if ($t.isPrimary -and $t.type -eq 'FileTask' -and $t.path -like '*.exe') { $declared=Resolve-DirectExecutable $fields.path $t.path 'GOG游戏入口'; break }
                    }
                    if ($declared) { break }
                }
                $records += [pscustomobject]@{Platform='GOG';Name=$name;GameDirectory=[string]$fields.path;AppId='';DeclaredExecutable=$declared}
            } catch { if ($null -ne $Issues) { $Issues.Add("GOG记录跳过：$($key.Name)：$($_.Exception.Message)") } }
        }
    }
    $seen=@{}
    foreach ($record in $records) {
        if (Test-DiscoveryCancelled $Cancellation) { break }
        try {
            $path=[IO.Path]::GetFullPath($record.GameDirectory).TrimEnd('\')
            $item=Get-Item -LiteralPath $path -Force -ErrorAction Stop
            if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { continue }
            if ($seen.ContainsKey($path)) { continue }; $seen[$path]=$true
            $record.GameDirectory=$path
            if (-not $record.Name) { $record.Name=Split-Path -Leaf $path }
            $record | Add-Member NoteProperty Display ("$($record.Name) [$($record.Platform)] — $path")
            $record
        } catch { if ($null -ne $Issues) { $Issues.Add("安装位置不可用：$($record.GameDirectory)") } }
    }
}
