# Isolated UE detection and quality selection tests; no game deployment or preferences touched.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path -Parent $PSScriptRoot
. (Join-Path $root 'scripts\common\LauncherConfig.ps1')
. (Join-Path $root 'scripts\common\PortableLauncher.ps1')
function Assert-Test { param([bool]$Condition,[string]$Message) if (-not $Condition) { throw $Message } }
$testRoot=Join-Path $env:TEMP ('dlssg-quality-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testRoot)
function New-Sample {
    param([string]$Name,[string]$Text,[switch]$Unicode,[int]$Prefix=0)
    $dir=Join-Path $testRoot $Name
    [void][IO.Directory]::CreateDirectory($dir)
    $exe=Join-Path $dir 'Game.exe'
    $enc=if ($Unicode) { [Text.Encoding]::Unicode } else { [Text.Encoding]::ASCII }
    $payload=$enc.GetBytes($Text)
    $bytes=New-Object byte[] ($Prefix+$payload.Length)
    [Array]::Copy($payload,0,$bytes,$Prefix,$payload.Length)
    [IO.File]::WriteAllBytes($exe,$bytes)
    return [pscustomobject]@{Directory=$dir;Executable=$exe}
}
function Detect { param($Sample) $task=[DlssgUnrealDetector]::Start($Sample.Directory,$Sample.Executable); return $task.GetAwaiter().GetResult() }
$a=New-Sample ascii '++UE4+Release-4.27'
$b=New-Sample unicode '++UE5+Release-5.3' -Unicode -Prefix 1
$c=New-Sample crossing '++UE5+Release-5.0' -Prefix (1048576-7)
$d=New-Sample crossingUnicode '++UE4+Release-4.26' -Unicode -Prefix (1048576-13)
$unknown=New-Sample unknown 'Game version 5.1 Shipping.exe'
Assert-Test ((Detect $a).Major -eq 4) 'ASCII UE4 failed.'
Assert-Test ((Detect $b).Major -eq 5) 'Unaligned UTF16 UE5 failed.'
Assert-Test ((Detect $c).Major -eq 5 -and (Detect $d).Major -eq 4) 'Cross-chunk marker failed.'
Assert-Test ((Detect $unknown).Major -eq 0) 'Generic product version was mistaken for UE.'
$conflict=New-Sample conflict '++UE4+Release-4.27 ++UE5+Release-5.1'
Assert-Test ((Detect $conflict).Major -eq 0) 'Mixed EXE evidence must be unknown.'
$build=Join-Path $a.Directory 'Engine\Build'
[void][IO.Directory]::CreateDirectory($build)
[IO.File]::WriteAllText((Join-Path $build 'Build.version'),'{"MajorVersion":5,"MinorVersion":0}')
Assert-Test ((Detect $a).Major -eq 0) 'Build and EXE conflict was ignored or build cache stale.'
[IO.File]::WriteAllText((Join-Path $build 'Build.version'),'{"MajorVersion":4,"MinorVersion":27}')
Assert-Test ((Detect $a).Major -eq 4) 'Build cache invalidation failed.'
$task1=[DlssgUnrealDetector]::Start($b.Directory,$b.Executable)
$task2=[DlssgUnrealDetector]::Start($b.Directory,$b.Executable)
Assert-Test ([object]::ReferenceEquals($task1,$task2)) 'Unchanged file was scanned again.'
[IO.File]::WriteAllText($b.Executable,'++UE4+Release-4.27 extra changed size')
Assert-Test ((Detect $b).Major -eq 4) 'EXE cache did not invalidate.'
$profile=[pscustomobject]@{Id='one';Name='One';AppId='123';RenderExecutable='Game.exe';ConfigDirectory='C:\inert';GameDirectory='';EngineConfigSource='配置\SteamUE5\Engine.ini';ProxyDll=1}
Initialize-PortableQuality $profile
Assert-Test ($profile.QualityPreset -eq 'Current' -and $profile.EngineOverride -eq 'Auto') 'Old preferences were not preserved.'
$current=(Resolve-PortableQuality $profile $null -Root $root).Path
$profile.QualityPreset='Tier2'
$blocked=$false
try { Resolve-PortableQuality $profile $null -Root $root | Out-Null } catch { $blocked=$true }
Assert-Test $blocked 'Unknown engine accepted a tier.'
foreach ($major in @(4,5)) {
    foreach ($tier in 1..5) {
        $profile.QualityPreset='Tier'+$tier
        $q=Resolve-PortableQuality $profile ([pscustomobject]@{Major=$major;Source='fixture'}) -Root $root
        Assert-Test ([IO.File]::Exists($q.Path) -and $q.Path.Contains("\UE$major\0$tier-")) 'Incorrect tier mapping.'
    }
}
$profile.EngineOverride='UE4'
$q=Resolve-PortableQuality $profile ([pscustomobject]@{Major=5;Source='fixture'}) -Root $root
Assert-Test ($q.Major -eq 4 -and $q.Source -eq '手动校准') 'Manual engine did not win.'
$profile.QualityPreset='Current'
Assert-Test ((Resolve-PortableQuality $profile $null -Root $root).Path -eq $current) 'Tier switch destroyed custom source.'
$prefs=@{SteamPath='';Profiles=@($profile)}
$directory=Join-Path $testRoot 'prefs'
Save-PortablePreferences $prefs -Directory $directory
$copy=$profile | ConvertTo-Json | ConvertFrom-Json
$copy.QualityPreset='Tier5'
Save-PortableProfileChange $prefs $copy -Directory $directory
$reloaded=Read-PortablePreferences -Directory $directory
Assert-Test ($reloaded.Profiles[0].QualityPreset -eq 'Tier5' -and $reloaded.Profiles[0].EngineOverride -eq 'UE4') 'Selection did not persist.'
$failed=$copy | ConvertTo-Json | ConvertFrom-Json
$failed.QualityPreset='Tier1'
$bad=Join-Path $testRoot 'not-a-directory'
[IO.File]::WriteAllText($bad,'inert')
$threw=$false
try { Save-PortableProfileChange $prefs $failed -Directory $bad } catch { $threw=$true }
Assert-Test ($threw -and $prefs.Profiles[0].QualityPreset -eq 'Tier5') 'Failed save did not roll back preferences.'
$other=$profile | ConvertTo-Json | ConvertFrom-Json
$other.Id='two';$other.AppId='456';$other.QualityPreset='Tier2'
Save-PortableProfileChange $prefs $other -Directory $directory
$reloaded=Read-PortablePreferences -Directory $directory
Assert-Test ($reloaded.Profiles.Count -eq 2 -and $reloaded.Profiles[0].QualityPreset -eq 'Tier5' -and $reloaded.Profiles[1].QualityPreset -eq 'Tier2') 'Per-game state leaked.'
Write-Host 'PASS: UE markers, overlap/UTF16, unknown/conflict, Build.version, cache, ten mappings including custom, overrides, legacy preferences, persistence and save rollback.'
Write-Host "Artifacts: $testRoot"

$ui=Join-Path $root 'scripts\ui\Show-SteamLauncher.ps1'
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($ui,[ref]$tokens,[ref]$errors)
Assert-Test ($errors.Count -eq 0) 'GUI syntax failed.'
$refreshNode=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Refresh-Game'},$true)
. ([scriptblock]::Create($refreshNode.Extent.Text))
function Get-PortableSteamPath { param($Preferences) return 'inert' }
function Get-PortableGameModel {
    param($Profile,$SteamPath,$EngineDetection)
    $major=0;$source='无法确定'
    if ($null -eq $EngineDetection) { $EngineDetection=Get-PortableSavedEngine $Profile }
    if ($null -ne $EngineDetection) {$major=$EngineDetection.Major;$source=$EngineDetection.Source}
    return [pscustomobject]@{Profile=$Profile;GameDirectory=$Profile.GameDirectory;Executable=(Join-Path $Profile.GameDirectory 'Game.exe');
        EngineMajor=$major;EngineSource=$source;Status='fixture';Ready=$true;Recovery=$script:recoverFixture;ConfigDirectory='inert';SteamPath='inert'}
}
$first=$profile | ConvertTo-Json | ConvertFrom-Json
$first.EngineOverride='Auto';$first.GameDirectory=$c.Directory
$second=$first | ConvertTo-Json | ConvertFrom-Json
$second.Id='second';$second.GameDirectory=$b.Directory
$games=[pscustomobject]@{SelectedItem=$first}
$script:preferences=@{SteamPath='';Profiles=@()}
$script:engineSaveFailed=@{}
function Get-PortableDataDirectory { return Join-Path $testRoot 'gui-engine-prefs' }
Add-Type -AssemblyName System.Drawing
$script:busy=$false;$script:recoverFixture=$false;$script:qualityIds=@('Current','Tier1','Tier2','Tier3','Tier4','Tier5')
$script:status=[pscustomobject]@{Text='';ForeColor=$null};$script:details=[pscustomobject]@{Text=''}
$launch=[pscustomobject]@{Enabled=$false};$restore=[pscustomobject]@{Enabled=$false};$check=[pscustomobject]@{Enabled=$false}
$choose=[pscustomobject]@{Enabled=$false};$advancedToggle=[pscustomobject]@{Enabled=$false}
$steamBox=[pscustomobject]@{Text='';Enabled=$true};$browseSteam=[pscustomobject]@{Enabled=$true};$sourceBox=[pscustomobject]@{Text=''}
$proxyBox=[pscustomobject]@{SelectedIndex=-1}
$qualityBox=[pscustomobject]@{SelectedIndex=-1;Enabled=$true};$engineBox=[pscustomobject]@{SelectedIndex=-1;Enabled=$true}
Refresh-Game
$lateFirst=[DlssgUnrealDetector]::Start($first.GameDirectory,(Join-Path $first.GameDirectory 'Game.exe'))
$games.SelectedItem=$second
[void][DlssgUnrealDetector]::Start($second.GameDirectory,(Join-Path $second.GameDirectory 'Game.exe')).GetAwaiter().GetResult()
[void]$lateFirst.GetAwaiter().GetResult()
Refresh-Game
Assert-Test ($script:model.Profile.Id -eq 'second' -and $script:model.EngineMajor -eq 4) 'Late first-game result overwrote selected game.'
$sourceBox.Text='unsaved.ini';$engineBox.SelectedIndex=2
Refresh-Game -PreserveEditors
Assert-Test ($sourceBox.Text -eq 'unsaved.ini' -and $engineBox.SelectedIndex -eq 2) 'Background completion discarded unsaved advanced edits.'
$script:recoverFixture=$true
Refresh-Game
Assert-Test (-not $qualityBox.Enabled -and -not $engineBox.Enabled -and $restore.Enabled) 'Recovery did not lock quality/engine controls.'
Write-Host 'PASS: actual GUI refresh ignores stale game result, preserves unsaved editors and locks quality during recovery.'

# A persisted successful result must work even after the EXE contents change.
$reopened=Read-PortablePreferences -Directory (Join-Path $testRoot 'gui-engine-prefs')
$persisted=@($reopened.Profiles | Where-Object Id -eq 'second')[0]
Assert-Test ($persisted.DetectedEngineMajor -eq 4) 'Detected engine was not saved.'
$games.SelectedItem=$persisted
$script:recoverFixture=$false
[IO.File]::WriteAllText((Join-Path $persisted.GameDirectory 'Game.exe'),'++UE5+Release-5.3 changed')
Refresh-Game
Assert-Test ($script:model.EngineMajor -eq 4 -and $null -eq $script:scanTask -and $script:model.EngineSource -match '已保存') 'Reopened game was rescanned or lost saved version.'
$persisted.EngineOverride='UE5'
Assert-Test ((Resolve-PortableQuality $persisted $null -Root $root).Major -eq 5) 'Manual calibration lost precedence over saved detection.'
$before=$persisted.DetectedEngineMajor
$invalid=[pscustomobject]@{Major=0;Source='conflict'}
Save-PortableEngineDetection $script:preferences $persisted $invalid -Directory (Join-Path $testRoot 'gui-engine-prefs')
Assert-Test ($persisted.DetectedEngineMajor -eq $before) 'Unknown result replaced a saved version.'
$failed=$persisted | ConvertTo-Json -Depth 6 | ConvertFrom-Json
$failed.DetectedEngineMajor=0
$originalProfiles=$script:preferences.Profiles
$failedWrite=$false
try { Save-PortableEngineDetection $script:preferences $failed ([pscustomobject]@{Major=5;Source='fixture'}) -Directory $bad } catch { $failedWrite=$true }
Assert-Test ($failedWrite -and $failed.DetectedEngineMajor -eq 0 -and [object]::ReferenceEquals($originalProfiles,$script:preferences.Profiles)) 'Engine save failure published unsaved state.'
Write-Host 'PASS: detected version persists per game, reopen skips scanning despite changed EXE, manual override wins, unknown is not saved and failed save rolls back.'
