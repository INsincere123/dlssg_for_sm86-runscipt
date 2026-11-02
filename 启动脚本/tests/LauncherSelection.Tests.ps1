# Actual desktop reload/selection callbacks with isolated preferences. No game/UAC.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$sourceRoot=Split-Path -Parent $PSScriptRoot
. (Join-Path $sourceRoot 'scripts\common\LauncherConfig.ps1')
. (Join-Path $sourceRoot 'scripts\common\PortableLauncher.ps1')
Add-Type -AssemblyName System.Windows.Forms
function Assert-Test($Value,[string]$Message) { if (-not $Value) { throw $Message } }
$testRoot=Join-Path $env:TEMP ('launcher-selection-'+[guid]::NewGuid().ToString('N'))
$fixture=Join-Path $testRoot 'launcher'
[void][IO.Directory]::CreateDirectory((Join-Path $fixture 'presets'))
[IO.File]::WriteAllText((Join-Path $fixture 'presets\SteamUE5.json'),'[]')
function Get-LauncherRoot { return $fixture }
function Get-PortableDataDirectory { return $dataDirectory }
function Refresh-Game { $script:refreshed=$games.SelectedItem }
function Show-Failure($Message) { throw $Message }
foreach ($ui in @('Show-DlssgLauncher.ps1','Show-SteamLauncher.ps1')) {
    $dataDirectory=Join-Path $testRoot $ui
    [void][IO.Directory]::CreateDirectory($dataDirectory)
    # Old Version=1 documents have no selection property.
    $profiles=@(
        [pscustomobject]@{Id='custom-123';Name='one';AppId='123';RenderExecutable='game.exe';ConfigDirectory=$testRoot;EngineConfigSource='source.ini';ProxyDll=1},
        [pscustomobject]@{Id='custom-456';Name='two';AppId='456';RenderExecutable='game.exe';ConfigDirectory=$testRoot;EngineConfigSource='source.ini';ProxyDll=1}
    )
    $old=@{Version=1;SteamPath='';Profiles=$profiles}
    [IO.File]::WriteAllText((Join-Path $dataDirectory 'preferences.json'),($old | ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($true))
    $script:preferences=Read-PortablePreferences $dataDirectory
    Assert-Test ($script:preferences.LastSelectedProfileId -ceq '') 'Old preferences failed migration.'
    $games=if ($ui -match 'Dlssg') { [Windows.Forms.ListBox]::new() } else { [Windows.Forms.ComboBox]::new() }
    try {
        $tokens=$null; $errors=$null
        $ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $sourceRoot "scripts\ui\$ui"),[ref]$tokens,[ref]$errors)
        Assert-Test ($errors.Count -eq 0) 'UI parser failed.'
        $reloadName=if ($ui -match 'Dlssg') { 'Reload-Games' } else { 'Load-Games' }
        $node=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $reloadName},$true)
        . ([scriptblock]::Create($node.Extent.Text))
        $node=$ast.Find({param($n) $n -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $n.Extent.Text.StartsWith('$games.Add_SelectedIndexChanged(')},$true)
        $callbackText=$node.Arguments[0].ScriptBlock.Extent.Text
        $callback=[scriptblock]::Create($callbackText.Substring(1,$callbackText.Length-2))
        & $reloadName
        Assert-Test ($games.SelectedIndex -eq 0) 'First use must select first available game.'
        $games.SelectedIndex=1
        & $callback
        Assert-Test ($script:refreshed.Id -eq 'custom-456') 'Selection callback did not refresh chosen game.'
        $script:preferences=Read-PortablePreferences $dataDirectory
        Assert-Test ($script:preferences.LastSelectedProfileId -ceq 'custom-456') 'Selection not saved immediately.'
        & $reloadName
        Assert-Test ($games.SelectedItem.Id -eq 'custom-456') 'Reopen reset to first game.'
        $script:preferences.Profiles=@($script:preferences.Profiles[1],$script:preferences.Profiles[0])
        & $reloadName
        Assert-Test ($games.SelectedIndex -eq 0 -and $games.SelectedItem.Id -eq 'custom-456') 'Selection used list position instead of stable ID.'
        & $reloadName 'custom-123'
        Assert-Test ($games.SelectedItem.Id -eq 'custom-123' -and (Read-PortablePreferences $dataDirectory).LastSelectedProfileId -eq 'custom-123') 'Explicit save/add selection not remembered.'
        $script:preferences.Profiles=@($script:preferences.Profiles | Where-Object Id -ne 'custom-123')
        & $reloadName
        Assert-Test ($games.SelectedItem.Id -eq 'custom-456') 'Deleted remembered game did not fall back.'
        $script:preferences.Profiles=@()
        & $reloadName
        Assert-Test ($games.SelectedIndex -eq -1) 'Empty list failed.'
    } finally { $games.Dispose() }
}
$blocked=Join-Path $testRoot 'not-a-directory'; [IO.File]::WriteAllText($blocked,'inert')
$prefs=@{SteamPath='';Profiles=@();LastSelectedProfileId='old'}
$failed=$false
try { Save-PortableSelectedProfile $prefs 'new' -Directory $blocked } catch { $failed=$true }
Assert-Test ($failed -and $prefs.LastSelectedProfileId -ceq 'old') 'Failed preference save did not roll back selection.'
$prefs=@{SteamPath='';Profiles=@()}
try { Save-PortableSelectedProfile $prefs 'new' -Directory $blocked } catch {}
Assert-Test (-not $prefs.ContainsKey('LastSelectedProfileId')) 'Failed migration left a phantom selection.'
Write-Host "PASS: both actual UI reload/selection callbacks, old preferences, immediate save, reopen, reordered/removed/empty lists, explicit selection and failed-save rollback. Artifacts: $testRoot"

