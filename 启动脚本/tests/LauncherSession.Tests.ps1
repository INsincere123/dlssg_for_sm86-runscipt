# Isolated process polling tests. No game processes or deployments are used.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$launcherRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $launcherRoot 'scripts\common\LauncherSession.ps1')
function Assert-Test { param([bool]$Condition, [string]$Message) if (-not $Condition) { throw $Message } }
function Write-Log { param([string]$Message) $script:messages.Add($Message) }
function Get-Process {
    param([string[]]$Name, $ErrorAction)
    foreach ($processName in $Name) {
        if ($processName -eq 'test-game') {
            if ($script:gamePresent) { $script:fakeGame }
        }
        elseif ($processName -in @('test-launcher', 'test-main', 'test-updater')) {
            $state = $script:states[$script:poll]
            $expected = switch ($state) { 'launcher' { 'test-launcher' } 'main' { 'test-main' } 'updater' { 'test-updater' } }
            if ($processName -eq $expected) { [pscustomobject]@{ Id = 2; ProcessName = $processName } }
        }
        else { throw "Unexpected process query: $processName" }
    }
}
function Start-Sleep {
    param([int]$Seconds)
    $script:poll++
    if ($script:poll -ge $script:states.Count) { throw 'Polling exceeded the expected state sequence.' }
    $script:gamePresent = $script:states[$script:poll] -eq 'game'
    $script:launcherPresent = $script:states[$script:poll] -eq 'launcher'
}
$fakeGame = [pscustomobject]@{ Id = 1 }
$cases = @(
    @{ Name='game-present'; States=@('game'); Game=$true },
    @{ Name='timeout-before-launcher-appears'; States=@('absent'); Timeout=0 },
    @{ Name='timeout-with-launcher'; States=@('launcher'); Timeout=0 },
    @{ Name='delayed-launcher-startup'; States=@('absent','absent','launcher','game'); Game=$true },
    @{ Name='game-starts-without-observed-launcher'; States=@('absent','absent','game'); Game=$true },
    @{ Name='launcher-closes-after-observed'; States=@('launcher','absent'); Grace=0 },
    @{ Name='launcher-handoff'; States=@('launcher','absent','absent','launcher','game'); Game=$true },
    @{ Name='entry-updater-main-handoff'; States=@('launcher','updater','main','main','game'); Game=$true; Grace=0 },
    @{ Name='main-closes-after-bootstrap-exits'; States=@('launcher','main','main','absent'); Grace=0 },
    @{ Name='game-starts-during-closure-grace'; States=@('launcher','absent','game'); Game=$true }
)
foreach ($case in $cases) {
    $messages = [Collections.Generic.List[string]]::new()
    $states = $case.States
    $poll = 0
    $gamePresent = $states[0] -eq 'game'
    $launcherPresent = $states[0] -eq 'launcher'
    $timeout = if ($case.ContainsKey('Timeout')) { $case.Timeout } else { 120 }
    $grace = if ($case.ContainsKey('Grace')) { $case.Grace } else { 6 }
    $result = Wait-ForGameProcess -GameProcessName 'test-game' -LauncherProcessName @('test-launcher','test-main','test-updater') -TimeoutSeconds $timeout -ClosedGraceSeconds $grace
    if ($case.ContainsKey('Game')) {
        Assert-Test ($result -eq $fakeGame) "Game was not returned: $($case.Name)"
    }
    else {
        Assert-Test ($null -eq $result) "Expected normal cancellation: $($case.Name)"
        Assert-Test ($messages[$messages.Count - 1] -match '自动清理部署并关闭脚本') "Missing cleanup message: $($case.Name)"
    }
    if ($timeout -eq 0) {
        Assert-Test ($messages[$messages.Count - 1] -match '超过 0 秒') 'Timeout did not use the normal cancellation path.'
    }
    Assert-Test ($poll -eq ($states.Count - 1)) "Stopped before completing the expected sequence: $($case.Name)"
    Write-Host "PASS: $($case.Name)"
}

Assert-Test ((Get-LauncherProcessNames Nte 'C:\game\NTELauncher.exe') -contains 'NTEGame') 'NTE launcher main process missing.'
Assert-Test ((Get-LauncherProcessNames Wuthering 'C:\launcher\launcher.exe') -contains 'launcher_main') 'Wuthering launcher main process missing.'
Assert-Test ((Get-LauncherProcessNames Nte 'C:\game\custom.exe') -contains 'custom') 'Configured entry process was lost.'
Write-Host 'PASS: both launcher process families and custom entry preserved.'

# Exercise the actual manual recovery guard with only the launcher main alive.
& {
    $tokens = $null; $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $launcherRoot 'scripts\common\DeploymentJournal.ps1'), [ref]$tokens, [ref]$errors)
    $node = $ast.Find({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Assert-RecoveryProcessesStopped' }, $true)
    . ([scriptblock]::Create($node.Extent.Text))
    function Get-Process { param($ErrorAction) [pscustomobject]@{ ProcessName = 'test-main' } }
    $oldContext = if (Test-Path variable:script:journalContext) { $script:journalContext } else { $null }
    try {
        $script:journalContext = @{ GameProcess='test-game'; LauncherProcess=@('test-launcher','test-main') }
        $blocked = $false
        try { Assert-RecoveryProcessesStopped } catch { $blocked = $_.Exception.Message -eq 'Close test-main before recovery.' }
        Assert-Test $blocked 'Manual recovery ignored the launcher main process.'
    }
    finally { $script:journalContext = $oldContext }
}
Write-Host 'PASS: manual recovery blocked by launcher main after entry exits.'

foreach ($relative in @('scripts\common\LauncherSession.ps1', 'scripts\games\Run-WutheringWaves-DLSSG.ps1', 'scripts\games\Run-NevernessToEverness-DLSSG.ps1')) {
    $tokens = $null; $errors = $null
    [void][Management.Automation.Language.Parser]::ParseFile((Join-Path $launcherRoot $relative), [ref]$tokens, [ref]$errors)
    Assert-Test ($errors.Count -eq 0) "Syntax errors: $relative"
}
Write-Host 'PASS: syntax of the shared module and both game entry scripts.'
