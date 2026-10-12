[CmdletBinding()]
param(
    [switch]$Status
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'ScriptLogging.ps1')
$launcherLogPath = Start-LauncherLog -ScriptPath $PSCommandPath
try {
$deviceKeyPath = 'HKLM:\SYSTEM\CurrentControlSet\Enum\PCI\VEN_10DE&DEV_2420&SUBSYS_13241462&REV_A1\4&3062272c&0&0008'
$classKeyRoot = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
$targetName = 'NVIDIA GeForce RTX 5070'
$stateDirectory = Join-Path $env:ProgramData 'Toggle-NvidiaDeviceName'
$statePath = Join-Path $stateDirectory 'state.json'

function Write-Log {
    param([string]$Message)
    Write-Host ("[{0}] {1}" -f (Get-Date -Format 'HH:mm:ss'), $Message)
}

function Show-Message {
    param(
        [string]$Message,
        [string]$Title = 'NVIDIA 设备名称切换'
    )

    Add-Type -AssemblyName System.Windows.Forms -ErrorAction Stop
    [void][System.Windows.Forms.MessageBox]::Show(
        $Message,
        $Title,
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Information
    )
}

function Show-CurrentStateDialog {
    $properties = Get-DeviceProperties
    $description = Split-DeviceDescription -Value ([string]$properties.DeviceDesc)
    $enabled = [string]::Equals($description.DisplayName, $targetName, [System.StringComparison]::OrdinalIgnoreCase)
    $backup = if (Test-Path -LiteralPath $statePath -PathType Leaf) { '存在' } else { '不存在' }
    $stateText = if ($enabled) { '状态：已伪装为 RTX 5070' } else { '状态：原始适配器名称' }

    Show-Message @"
$stateText

当前名称：$($description.DisplayName)
状态备份：$backup

本脚本不会重启电脑，请手动重启。
"@ 'NVIDIA 设备名称状态'
}

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        return
    }

    $arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    if ($Status) { $arguments += ' -Status' }

    Write-Log 'Requesting administrator permission...'
    $elevated = Start-Process -FilePath 'powershell.exe' -Verb RunAs -ArgumentList $arguments -PassThru -Wait
    exit $elevated.ExitCode
}

function Get-DeviceProperties {
    if (-not (Test-Path -LiteralPath $deviceKeyPath)) {
        throw "Target device registry key was not found: $deviceKeyPath"
    }

    return Get-ItemProperty -LiteralPath $deviceKeyPath -ErrorAction Stop
}

function Split-DeviceDescription {
    param([string]$Value)

    $separator = $Value.LastIndexOf(';')
    if ($separator -lt 0 -or $separator -eq ($Value.Length - 1)) {
        throw "DeviceDesc has an unexpected format and will not be overwritten: $Value"
    }

    [pscustomobject]@{
        Prefix = $Value.Substring(0, $separator + 1)
        DisplayName = $Value.Substring($separator + 1)
    }
}

function Set-DeviceDescription {
    param([string]$Value)
    Set-ItemProperty -LiteralPath $deviceKeyPath -Name 'DeviceDesc' -Value $Value -ErrorAction Stop
}

function Get-FallbackOriginalDescription {
    $deviceProperties = Get-DeviceProperties
    $driverReference = [string]$deviceProperties.Driver
    if ($driverReference -notmatch '^(\{[0-9a-fA-F-]+\})\\(\d{4})$') {
        throw "The device Driver reference has an unexpected format: $driverReference"
    }

    $classKeyPath = Join-Path $classKeyRoot $Matches[2]
    $classProperties = Get-ItemProperty -LiteralPath $classKeyPath -Name 'DriverDesc' -ErrorAction Stop
    if ([string]::IsNullOrWhiteSpace([string]$classProperties.DriverDesc)) {
        throw "The driver class entry does not contain a usable DriverDesc: $classKeyPath"
    }

    return [string]$classProperties.DriverDesc
}

