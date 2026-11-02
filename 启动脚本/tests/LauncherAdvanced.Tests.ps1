# Isolated advanced options, actual WinForms controls and request/settings round trips.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path -Parent $PSScriptRoot
. (Join-Path $root 'scripts\common\LauncherConfig.ps1')
. (Join-Path $root 'scripts\common\SteamLauncher.ps1')
. (Join-Path $root 'scripts\common\DlssgOnlyLauncher.ps1')
function Assert-Test($Value,[string]$Message) { if (-not $Value) { throw $Message } }
function Expect-Failure([scriptblock]$Action) { $failed=$false; try { & $Action } catch { $failed=$true }; Assert-Test $failed 'Expected structural input rejection.' }
$testRoot=Join-Path $env:TEMP ('launcher-advanced-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testRoot)
$defaults=Get-LauncherAdvancedSettings @{}
Assert-Test ($defaults.SuperResolutionMode -ceq 'Off' -and $defaults.DlssgPreset -ceq 'A' -and $defaults.Optimized -ceq '2' -and $defaults.Level -ceq '2') 'Defaults changed.'
$warnings=@(); $auto=Resolve-LauncherAdvancedValue DlssgPreset Auto -WarningVariable warnings
Assert-Test ($auto -ceq 'A' -and $warnings.Count -eq 1) 'Auto migration missing.'
foreach ($invalid in @('On','Typo','invalid$1')) {
    Assert-Test ((Resolve-LauncherAdvancedValue SuperResolutionMode $invalid) -ceq $invalid) 'Invalid SR silently defaulted.'
    Assert-Test ((Resolve-LauncherAdvancedValue DlssgPreset $invalid) -ceq $invalid) 'Invalid preset silently defaulted.'
}
foreach ($invalid in @("Off`n[General]", "A`rB", "A`tB", ('A'+[char]0+'B'), '[x]', ';x', '#x', 'A=B')) {
    Expect-Failure { Resolve-LauncherAdvancedValue SuperResolutionMode $invalid }
    Expect-Failure { Resolve-LauncherAdvancedValue DlssgPreset $invalid }
}
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$form=[Windows.Forms.Form]::new()
try {
    $controls=New-PortableAdvancedControls $form 20 580
    Assert-Test ($controls.SuperResolutionMode.Items.Count -eq 5 -and $controls.DlssgPreset.Items.Count -eq 3) 'Wrong UI choices.'
    Assert-Test ($controls.SuperResolutionMode.DropDownStyle -eq 'DropDownList') 'UI permits arbitrary enum edits.'
    $old=[pscustomobject]@{Id='old'}
    Set-PortableAdvancedControls $controls $old
    Assert-Test ($old.SuperResolutionMode -eq 'Off' -and $old.DlssgPreset -eq 'A' -and $old.Optimized -eq '2' -and $old.Level -eq '2') 'Old profile not initialized.'
    Assert-Test ($controls.Optimized.Items.Count -eq 4 -and $controls.Level.Items.Count -eq 4) 'Wrong optimization/logging choices.'
    $bad=[pscustomobject]@{SuperResolutionMode='On';DlssgPreset='Typo';Optimized='99';Level='99'}
    Set-PortableAdvancedControls $controls $bad
    Assert-Test ($controls.SuperResolutionMode.SelectedIndex -eq -1 -and $controls.DlssgPreset.SelectedIndex -eq -1 -and $controls.Warning.Text) 'Invalid values hidden by UI.'
    Save-PortableAdvancedControls $controls $bad
    Assert-Test ($bad.SuperResolutionMode -ceq 'On' -and $bad.DlssgPreset -ceq 'Typo' -and $bad.Optimized -ceq '99' -and $bad.Level -ceq '99') 'Untouched invalid preference overwritten.'
    $controls.SuperResolutionMode.SelectedIndex=1; $controls.DlssgPreset.SelectedIndex=2
    Save-PortableAdvancedControls $controls $bad
    Assert-Test ($bad.SuperResolutionMode -eq 'M-INT8' -and $bad.DlssgPreset -eq 'C') 'Explicit UI repair failed.'
    foreach ($name in @('Optimized','Level')) {
        foreach ($value in @('0','1','2','3','99')) {
            $bad.$name=$value
            Set-PortableAdvancedControls $controls $bad
            Save-PortableAdvancedControls $controls $bad
            Assert-Test ($bad.$name -ceq $value) 'Numeric UI value lost.'
        }
        Expect-Failure { Resolve-LauncherAdvancedValue $name "2`n[General]" }
    }
    $key='a'*32
    $steam=Join-Path $testRoot 'steam.exe'; [IO.File]::WriteAllText($steam,'inert')
    $profiles=@()
    foreach ($mode in @('Steam','Direct')) {
        $id=if ($mode -eq 'Direct') { 'direct-'+$key } else { 'custom-123' }
        $p=[pscustomobject]@{Id=$id;Name=$mode;AppId='123';LaunchMode=$mode;GameDirectory=$testRoot;RenderExecutable='game.exe';LaunchExecutable='start.exe';ConfigDirectory=$testRoot;EngineConfigSource=$steam;ProxyDll=1;SuperResolutionMode='Off';DlssgPreset='A'}
        $profiles+=,$p
        $model=[pscustomobject]@{Profile=$p;SteamPath=$steam;GameDirectory=$testRoot;ConfigDirectory=$testRoot;EngineConfigSource=$steam;Recovery=$false}
        foreach ($sr in @('Off','M-INT8','L-INT8','M-FP16','L-FP16','On')) {
            foreach ($preset in @('A','B','C','Typo')) {
                Initialize-PortableAdvanced $p
                $p.SuperResolutionMode=$sr; $p.DlssgPreset=$preset
                $p.Optimized='0'; $p.Level='3'
                Set-PortableAdvancedControls $controls $p
                Save-PortableAdvancedControls $controls $p
                foreach ($only in @($false,$true)) {
                    $dir=Join-Path $testRoot ($mode+'-'+$only)
                    $path=if ($only) { Write-DlssgOnlyLaunchConfig $model $dir } else { Write-PortableLaunchConfig $model $dir }
                    $overrides=@{DlssgOnly=$only}
                    $settings=if ($mode -eq 'Direct') { Get-DirectUESettings -GameKey $key -Path $path -Overrides $overrides }
                        else { Get-SteamUE5Settings -AppId 123 -Profile desktop -Path $path -Overrides $overrides }
                    Assert-Test ($settings.SuperResolutionMode -ceq $sr -and $settings.DlssgPreset -ceq $preset -and $settings.Optimized -ceq '0' -and $settings.Level -ceq '3') 'Request lost advanced options.'
                    $overrides.SuperResolutionMode='L-FP16'; $overrides.DlssgPreset='B'; $overrides.Optimized='3'; $overrides.Level='0'
                    $settings=if ($mode -eq 'Direct') { Get-DirectUESettings -GameKey $key -Path $path -Overrides $overrides }
                        else { Get-SteamUE5Settings -AppId 123 -Profile desktop -Path $path -Overrides $overrides }
                    Assert-Test ($settings.SuperResolutionMode -ceq 'L-FP16' -and $settings.DlssgPreset -ceq 'B' -and $settings.Optimized -ceq '3' -and $settings.Level -ceq '0') 'CLI override lost.'
                }
            }
        }
    }
    $profiles[0].SuperResolutionMode='M-INT8'; $profiles[0].DlssgPreset='B'
    $profiles[1].SuperResolutionMode='L-FP16'; $profiles[1].DlssgPreset='C'
    $fixture=Join-Path $testRoot 'launcher'
    [void][IO.Directory]::CreateDirectory((Join-Path $fixture 'presets'))
    foreach ($count in @(1,2)) {
        $presets=@([pscustomobject]@{Id='custom-123';Name='preset';AppId='123';RenderExecutable='game.exe';ConfigDirectory=$testRoot;EngineConfigSource=$steam})
        if ($count -eq 2) { $presets+=([pscustomobject]@{Id='second';Name='second';AppId='456';RenderExecutable='game.exe';ConfigDirectory=$testRoot;EngineConfigSource=$steam}) }
        [IO.File]::WriteAllText((Join-Path $fixture 'presets\SteamUE5.json'),(ConvertTo-Json -InputObject $presets),[Text.UTF8Encoding]::new($true))
        $merged=@(Get-PortableProfiles @{SteamPath=$steam;Profiles=$profiles} -Root $fixture)
        $saved=@($merged | Where-Object Id -eq 'custom-123')
        Assert-Test ($saved.Count -eq 1 -and $saved[0].SuperResolutionMode -eq 'M-INT8' -and $saved[0].DlssgPreset -eq 'B') 'Preset merge lost advanced preference.'
        Assert-Test (@($merged | Where-Object Id -eq ('direct-'+$key)).Count -eq 1) 'Merge lost custom direct profile.'
        if ($count -eq 2) { Assert-Test (($merged | Where-Object Id -eq 'second').SuperResolutionMode -eq 'Off') 'New preset defaults lost.' }
    }
    foreach ($dir in @('general-preferences','unreal-preferences')) {
        $prefs=@{SteamPath=$steam;Profiles=$profiles}
        $path=Join-Path $testRoot $dir
        Save-PortablePreferences $prefs -Directory $path
        $loaded=Read-PortablePreferences -Directory $path
        foreach ($p in $loaded.Profiles) { Set-PortableAdvancedControls $controls $p; Save-PortableAdvancedControls $controls $p }
        Assert-Test ($loaded.Profiles.Count -eq 2 -and $loaded.Profiles[0].SuperResolutionMode -eq 'M-INT8' -and $loaded.Profiles[1].DlssgPreset -eq 'C' -and $loaded.Profiles[0].Optimized -eq '0' -and $loaded.Profiles[1].Level -eq '3') 'Save/reopen/game switch lost choices.'
    }
} finally { $form.Dispose() }
Write-Host "PASS: defaults, migration, invalid enums, structural rejection, actual UI controls, 15 combinations, Steam/Direct and both request paths, overrides and preference reopen. Artifacts: $testRoot"

# Official-launcher entries share the same advanced parser; no real game paths.
$officialIni=Join-Path $testRoot 'official-launchers.ini'
$base="游戏目录=$testRoot`r`n启动器路径=$steam`r`n配置源=$steam`r`n游戏配置目录=$testRoot`r`n"
foreach ($game in @('Wuthering','Nte')) {
    $label=if ($game -eq 'Wuthering') { '鸣潮' } else { '异环' }
    $text="[$label]`r`n"+$base
    if ($game -eq 'Wuthering') { $text+="重启等待秒数=10`r`n" }
    [IO.File]::WriteAllText($officialIni,$text,[Text.UTF8Encoding]::new($true))
    $settings=Get-LauncherGameSettings -GameId $game -Path $officialIni
    Assert-Test ($settings.SuperResolutionMode -ceq 'Off' -and $settings.DlssgPreset -ceq 'A' -and $settings.Optimized -ceq '2' -and $settings.Level -ceq '2') 'Official launcher defaults changed.'
    $text+="SuperResolutionMode=L-INT8`r`nDlssgPreset=C`r`nOptimized=1`r`nLevel=3`r`n"
    [IO.File]::WriteAllText($officialIni,$text,[Text.UTF8Encoding]::new($true))
    $settings=Get-LauncherGameSettings -GameId $game -Path $officialIni
    Assert-Test ($settings.SuperResolutionMode -ceq 'L-INT8' -and $settings.DlssgPreset -ceq 'C' -and $settings.Optimized -ceq '1' -and $settings.Level -ceq '3') 'Official launcher INI ignored.'
    $settings=Get-LauncherGameSettings -GameId $game -Path $officialIni -Overrides @{SuperResolutionMode='M-FP16';DlssgPreset='B';Optimized='0';Level='0'}
    Assert-Test ($settings.SuperResolutionMode -ceq 'M-FP16' -and $settings.DlssgPreset -ceq 'B' -and $settings.Optimized -ceq '0' -and $settings.Level -ceq '0') 'Official launcher CLI precedence failed.'
    $settings=Get-LauncherGameSettings -GameId $game -Path $officialIni -Overrides @{SuperResolutionMode='On';DlssgPreset='Typo';Optimized='99';Level='99'}
    Assert-Test ($settings.SuperResolutionMode -ceq 'On' -and $settings.DlssgPreset -ceq 'Typo' -and $settings.Optimized -ceq '99' -and $settings.Level -ceq '99') 'Official launcher invalid values silently defaulted.'
    Expect-Failure { Get-LauncherGameSettings -GameId $game -Path $officialIni -Overrides @{SuperResolutionMode="On`n[General]"} }
    if ($game -eq 'Wuthering') { Assert-Test ($settings.RestartGraceSeconds -eq 10) 'Restart window changed.' }
}
Write-Host 'PASS: Wuthering/Nte sections, missing defaults, INI values, CLI precedence, invalid passthrough and structural input rejection.'
