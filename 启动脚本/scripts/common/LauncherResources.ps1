# Shared 0.4.0 resource deployment. Never traverse a resource link during cleanup.
function New-DlssgLogCursor {
    param([string]$LogDirectory,[string]$Executable)
    $path=Join-Path $LogDirectory 'dlssg_sm86.log'
    $offset=0L; $created=$null
    if ([IO.File]::Exists($path)) {
        try {
            $item=Get-Item -LiteralPath $path -Force -ErrorAction Stop
            $offset=$item.Length; $created=$item.CreationTimeUtc
        } catch { Write-Warning '无法读取日志起始位置，后续仅按时间与EXE筛选，不阻止部署恢复。' }
    }
    return [pscustomobject]@{Path=$path;Directory=$LogDirectory;Offset=$offset;Created=$created;
        StartedAt=[DateTime]::Now;Executable=[IO.Path]::GetFileName($Executable)}
}

function Get-DlssgSessionLog {
    param($Cursor)
    $result=[pscustomobject]@{Path=$Cursor.Path;Exists=$false;Pids=@();Records=0;LegacyPaths=@()}
    if (-not [IO.File]::Exists($Cursor.Path)) {
        # Historical files are hints only, never evidence of this session.
        if ([IO.Directory]::Exists($Cursor.Directory)) {
            $result.LegacyPaths=@(Get-ChildItem -LiteralPath $Cursor.Directory -File -ErrorAction Stop |
                Where-Object { $_.Name -match '^(loader|backend)_\d+\.jsonl$' } | ForEach-Object { $_.FullName })
        }
        return $result
    }
    $result.Exists=$true; $stream=$null; $reader=$null; $ids=@{}
    try {
        $stream=[IO.File]::Open($Cursor.Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
        $item=Get-Item -LiteralPath $Cursor.Path -Force -ErrorAction Stop
        $offset=if ($Cursor.Created -eq $item.CreationTimeUtc -and $stream.Length -ge $Cursor.Offset) { $Cursor.Offset } else { 0L }
        [void]$stream.Seek($offset,[IO.SeekOrigin]::Begin)
        $reader=[IO.StreamReader]::new($stream,[Text.Encoding]::UTF8,$true)
        $pattern='^(?<time>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3})\s+pid\s+(?<pid>\d+)\s+'+[regex]::Escape($Cursor.Executable)+'\s+[EID]\s+\{'
        while ($null -ne ($line=$reader.ReadLine())) {
            if ($line -notmatch $pattern) { continue }
            $stamp=[DateTime]::MinValue
            if (-not [DateTime]::TryParseExact($Matches.time,'yyyy-MM-dd HH:mm:ss.fff',[Globalization.CultureInfo]::InvariantCulture,[Globalization.DateTimeStyles]::None,[ref]$stamp)) { continue }
            if ($stamp -lt $Cursor.StartedAt.AddMilliseconds(-1)) { continue }
            $ids[$Matches.pid]=$true; $result.Records++
        }
    } finally { if ($null -ne $reader) { $reader.Dispose() } elseif ($null -ne $stream) { $stream.Dispose() } }
    $result.Pids=@($ids.Keys | Sort-Object)
    return $result
}

function Show-DlssgSessionLogStatus {
    [CmdletBinding()]
    param($Cursor)
    try {
        $info=Get-DlssgSessionLog $Cursor
        if ($info.Exists) {
            Write-Host ("组件日志：{0}" -f $info.Path)
            if ($info.Records) { Write-Host ("本次 {0} 的日志记录：{1}；PID：{2}" -f $Cursor.Executable,$info.Records,($info.Pids -join ', ')) }
            else { Write-Warning '统一日志中未找到本次游戏的新记录，不能据此确认组件加载或生效。' }
        } else {
            Write-Warning ("未找到0.4.2统一日志：{0}" -f $info.Path)
            foreach ($path in $info.LegacyPaths) { Write-Host ("历史格式日志（未认定属于本次）：{0}" -f $path) }
        }
        Write-Host '日志定位不代表帧生成成功；需检查本次配置、加载、错误与执行记录。超分Mode=Off时不要求INT8执行。'
    } catch { Write-Warning ("组件日志读取失败，不影响部署恢复结果：{0}" -f $_.Exception.Message) }
}

function Select-DlssgProxy {
    param([string]$ProjectDirectory, [string]$RenderDirectory,
        [ValidateSet('version.dll','winmm.dll','dbghelp.dll','dinput8.dll','dxgi.dll','d3d12.dll')][string]$ProxyName)
    $relative = if ($ProxyName -eq 'version.dll') { $ProxyName } else { "alternatives\$ProxyName" }
    $script:projectProxy = Join-Path $ProjectDirectory $relative
    $script:storageDll = Join-Path $RenderDirectory "_storage_\$ProxyName"
    $script:deployment = New-DlssgDeployment -ProjectDirectory $ProjectDirectory -RenderDirectory $RenderDirectory -ProxyName $ProxyName
    $script:linkSpecs[0] = @{ Path = (Join-Path $RenderDirectory $ProxyName); Target = $script:projectProxy }
}

function New-DlssgDeployment {
    param([string]$ProjectDirectory, [string]$RenderDirectory, [string]$ProxyName)
    $root = Join-Path $RenderDirectory 'dlssg_sm86'
    $specs = @(foreach ($name in @('runtime', 'backend', 'kernels', 'xefg', 'sr')) {
        @{ Path = (Join-Path $root $name); Target = (Join-Path $ProjectDirectory "dlssg_sm86\$name") }
    })
    return @{ Root = $root; Specs = $specs; Created = [Collections.Generic.List[object]]::new();
        Project = $ProjectDirectory; Render = $RenderDirectory; Proxy = $ProxyName }
}

function Assert-DlssgDeployment {
    param($Deployment)
    foreach ($relative in @('dlssg_sm86.ini', 'dlssg_sm86\runtime\nvngx_dlssg.dll',
        'dlssg_sm86\backend\sm86_backend.dll', 'dlssg_sm86\kernels\fg_kernels.dll',
        'dlssg_sm86\xefg\xefg_kernels.dll', 'dlssg_sm86\sr\sr_kernels.dll',
        'dlssg_sm86\sr\nvngx_dlss.dll')) {
        $path = Join-Path $Deployment.Project $relative
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing DLSSG resource: $path" }
    }
    $conflicts = @(foreach ($name in @('version.dll', 'winmm.dll', 'dbghelp.dll', 'dinput8.dll', 'dxgi.dll', 'd3d12.dll')) {
        $path = Join-Path $Deployment.Render $name
        if ($name -ne $Deployment.Proxy -and $null -ne (Get-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue)) { $path }
    })
    if ($conflicts.Count) { throw "Conflicting proxy files (preserved): $($conflicts -join '; ')" }
    $rootItem = Get-Item -LiteralPath $Deployment.Root -Force -ErrorAction SilentlyContinue
    if ($null -ne $rootItem -and (-not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint))) {
        throw "Resource root must be an ordinary directory: $($Deployment.Root)"
    }
    foreach ($spec in $Deployment.Specs) {
        $item = Get-Item -LiteralPath $spec.Path -Force -ErrorAction SilentlyContinue
        if ($null -ne $item -and ($item.LinkType -ne 'SymbolicLink' -or
            [IO.Path]::GetFullPath([string]($item.Target -join '')).TrimEnd('\') -ine [IO.Path]::GetFullPath($spec.Target).TrimEnd('\'))) {
            throw "Resource path conflict (preserved): $($spec.Path)"
        }
    }
}

function Install-DlssgResources {
    param($Deployment, [switch]$DryRun)
    Assert-DlssgDeployment $Deployment
    foreach ($spec in $Deployment.Specs) {
        if ($null -ne (Get-Item -LiteralPath $spec.Path -Force -ErrorAction SilentlyContinue)) {
            if (-not $DryRun) { Register-ReusedDeploymentLink -Path $spec.Path -Target $spec.Target }
            continue
        }
        if ($DryRun) { Write-Host "DryRun: link $($spec.Path) -> $($spec.Target)"; continue }
        if (-not (Test-Path -LiteralPath $Deployment.Root)) {
            $record = Start-DeploymentOperation -Kind directory -Path $Deployment.Root
            New-Item -ItemType Directory -Path $Deployment.Root | Out-Null
            Complete-DeploymentOperation $record
        }
        $record = Start-DeploymentOperation -Kind link -Path $spec.Path -Target $spec.Target
        New-Item -ItemType SymbolicLink -Path $spec.Path -Target $spec.Target | Out-Null
        Complete-DeploymentOperation $record
        $Deployment.Created.Add($spec)
        Write-Host "Resource link: $($spec.Path) -> $($spec.Target)"
    }
}

function Undo-DlssgResources {
    param($Deployment)
    $rootItem = Get-Item -LiteralPath $Deployment.Root -Force -ErrorAction SilentlyContinue
    if ($null -eq $rootItem) { return }
    if (-not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        Write-Warning "Resource root changed; cleanup skipped: $($Deployment.Root)"
        return
    }
    foreach ($spec in $Deployment.Created) {
        $item = Get-Item -LiteralPath $spec.Path -Force -ErrorAction SilentlyContinue
        if ($null -eq $item) { continue }
        if ($item.LinkType -eq 'SymbolicLink' -and
            [IO.Path]::GetFullPath([string]($item.Target -join '')).TrimEnd('\') -ieq [IO.Path]::GetFullPath($spec.Target).TrimEnd('\')) {
            # Directory.Delete without recursion removes only the directory symlink, including on PS 5.1.
            try { [IO.Directory]::Delete($spec.Path) }
            catch { Write-Warning "Could not remove resource link $($spec.Path): $_" }
        }
        else { Write-Warning "Resource link changed; preserved: $($spec.Path)" }
    }
}
