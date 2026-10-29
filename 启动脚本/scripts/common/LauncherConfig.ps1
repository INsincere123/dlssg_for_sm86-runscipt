# Shared, data-only configuration reader. Windows PowerShell 5.1+.
function Get-LauncherRoot {
    Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
}

function Read-LauncherConfigDocument {
    param([string]$Path)
    try {
        $bytes = [IO.File]::ReadAllBytes($Path)
        $offset = 0
        # Test UTF-32 before UTF-16 because their BOMs overlap.
        if ($bytes.Length -ge 4 -and $bytes[0] -eq 255 -and $bytes[1] -eq 254 -and $bytes[2] -eq 0 -and $bytes[3] -eq 0) {
            $encoding = [Text.UTF32Encoding]::new($false,$true,$true); $offset = 4
        }
        elseif ($bytes.Length -ge 4 -and $bytes[0] -eq 0 -and $bytes[1] -eq 0 -and $bytes[2] -eq 254 -and $bytes[3] -eq 255) {
            $encoding = [Text.UTF32Encoding]::new($true,$true,$true); $offset = 4
        }
        elseif ($bytes.Length -ge 3 -and $bytes[0] -eq 239 -and $bytes[1] -eq 187 -and $bytes[2] -eq 191) {
            $encoding = [Text.UTF8Encoding]::new($false,$true); $offset = 3
        }
        elseif ($bytes.Length -ge 2 -and $bytes[0] -eq 255 -and $bytes[1] -eq 254) {
            $encoding = [Text.UnicodeEncoding]::new($false,$true,$true); $offset = 2
        }
        elseif ($bytes.Length -ge 2 -and $bytes[0] -eq 254 -and $bytes[1] -eq 255) {
            $encoding = [Text.UnicodeEncoding]::new($true,$true,$true); $offset = 2
        }
        else {
            try {
                $encoding = [Text.UTF8Encoding]::new($false,$true)
                $text = $encoding.GetString($bytes)
            }
            catch [Text.DecoderFallbackException] {
                $encoding = [Text.Encoding]::GetEncoding(936,[Text.EncoderFallback]::ExceptionFallback,[Text.DecoderFallback]::ExceptionFallback)
                $text = $encoding.GetString($bytes)
                Write-Warning "启动配置 '$Path' 不是有效 UTF-8，已按 GBK/CP936 尝试读取；请确认中文正确，建议另存为 UTF-8。"
            }
        }
        $text = $encoding.GetString($bytes,$offset,$bytes.Length-$offset)
        $preamble = if ($offset) { [byte[]]$bytes[0..($offset-1)] } else { [byte[]]@() }
        return @{ Text=$text; Encoding=$encoding; Preamble=$preamble; Bytes=$bytes }
    }
    catch { throw "无法读取启动配置 '$Path'，请检查路径或另存为 UTF-8：$($_.Exception.Message)" }
}

function Read-LauncherConfigText {
    param([string]$Path)
    return (Read-LauncherConfigDocument -Path $Path).Text
}

