[CmdletBinding()]
param(
    [switch]$Status,
    [Alias('r')]
    [switch]$ResetState
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot '..\common\ScriptLogging.ps1')
$launcherLogPath = Start-LauncherLog -ScriptPath $PSCommandPath
try {
. (Join-Path $PSScriptRoot '..\common\LauncherConfig.ps1')
$settings = Get-LauncherGpuSettings
$deviceKeyPath = 'HKLM:\SYSTEM\CurrentControlSet\Enum\' + $settings.DeviceInstanceId
$classKeyRoot = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class'
$targetName = $settings.TargetName
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
    $stateText = if ($enabled) { "状态：已伪装为 $targetName" } else { '状态：原始适配器名称' }

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
    if ($ResetState) { $arguments += ' -ResetState' }

    Write-Log 'Requesting administrator permission...'
    $elevated = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') -Verb RunAs -ArgumentList $arguments -PassThru -Wait
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
    if ($separator -lt 0 -and -not [string]::IsNullOrWhiteSpace($Value) -and -not $Value.StartsWith('@')) {
        return [pscustomobject]@{ Prefix = ''; DisplayName = $Value }
    }
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

    $classKeyPath = Join-Path $classKeyRoot $driverReference
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

    $state = [IO.File]::ReadAllText($statePath) | ConvertFrom-Json
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

function Reset-SpoofState {
    param([pscustomobject]$Description)

    if ($Description.DisplayName -ieq $targetName) {
        throw '-ResetState 不能在当前仍为伪装名称时使用；请正常运行脚本恢复原名称。'
    }
    if (Test-Path -LiteralPath $statePath -PathType Leaf) {
        Remove-Item -LiteralPath $statePath -Force -ErrorAction Stop
        Write-Log '已清除旧状态备份，将按当前设备名称重新备份并切换。'
    }
}

function Enable-Spoof {
    param(
        [string]$CurrentValue,
        [pscustomobject]$Description
    )

    if (Test-Path -LiteralPath $statePath -PathType Leaf) {
        throw "已有状态备份：$statePath。如果更新驱动后已恢复真实型号，请使用 -ResetState 重新备份并切换。"
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
    if ($Status -and $ResetState) { throw '-Status 与 -ResetState 不能同时使用。' }
    Assert-Administrator

    if ($Status) {
        Show-Status
        Show-CurrentStateDialog
        exit 0
    }

    $properties = Get-DeviceProperties
    $currentValue = [string]$properties.DeviceDesc
    $description = Split-DeviceDescription -Value $currentValue

    if ($ResetState) { Reset-SpoofState -Description $description }

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