# Removed Current is migrated only while reading preferences and durably saved.
$migrationDir=Join-Path $testRoot 'current-cleanup'
[void][IO.Directory]::CreateDirectory($migrationDir)
$migrationPath=Join-Path $migrationDir 'preferences.json'
$old=@{Version=1;SteamPath='C:\fixture\steam.exe';LastSelectedProfileId='second';Profiles=@(
    [pscustomobject]@{Id='first';QualityPreset='Current';GameDirectory='C:\fixture\game';SuperResolutionMode='L-INT8';Optimized='1'},
    [pscustomobject]@{Id='second';QualityPreset='Tier5';EngineConfigSource='original.ini'},
    [pscustomobject]@{Id='missing-quality';Name='old profile'},
    [pscustomobject]@{Id='unknown-quality';QualityPreset='Typo'}
)}
[IO.File]::WriteAllText($migrationPath,($old | ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($true))
$migrated=Read-PortablePreferences -Directory $migrationDir
$disk=[IO.File]::ReadAllText($migrationPath) | ConvertFrom-Json
Assert-Test ($migrated.Profiles[0].QualityPreset -ceq 'Tier2' -and $disk.Profiles[0].QualityPreset -ceq 'Tier2') 'Current cleanup was not durable.'
Assert-Test ($disk.Profiles[0].SuperResolutionMode -ceq 'L-INT8' -and $disk.Profiles[0].Optimized -ceq '1' -and $disk.Profiles[0].GameDirectory -ceq 'C:\fixture\game') 'Cleanup changed other game settings.'
Assert-Test ($disk.Profiles[1].QualityPreset -ceq 'Tier5' -and $disk.Profiles[1].EngineConfigSource -ceq 'original.ini' -and $disk.LastSelectedProfileId -ceq 'second' -and $disk.SteamPath -ceq $old.SteamPath) 'Cleanup changed selection/source/unrelated profile.'
Assert-Test ($null -eq $disk.Profiles[2].PSObject.Properties['QualityPreset'] -and $disk.Profiles[3].QualityPreset -ceq 'Typo') 'Cleanup silently rewrote unrelated invalid/missing values.'
$saveImplementation=${function:Save-PortablePreferences}
function Save-PortablePreferences { throw 'fixture write refused' }
try {
    # A subsequent read must not save or require write permission.
    $reopened=Read-PortablePreferences -Directory $migrationDir
    Assert-Test ($reopened.Profiles[0].QualityPreset -ceq 'Tier2') 'Reopen failed after migration.'
    $old.Profiles=@([pscustomobject]@{Id='single';QualityPreset='Current'})
    [IO.File]::WriteAllText($migrationPath,($old | ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($true))
    $before=[IO.File]::ReadAllBytes($migrationPath)
    $failed=$false
    try { Read-PortablePreferences -Directory $migrationDir | Out-Null } catch { $failed=$_.Exception.Message -match '无法保存为主流均衡' }
    Assert-Test $failed 'Migration write failure was hidden.'
    Assert-Test ([Convert]::ToBase64String($before) -ceq [Convert]::ToBase64String([IO.File]::ReadAllBytes($migrationPath))) 'Failed migration damaged original preference.'
} finally { Set-Item Function:\Save-PortablePreferences $saveImplementation }
$single=Read-PortablePreferences -Directory $migrationDir
Assert-Test ($single.Profiles.Count -eq 1 -and $single.Profiles[0].QualityPreset -ceq 'Tier2') 'Single-profile cleanup failed.'
Write-Host 'PASS: Current preference cleanup, single/multiple profiles, durable reopen without rewrite, unrelated values preserved and failed-save original preserved.'