function Read-LauncherConfigSection {
    param([string]$Name, [string]$Path = (Join-Path (Get-LauncherRoot) '启动配置.ini'))
    $text = Read-LauncherConfigText -Path $Path
    if ($text.Contains([string][char]0) -or $text.Contains([string][char]0xFFFD)) {
        throw "启动配置 '$Path' 含无效文本，请另存为 UTF-8。"
    }
    $groups = @{ '鸣潮'='Wuthering'; '异环'='Nte'; '显卡'='Gpu'; Wuthering='Wuthering'; Nte='Nte'; Gpu='Gpu' }
    $keys = @{
        '游戏目录'='GameDirectory'; '启动器路径'='LauncherPath'; '配置源'='EngineConfigSource'
        '游戏配置目录'='ConfigDirectory'; '重启等待秒数'='RestartGraceSeconds'
        '设备实例标识'='DeviceInstanceId'; '目标显示名称'='TargetName'
        '启动程序'='LaunchExecutable'; '游戏名称'='GameName'; '渲染程序'='RenderExecutable'; '代理DLL'='ProxyDll'
    }
    $allowed = if ($Name -eq 'Gpu') { @('DeviceInstanceId','TargetName') }
        elseif ($Name -eq 'Steam') { @('LauncherPath') }
        elseif ($Name -match '^SteamUE5(?::[^\[\]]+)?$') { @('GameName','GameDirectory','RenderExecutable','ConfigDirectory','EngineConfigSource','ProxyDll') }
        elseif ($Name -match '^DirectUE:[^\[\]]+$') { @('GameName','GameDirectory','RenderExecutable','LaunchExecutable','ConfigDirectory','EngineConfigSource','ProxyDll') }
        elseif ($Name -eq 'Wuthering') { @('GameDirectory','LauncherPath','EngineConfigSource','ConfigDirectory','RestartGraceSeconds') }
        else { @('GameDirectory','LauncherPath','EngineConfigSource','ConfigDirectory') }
    $section = @{}; $current = ''; $found = $false; $lineNumber = 0
    foreach ($line in ($text -split '\r\n|\n|\r')) {
        $lineNumber++
        $trimmed = $line.Trim()
        if (-not $trimmed -or $trimmed.StartsWith(';') -or $trimmed.StartsWith('#')) { continue }
        if ($trimmed -match '^\[([^\[\]]+)\]$') {
            $label = $Matches[1].Trim()
            $current = if ($groups.ContainsKey($label)) { $groups[$label] } else { $label }
            if ($current -eq $Name) {
                if ($found) { throw "启动配置第 $lineNumber 行：分组 $Name 重复。" }
                $found = $true
            }
            continue
        }
        $separator = $trimmed.IndexOf('=')
        if (-not $current -or $separator -le 0) { throw "启动配置第 $lineNumber 行格式错误，应为 [分组] 或 键=值。" }
        if ($current -ne $Name) { continue }
        $key = $trimmed.Substring(0,$separator).Trim()
        if ($keys.ContainsKey($key)) { $key = $keys[$key] }
        if ($key -notin $allowed) { throw "启动配置第 $lineNumber 行：未知配置项 $Name.$key。" }
        if ($section.ContainsKey($key)) { throw "启动配置第 $lineNumber 行：配置项 $Name.$key 重复。" }
        # Do not split inline comments: #, ; and = are valid filename characters.
        $section[$key] = $trimmed.Substring($separator+1).Trim()
    }
    if (-not $found) { throw "启动配置缺少有效分组：$Name" }
    return $section
}

function Remove-LauncherValueQuotes {
    param([string]$Value, [string]$Name)
    $value = $Value.Trim()
    $quotes = @{ '"'='"'; "'"="'"; ([string][char]0x201C)=([string][char]0x201D); ([string][char]0x2018)=([string][char]0x2019) }
    if ($value.Length -gt 0 -and $quotes.ContainsKey($value.Substring(0,1))) {
        if ($value.Length -lt 2 -or $value.Substring($value.Length-1,1) -ne $quotes[$value.Substring(0,1)]) {
            throw "启动配置 $Name 的外层引号不成对。"
        }
        return $value.Substring(1,$value.Length-2)
    }
    return $value
}

function Get-LauncherText {
    param([System.Collections.IDictionary]$Section, [string]$Key, [string]$Group, [switch]$Optional)
    $value = $Section[$Key]
    if ($null -ne $value -and $value -isnot [string]) { throw "启动配置 $Group.$Key 必须是字符串。" }
    if ($null -ne $value) { $value = Remove-LauncherValueQuotes -Value $value -Name "$Group.$Key" }
    if ([string]::IsNullOrWhiteSpace($value)) {
        if ($Optional) { return '' }
        throw "启动配置 $Group.$Key 不能为空。"
    }
    return $value
}