function Show-Status {
    $properties = Get-DeviceProperties
    $description = Split-DeviceDescription -Value ([string]$properties.DeviceDesc)

    Write-Log "Target: $deviceKeyPath"
    Write-Log "Current DeviceDesc: $($properties.DeviceDesc)"
    Write-Log "Current display name: $($description.DisplayName)"
    Write-Log "Spoof enabled: $([string]::Equals($description.DisplayName, $targetName, [System.StringComparison]::OrdinalIgnoreCase))"

    if (Test-Path -LiteralPath $statePath -PathType Leaf) {
        Write-Log "State backup: $statePath"
    }
    else {
        Write-Log 'State backup: not present'
    }
}

function Restore-FromState {
    param([string]$CurrentValue)

    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    if ([string]$state.KeyPath -ne $deviceKeyPath -or [string]::IsNullOrWhiteSpace([string]$state.OriginalValue)) {
        throw "The state file is invalid or belongs to another registry key: $statePath"
    }

    if ([string]$state.ExpectedValue -and $CurrentValue -ne [string]$state.ExpectedValue) {
        throw 'The registry value changed after spoofing. Refusing to overwrite the unexpected current value; the state file was preserved.'
    }

    Set-DeviceDescription -Value ([string]$state.OriginalValue)
    Remove-Item -LiteralPath $statePath -Force
    Write-Log "Restored original DeviceDesc: $($state.OriginalValue)"
}

function Enable-Spoof {
    param(
        [string]$CurrentValue,
        [pscustomobject]$Description
    )

    if (Test-Path -LiteralPath $statePath -PathType Leaf) {
        throw "A state backup already exists. Run the script again to restore it: $statePath"
    }

    $expectedValue = $Description.Prefix + $targetName
    $state = [ordered]@{
        KeyPath = $deviceKeyPath
        OriginalValue = $CurrentValue
        ExpectedValue = $expectedValue
        TargetName = $targetName
        CreatedAt = (Get-Date).ToString('o')
    }

    New-Item -ItemType Directory -Path $stateDirectory -Force | Out-Null
    [System.IO.File]::WriteAllText(
        $statePath,
        ($state | ConvertTo-Json -Depth 3),
        [System.Text.UTF8Encoding]::new($false)
    )

    try {
        Set-DeviceDescription -Value $expectedValue
    }
    catch {
        Write-Warning "Registry modification failed. The state backup was kept at $statePath."
        throw
    }

    Write-Log "Changed display name to: $targetName"
    Write-Log 'No reboot is performed. Restart applications that need to detect the new name.'
}

try {
    Assert-Administrator

    if ($Status) {
        Show-Status
        Show-CurrentStateDialog
        exit 0
    }

    $properties = Get-DeviceProperties
    $currentValue = [string]$properties.DeviceDesc
    $description = Split-DeviceDescription -Value $currentValue

    if ($description.DisplayName -ieq $targetName) {
        if (Test-Path -LiteralPath $statePath -PathType Leaf) {
            Restore-FromState -CurrentValue $currentValue
        }
        else {
            $fallbackName = Get-FallbackOriginalDescription
            $fallbackValue = $description.Prefix + $fallbackName
            Set-DeviceDescription -Value $fallbackValue
            Write-Log "No state backup was found. Restored from DriverDesc: $fallbackValue"
        }
    }
    else {
        Enable-Spoof -CurrentValue $currentValue -Description $description
    }

    Show-CurrentStateDialog
}
catch {
    Write-Error $_
    try {
        Add-Type -AssemblyName System.Windows.Forms -ErrorAction Stop
        [void][System.Windows.Forms.MessageBox]::Show(
            $_.Exception.Message,
            'NVIDIA 设备名称切换失败',
            [System.Windows.Forms.MessageBoxButtons]::OK,
            [System.Windows.Forms.MessageBoxIcon]::Error
        )
    }
    catch {
        # Keep the original PowerShell error if the graphical message box cannot open.
    }
    exit 1
}

}
catch {
    # Include the full error and source position even for errors before main setup.
    Write-Host ($_ | Format-List * -Force | Out-String) -ForegroundColor Red
    throw
}
finally {
    Stop-LauncherLog -Path $launcherLogPath
}
