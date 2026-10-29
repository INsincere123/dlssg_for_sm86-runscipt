# Shared 0.4.0 resource deployment. Never traverse a resource link during cleanup.
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