function Resolve-LauncherConfigPath {
    param([string]$Value, [string]$Name, [string]$Root = (Get-LauncherRoot))
    if ([string]::IsNullOrWhiteSpace($Value)) { return '' }
    $expanded = Remove-LauncherValueQuotes -Value $Value -Name $Name
    $fromUri = $false
    if ($expanded -match '^[A-Za-z][A-Za-z0-9+.-]*://' -or $expanded -match '^file:') {
        $uri = $null
        if (-not [Uri]::TryCreate($expanded,[UriKind]::Absolute,[ref]$uri) -or -not $uri.IsFile -or $uri.Query -or $uri.Fragment) {
            throw "启动配置 $Name 仅支持文件路径或 file URI，不支持此网址：$Value"
        }
        $expanded = $uri.LocalPath
        $fromUri = $true
    }
    # Expand only environment references, never execute PowerShell expressions.
    if (-not $fromUri) { $expanded = [regex]::Replace($expanded, '%([^%]+)%|\$\{env:([^}]+)\}|\$env:([A-Za-z_][A-Za-z0-9_]*)', {
        param($match)
        $variable = ($match.Groups | Select-Object -Skip 1 | Where-Object Success | Select-Object -First 1).Value
        $replacement = [Environment]::GetEnvironmentVariable($variable)
        if ($null -eq $replacement) { throw "启动配置 $Name 包含未定义的环境变量：$variable" }
        return $replacement
    }, [Text.RegularExpressions.RegexOptions]::IgnoreCase) }
    if ($expanded -eq '~' -or $expanded -match '^~[\\/]') {
        $expanded = [Environment]::GetFolderPath('UserProfile') + $expanded.Substring(1)
    }
    $expanded = $expanded -replace '^(?:Microsoft\.PowerShell\.Core\\)?FileSystem::',''
    $expanded = $expanded.Replace('/','\')
    # Use one spelling for extended and ordinary paths so journal identity and locks agree.
    if ($expanded.StartsWith('\\?\UNC\',[StringComparison]::OrdinalIgnoreCase)) {
        $expanded = '\\' + $expanded.Substring(8)
    }
    elseif ($expanded -match '^\\\\\?\\[A-Za-z]:\\') { $expanded = $expanded.Substring(4) }
    elseif ($expanded.StartsWith('\\?\') -or $expanded.StartsWith('\\.\')) {
        throw "启动配置 $Name 只支持普通文件路径、盘符或 UNC 扩展路径：$Value"
    }
    if ($expanded -match '^[A-Za-z]:[^\\]' -or $expanded -match '^[A-Za-z]:$') {
        throw "启动配置 $Name 不支持含义不明确的盘符相对路径，请写 C:\目录 或相对路径：$Value"
    }
    if ($expanded -match '[\x00-\x1F<>"|?*]') {
        throw "启动配置 $Name 含无效路径字符：$Value"
    }
    try {
        if ($expanded -match '^\\(?!\\)') { $expanded = [IO.Path]::GetPathRoot($Root).TrimEnd('\') + $expanded }
        elseif (-not [IO.Path]::IsPathRooted($expanded)) { $expanded = Join-Path $Root $expanded }
        return [IO.Path]::GetFullPath($expanded)
    }
    catch { throw "启动配置 $Name 路径无效：$Value；$($_.Exception.Message)" }
}

function Save-LauncherDetectedPath {
    param([string]$Path, [string]$GameId, [Alias('Value')][string]$LauncherPath,
        [ValidateSet('LauncherPath','ConfigDirectory')][string]$Key = 'LauncherPath')
    $displayKey = if ($Key -eq 'LauncherPath') { '启动器路径' } else { '游戏配置目录' }
    $keyPattern = '^(\s*(?:' + [regex]::Escape($displayKey) + '|' + $Key + ')\s*=)[^\r\n]*'
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $id = [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes([IO.Path]::GetFullPath($Path).ToLowerInvariant()))).Replace('-','') }
    finally { $sha.Dispose() }
    $mutex = [Threading.Mutex]::new($false, "Local\DLSSG-LauncherConfig-$id")
    $locked = $false; $temporary = $null
    try {
        try { $locked = $mutex.WaitOne(5000) } catch [Threading.AbandonedMutexException] { $locked = $true }
        if (-not $locked) { throw '另一个进程正在更新配置，请稍后重试。' }
        $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '配置是链接，请手动填写对应路径。' }
        if ($item.Attributes -band [IO.FileAttributes]::ReadOnly) { throw '配置文件为只读，请取消只读后重试或手动填写。' }
        $document = Read-LauncherConfigDocument -Path $Path
        $section = Read-LauncherConfigSection -Name $GameId -Path $Path
        $existing = Get-LauncherText -Section $section -Key $Key -Group $GameId -Optional
        if ($existing) { return (Resolve-LauncherConfigPath -Value $existing -Name "$GameId.$Key") }
        $label = if ($GameId -eq 'Wuthering') { '鸣潮' } else { '异环' }
        $lines = [regex]::Matches($document.Text, '[^\r\n]*(?:\r\n|\n|\r|$)')
        $inside = $false; $keyLine = $null; $sectionLine = $null
        foreach ($line in $lines) {
            if ($line.Value.Trim() -match '^\[([^\[\]]+)\]$') {
                $inside = $Matches[1].Trim() -in @($label,$GameId)
                if ($inside) { $sectionLine = $line }
            }
            elseif ($inside -and $line.Value -match $keyPattern) { $keyLine = $line }
        }
        if ($null -eq $sectionLine) { throw "找不到配置分组：$GameId" }
        if ($null -ne $keyLine) {
            $replacement = [regex]::Replace($keyLine.Value, $keyPattern, { param($match) $match.Groups[1].Value + $LauncherPath })
            $updated = $document.Text.Remove($keyLine.Index,$keyLine.Length).Insert($keyLine.Index,$replacement)
        }
        else {
            $newline = [regex]::Match($document.Text,'\r\n|\n|\r').Value
            if (-not $newline) { $newline = "`r`n" }
            $at = $sectionLine.Index + $sectionLine.Length
            $prefix = if ($sectionLine.Value -match '[\r\n]$') { '' } else { $newline }
            $updated = $document.Text.Insert($at,$prefix + "$displayKey=$LauncherPath" + $newline)
        }
        $bytes = [byte[]]($document.Preamble + $document.Encoding.GetBytes($updated))
        $temporary = Join-Path $item.DirectoryName ([guid]::NewGuid().ToString('N') + '.tmp')
        [IO.File]::WriteAllBytes($temporary,$bytes)
        if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($Path)) -cne [Convert]::ToBase64String($document.Bytes)) {
            throw '配置在更新过程中被修改，已停止写回，请重试。'
        }
        [IO.File]::Replace($temporary,$item.FullName,[System.Management.Automation.Language.NullString]::Value)
        Write-Host "已记录${displayKey}到启动配置.ini：$LauncherPath"
        return $LauncherPath
    }
    catch { throw "无法记录启动配置 $GameId.$Key：$($_.Exception.Message)" }
    finally {
        if ($temporary -and [IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
        if ($locked) { $mutex.ReleaseMutex() }
        $mutex.Dispose()
    }
}

function Save-LauncherDetectedConfigDirectory {
    param([string]$GameId, [string]$Directory, [System.Collections.IDictionary]$Overrides = @{},
        [string]$Path = (Join-Path (Get-LauncherRoot) '启动配置.ini'))
    foreach ($flag in @('DryRun','RestoreDeployment','RestoreEngineConfig')) {
        if ($Overrides.ContainsKey($flag) -and $Overrides[$flag]) { return $Directory }
    }
    # A temporary command-line override must never become the persistent default.
    if ($Overrides.ContainsKey('ConfigDirectory')) { return $Directory }
    $section = Read-LauncherConfigSection -Name $GameId -Path $Path
    if (Get-LauncherText -Section $section -Key ConfigDirectory -Group $GameId -Optional) { return $Directory }
    if (-not [IO.Directory]::Exists($Directory)) { throw "游戏配置目录不存在，未写回启动配置：$Directory" }
    $saved = Save-LauncherDetectedPath -Path $Path -GameId $GameId -Value $Directory -Key ConfigDirectory
    if ([IO.Path]::GetFullPath($saved).TrimEnd('\') -ine [IO.Path]::GetFullPath($Directory).TrimEnd('\')) {
        throw '游戏配置目录在查找期间被修改，请重新运行以核验新目录。'
    }
    return $saved
}

function Find-LauncherExecutable {
    param([ValidateSet('Wuthering','Nte')][string]$GameId, [string]$GameDirectory,
        [string[]]$DefaultDirectories = @(), [switch]$RestoreOnly)
    $name = if ($GameId -eq 'Wuthering') { 'launcher.exe' } else { 'NTELauncher.exe' }
    $directories = @($GameDirectory, (Split-Path -Parent $GameDirectory)) + $DefaultDirectories
    $candidates = @($directories | Where-Object { $_ } | ForEach-Object {
        $candidate = Join-Path $_ $name
        if ([IO.File]::Exists($candidate)) { [IO.Path]::GetFullPath($candidate) }
    } | Sort-Object -Unique)
    if ($candidates.Count -eq 1) { return $candidates[0] }
    # Recovery needs the known launcher process name, but never needs to launch its EXE.
    if ($RestoreOnly) { return '' }
    if ($candidates.Count -eq 0) {
        throw "启动配置 $GameId.LauncherPath：未找到启动器，请在启动配置.ini 填写启动器路径（完整 EXE 路径）。已检查：$($directories -join '；')"
    }
    throw "启动配置 $GameId.LauncherPath：找到多个启动器，请在启动配置.ini 明确填写启动器路径：$($candidates -join '；')"
}

function Get-LauncherGameSettings {
    [CmdletBinding()]
    param([ValidateSet('Wuthering','Nte')][string]$GameId,
        [System.Collections.IDictionary]$Overrides = @{},
        [string]$Path = (Join-Path (Get-LauncherRoot) '启动配置.ini'))
    $section = (Read-LauncherConfigSection -Name $GameId -Path $Path).Clone()
    foreach ($key in @('ConfigDirectory','EngineConfigSource','RestartGraceSeconds')) {
        if ($Overrides.ContainsKey($key)) { $section[$key] = $Overrides[$key] }
    }
    $settings = @{}
    foreach ($key in @('GameDirectory','LauncherPath','EngineConfigSource','ConfigDirectory')) {
        $value = Get-LauncherText -Section $section -Key $key -Group $GameId -Optional:($key -in @('ConfigDirectory','LauncherPath'))
        $settings[$key] = Resolve-LauncherConfigPath -Value $value -Name "$GameId.$key"
    }
    $restoreOnly = $false
    if (-not $settings.LauncherPath) {
        $folder = if ($GameId -eq 'Wuthering') { 'Wuthering Waves' } else { 'Neverness To Everness' }
        $defaults = @(@($env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:ProgramW6432) |
            Where-Object { $_ } | ForEach-Object { Join-Path $_ $folder } | Sort-Object -Unique)
        $restoreOnly = ($Overrides.ContainsKey('RestoreDeployment') -and $Overrides['RestoreDeployment']) -or
            ($Overrides.ContainsKey('RestoreEngineConfig') -and $Overrides['RestoreEngineConfig'])
        $settings.LauncherPath = Find-LauncherExecutable -GameId $GameId -GameDirectory $settings.GameDirectory -DefaultDirectories $defaults -RestoreOnly:$restoreOnly
    }
    if ($GameId -eq 'Nte' -and -not $settings.ConfigDirectory) {
        $settings.ConfigDirectory = Resolve-LauncherConfigPath -Value '%LOCALAPPDATA%\HT\Saved\Config\Windows' -Name 'Nte.ConfigDirectory'
    }
    if ($GameId -eq 'Wuthering') {
        $seconds = 0
        $secondsText = Remove-LauncherValueQuotes -Value ([string]$section['RestartGraceSeconds']) -Name 'Wuthering.RestartGraceSeconds'
        if (-not [int]::TryParse($secondsText,[ref]$seconds) -or $seconds -lt 1 -or $seconds -gt 3600) {
            throw '启动配置 Wuthering.RestartGraceSeconds 必须是 1–3600 的整数。'
        }
        $settings.RestartGraceSeconds = $seconds
    }
    if (-not (Get-LauncherText -Section $section -Key 'LauncherPath' -Group $GameId -Optional) -and
        -not $restoreOnly -and -not ($Overrides.ContainsKey('DryRun') -and $Overrides['DryRun'])) {
        $settings.LauncherPath = Save-LauncherDetectedPath -Path $Path -GameId $GameId -LauncherPath $settings.LauncherPath
    }
    return $settings
}

function Select-LauncherNvidiaDevice {
    param([object[]]$Devices, [string]$DeviceInstanceId)
    $candidates = @($Devices | Where-Object { $_.Class -eq 'Display' -and $_.InstanceId -match '^PCI\\VEN_10DE&' })
    if ($DeviceInstanceId) {
        $selected = @($candidates | Where-Object { $_.InstanceId -ieq $DeviceInstanceId })
    }
    else { $selected = $candidates }
    if ($selected.Count -ne 1) {
        $listing = ($candidates | ForEach-Object { "$($_.FriendlyName) : $($_.InstanceId)" }) -join "`r`n"
        if (-not $listing) { $listing = '无当前存在的 NVIDIA 显卡。' }
        throw "启动配置 Gpu.DeviceInstanceId 无法唯一确定 NVIDIA 显卡，请填写有效设备实例标识。`r`n$listing"
    }
    return [string]$selected[0].InstanceId
}

function Get-LauncherGpuSettings {
    [CmdletBinding()]
    param([string]$Path = (Join-Path (Get-LauncherRoot) '启动配置.ini'))
    $section = Read-LauncherConfigSection -Name 'Gpu' -Path $Path
    $target = Get-LauncherText -Section $section -Key 'TargetName' -Group 'Gpu'
    $instance = Get-LauncherText -Section $section -Key 'DeviceInstanceId' -Group 'Gpu' -Optional
    $devices = @(Get-PnpDevice -Class Display -PresentOnly -ErrorAction Stop)
    return @{
        DeviceInstanceId = Select-LauncherNvidiaDevice -Devices $devices -DeviceInstanceId $instance
        TargetName = $target
    }
}
