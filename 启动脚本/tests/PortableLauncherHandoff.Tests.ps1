# Targeted handoff tests: no Steam, game, UAC, package export or game deployment.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path -Parent $PSScriptRoot
function Assert-Test { param([bool]$Condition,[string]$Message) if (-not $Condition) { throw $Message } }
$ui=Join-Path $root 'scripts\ui\Show-SteamLauncher.ps1'
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($ui,[ref]$tokens,[ref]$errors)
Assert-Test ($errors.Count -eq 0) 'GUI syntax error.'
$node=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Start-Action'},$true)
. ([scriptblock]::Create($node.Extent.Text))
function Refresh-Game {}
function Get-PortableDataDirectory { return $env:TEMP }
function Set-PortableProfile { param($Preferences,$Profile) }
function Save-PortablePreferences { param($Preferences) }
function Write-PortableLaunchConfig { param($Model,$Directory) return 'C:\inert\launch.ini' }
function New-PortableWorkerCommand { param($ScriptPath,$ConfigPath,$AppId,$ResultPath,$Action) return '# inert' }
function Set-Working { param([bool]$Working) $script:busy=$Working }
function Show-Failure { param($Message) $script:failure=[string]$Message }
function Start-Process {
    param($FilePath,$ArgumentList,$PassThru,$Verb,$WindowStyle)
    if ($script:cancel) { throw 'UAC cancelled' }
    $script:observedVerb=$Verb
    $process=[pscustomobject]@{Disposed=$false;Killed=$false}
    $process | Add-Member ScriptMethod Dispose { $this.Disposed=$true }
    $process | Add-Member ScriptMethod Kill { $this.Killed=$true; throw 'Must never kill the worker.' }
    $script:createdWorker=$process
    return $process
}
foreach ($action in @('Launch','Restore','Check','CancelledLaunch')) {
    $script:preferences=@{}
    $script:model=[pscustomobject]@{Ready=$true;Recovery=$true;GameDirectory='C:\inert';Status='ready';Profile=[pscustomobject]@{GameDirectory='';AppId='456'}}
    $script:status=[pscustomobject]@{Text=''}
    $script:busy=$false; $script:failure=''; $script:worker=$null; $script:createdWorker=$null
    $script:cancel=$action -eq 'CancelledLaunch'
    $form=[pscustomobject]@{Hidden=$false;Closed=$false}
    $form | Add-Member ScriptMethod Hide { $this.Hidden=$true }
    $form | Add-Member ScriptMethod Close { $this.Closed=$true }
    $runAction=if ($script:cancel) { 'Launch' } else { $action }
    Start-Action $runAction
    if ($script:cancel) {
        Assert-Test (-not $form.Hidden -and -not $form.Closed -and $script:failure -match 'cancelled') 'Cancelled elevation hid or closed GUI.'
    } elseif ($action -eq 'Launch') {
        Assert-Test ($form.Hidden -and $form.Closed -and -not $script:busy) 'Launch did not release GUI.'
        Assert-Test ($script:createdWorker.Disposed -and -not $script:createdWorker.Killed -and $null -eq $script:worker) 'Worker was killed or handle retained.'
        Assert-Test ($script:observedVerb -eq 'RunAs') 'Launch lost elevation.'
    } else {
        Assert-Test (-not $form.Hidden -and -not $form.Closed -and $script:busy -and -not $script:createdWorker.Disposed) 'Check/restore no longer reports completion in GUI.'
    }
}
Write-Host 'PASS: actual GUI handoff, independent worker handle, cancelled UAC and retained check/restore UI.'

# Verify the moved BAT resolves its entry relative to itself, from any working directory.
$bat=Join-Path $root '启动器.bat'
$text=[IO.File]::ReadAllText($bat)
Assert-Test ($text.Contains('%~dp0scripts\ui\Show-SteamLauncher.ps1')) 'Moved BAT target is wrong.'
Assert-Test (-not [IO.File]::Exists((Join-Path (Split-Path -Parent $root) '启动器.bat'))) 'Old root entry remains.'
foreach ($relative in @('scripts\common\PortableLauncher.ps1','scripts\tools\Export-PortableLauncher.ps1')) {
    $tokens=$null; $errors=$null
    [void][Management.Automation.Language.Parser]::ParseFile((Join-Path $root $relative),[ref]$tokens,[ref]$errors)
    Assert-Test ($errors.Count -eq 0) "Syntax error: $relative"
}
Write-Host 'PASS: moved entry and Windows PowerShell syntax.'
