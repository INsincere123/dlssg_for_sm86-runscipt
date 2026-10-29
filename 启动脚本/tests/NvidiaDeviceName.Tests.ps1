$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$tool = Join-Path $PSScriptRoot '..\scripts\tools\Toggle-NvidiaDeviceName.ps1'
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($tool, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
foreach ($node in $ast.FindAll({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] }, $true)) {
    if ($node.Name -in @('Split-DeviceDescription','Reset-SpoofState','Enable-Spoof','Restore-FromState')) {
        . ([scriptblock]::Create($node.Extent.Text))
    }
}
function Assert-Test { param($Condition, [string]$Message) if (-not $Condition) { throw $Message } }
function Write-Log { param([string]$Message) }
# Only import function bodies. Registry writes and the entire UI/admin entry are never invoked.
function Set-DeviceDescription {
    param([string]$Value)
    Assert-Test (Test-Path -LiteralPath $statePath -PathType Leaf) 'Must save backup before modifying device.'
    $script:writtenValue = $Value
}
$stateDirectory = Join-Path ([IO.Path]::GetTempPath()) ('NvidiaStateTests-' + [guid]::NewGuid().ToString('N'))
$statePath = Join-Path $stateDirectory 'state.json'
$deviceKeyPath = 'HKLM:\FAKE-DEVICE'
$targetName = 'NVIDIA GeForce RTX 5070'
$original = '@oem662.inf,%device%;NVIDIA GeForce RTX 3080 Ti Laptop GPU'
$description = Split-DeviceDescription $original
$writtenValue = ''
New-Item -ItemType Directory -Path $stateDirectory | Out-Null
try {
    [IO.File]::WriteAllText($statePath, '{"OriginalValue":"old-driver"}')
    $failed = $false
    try { Enable-Spoof $original $description } catch { $failed = $_.Exception.Message -match '-ResetState' }
    Assert-Test $failed 'Normal invocation must retain stale-state protection.'
    Assert-Test ($writtenValue -eq '') 'Normal invocation unexpectedly modified device.'
    Reset-SpoofState $description
    Enable-Spoof $original $description
    $state = [IO.File]::ReadAllText($statePath) | ConvertFrom-Json
    Assert-Test ($state.OriginalValue -ceq $original) 'New backup must use the current driver reference.'
    Assert-Test ($writtenValue -ceq '@oem662.inf,%device%;NVIDIA GeForce RTX 5070') 'Spoof must preserve current driver prefix.'
    $before = [IO.File]::ReadAllText($statePath)
    $failed = $false
    try { Reset-SpoofState (Split-DeviceDescription $writtenValue) } catch { $failed = $true }
    Assert-Test $failed 'Reset must refuse while spoofed.'
    Assert-Test ([IO.File]::ReadAllText($statePath) -ceq $before) 'Rejected reset changed backup.'
    Restore-FromState $writtenValue
    Assert-Test ($writtenValue -ceq $original) 'Restore must use the new driver original value.'
    Assert-Test (-not (Test-Path -LiteralPath $statePath)) 'Successful restore must remove backup.'
    Reset-SpoofState $description
    Enable-Spoof $original $description
    Assert-Test (Test-Path -LiteralPath $statePath) 'Reset without an existing state must allow normal enable.'

    # Simulate a failed delete and verify no subsequent enable can bypass the retained state.
    function Remove-Item { param($LiteralPath, [switch]$Force, $ErrorAction) throw 'Simulated deletion failure' }
    $failed = $false
    try { Reset-SpoofState $description } catch { $failed = $_.Exception.Message -eq 'Simulated deletion failure' }
    Assert-Test $failed 'Deletion failure must propagate.'
    Assert-Test (Test-Path -LiteralPath $statePath) 'Deletion failure must retain backup.'
    Microsoft.PowerShell.Management\Remove-Item Function:\Remove-Item -ErrorAction SilentlyContinue
} finally {
    Microsoft.PowerShell.Management\Remove-Item -LiteralPath $stateDirectory -Recurse -Force
}
Write-Host 'PASS: stale-state protection, reset/re-enable, current-driver restore, spoof rejection, absent state and deletion failure (isolated files, mocked registry).'
