# Windows PowerShell 5.1; only isolated diagnostic files and synthetic logs.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path -Parent $PSScriptRoot
. (Join-Path $root 'scripts\common\LauncherResources.ps1')
. (Join-Path $root 'scripts\common\LauncherConfig.ps1')
$settings=@{SuperResolutionMode='Off';DlssgPreset='A';Optimized='2';Level='2'}
function Assert-Test($Value,[string]$Message) { if (-not $Value) { throw $Message } }
$testRoot=Join-Path $env:TEMP ('diagnostic041-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testRoot)
function Write-Text($Path,$Text) { [IO.File]::WriteAllText($Path,$Text,[Text.UTF8Encoding]::new($false)) }
foreach ($name in @('Run-WutheringWaves-DLSSG.ps1','Run-NevernessToEverness-DLSSG.ps1','Run-SteamUE5-DLSSG.ps1')) {
    $tokens=$null; $errors=$null
    $ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $root "scripts\games\$name"),[ref]$tokens,[ref]$errors)
    Assert-Test ($errors.Count -eq 0) ($errors | Out-String)
    foreach ($mode in @('Steam','Direct')) {
        $LaunchMode=$mode; $gameId=if ($mode -eq 'Steam') { 'steam-123' } else { 'direct-'+('a'*32) }
        $diagnosticIni040=$null; $diagnosticIni=$null; $legacyDiagnosticIni=$null; $previousDiagnosticIni=$null
        # Execute only the real configuration declarations, never the worker body.
        $nodes=$ast.FindAll({param($n)
            ($n -is [Management.Automation.Language.AssignmentStatementAst] -and $n.Left.Extent.Text -in @('$diagnosticIni040','$diagnosticIni','$diagnosticTemplate','$legacyDiagnosticIni','$previousDiagnosticIni')) -or
            ($n -is [Management.Automation.Language.IfStatementAst] -and $n.Extent.Text -match '^if \(\$LaunchMode -eq ''Direct''\) \{ \$diagnosticIni(?:040)?=')
        },$true)
        foreach ($node in $nodes) {
            if ($node.Parent -is [Management.Automation.Language.StatementBlockAst] -and $node.Parent.Parent -is [Management.Automation.Language.IfStatementAst]) { continue }
            . ([scriptblock]::Create($node.Extent.Text))
        }
        Assert-Test ($diagnosticIni -match 'Preset=A' -and $diagnosticIni -match 'Mode=Off' -and $diagnosticIni -notmatch 'Preset=Auto|Mode=On') "Invalid 0.4.1 config: $name/$mode"
        Assert-Test ($diagnosticIni -match 'Optimized=2' -and $diagnosticIni -match 'Mode=Bundled') 'Unrelated diagnostic policy changed.'
        $frames=5
        Assert-Test ($diagnosticIni.Contains("MaxGeneratedFrames=$frames")) 'Frame count changed.'
        if ($name -match 'SteamUE5' -and $mode -eq 'Direct') { Assert-Test ($diagnosticIni040 -match '; Direct UE 0.4.0') 'Direct historical text lost.' }
        foreach ($functionName in @('Get-NormalizedPath','Test-StaleDiagnosticIniTarget')) {
            $node=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $functionName},$true)
            . ([scriptblock]::Create($node.Extent.Text))
        }
        $prefix=if ($name -match 'Wuthering') { 'wuthering' } elseif ($name -match 'Neverness') { 'nte' } else { $gameId }
        $target=Join-Path $env:TEMP ("dlssg_sm86-$prefix-"+[string](Get-Random -Minimum 100000 -Maximum 999999)+'.ini')
        $linkSpecs=@(@{},@{Path=(Join-Path $testRoot 'dlssg_sm86.ini')})
        try {
            foreach ($text in @($diagnosticIni,$diagnosticIni040,$legacyDiagnosticIni,$previousDiagnosticIni)) {
                if (-not $text) { continue }
                Write-Text $target $text
                Assert-Test (Test-StaleDiagnosticIniTarget $linkSpecs[1].Path $target) "Known config rejected: $name/$mode"
            }
            foreach ($sr in @('Off','M-INT8','L-INT8','M-FP16','L-FP16')) {
                foreach ($preset in @('A','B','C')) {
                    foreach ($optimization in @('0','1','2','3')) {
                        foreach ($level in @('0','1','2','3')) {
                            $generated=Set-LauncherDiagnosticOptions $diagnosticTemplate $sr $preset $optimization $level
                            Assert-Test ($generated.Contains("Preset=$preset") -and $generated.Contains("Optimized=$optimization") -and $generated.Contains("Level=$level") -and ($generated -match ('(?m)^Mode='+[regex]::Escape($sr)+'\r?$')) -and $generated.Contains('Mode=Bundled')) 'Generated INI lost advanced selection.'
                            Write-Text $target $generated
                            Assert-Test (Test-StaleDiagnosticIniTarget $linkSpecs[1].Path $target) "New combo rejected: $sr/$preset/$optimization/$level/$mode"
                        }
                    }
                }
            }
            Write-Text $target (Set-LauncherDiagnosticOptions $diagnosticTemplate 'On' 'A')
            Assert-Test (-not (Test-StaleDiagnosticIniTarget $linkSpecs[1].Path $target)) 'Unjournaled invalid config claimed.'
            Write-Text $target ($diagnosticIni040.Replace('Optimized=2','Optimized=3'))
            Assert-Test (-not (Test-StaleDiagnosticIniTarget $linkSpecs[1].Path $target)) 'Unknown settings incorrectly claimed.'
            Assert-Test (-not (Test-StaleDiagnosticIniTarget (Join-Path $testRoot 'other.ini') $target)) 'Wrong link path accepted.'
            $outside=Join-Path $testRoot ([IO.Path]::GetFileName($target)); Write-Text $outside $diagnosticIni
            Assert-Test (-not (Test-StaleDiagnosticIniTarget $linkSpecs[1].Path $outside)) 'Non-TEMP target accepted.'
        } finally { if ([IO.File]::Exists($target)) { [IO.File]::Delete($target) } }
    }
}
Write-Host 'PASS: actual worker configs, Steam/Direct, old diagnostics, exact recognition and unchanged frame policy.'

$path=Join-Path $testRoot 'dlssg_sm86.log'
$old='2001-01-01 00:00:00.000  pid 1  Game.exe  I  {"event":"old"}'
Write-Text $path ($old+"`r`n")
$cursor=New-DlssgLogCursor $testRoot 'C:\fixture\Game.exe'
$cursor.StartedAt=[DateTime]::Now.AddSeconds(-1)
$time=[DateTime]::Now.ToString('yyyy-MM-dd HH:mm:ss.fff',[Globalization.CultureInfo]::InvariantCulture)
$lines=@("$time  pid 21  Game.exe  I  {`"event`":`"log_session`"}","$time  pid 22  Game.exe  D  {`"event`":`"evaluate`"}","$time  pid 23  Other.exe  E  {}",$old,'invalid')
[IO.File]::AppendAllText($path,(($lines -join "`r`n")+"`r`n"),[Text.UTF8Encoding]::new($false))
$result=Get-DlssgSessionLog $cursor
Assert-Test ($result.Exists -and $result.Records -eq 2 -and ($result.Pids -join ',') -eq '21,22') 'Old/foreign lines included or restart PID lost.'
$cursor=New-DlssgLogCursor $testRoot 'C:\fixture\Game.exe'
Assert-Test ((Get-DlssgSessionLog $cursor).Records -eq 0) 'Unchanged historical file reported as current.'
$cursor.StartedAt=[DateTime]::Now.AddSeconds(-1)
Write-Text $path "$time  pid 31  Game.exe  E  {}`r`n"
Assert-Test ((Get-DlssgSessionLog $cursor).Records -eq 1) 'Truncated log not handled.'
[IO.File]::Delete($path)
Write-Text (Join-Path $testRoot 'backend_1.jsonl') '{}'
Write-Text (Join-Path $testRoot 'frame_timing.txt') 'not a component log'
$result=Get-DlssgSessionLog $cursor
Assert-Test (-not $result.Exists -and $result.Records -eq 0 -and $result.LegacyPaths.Count -eq 1) 'Legacy file treated as current or frame timing selected.'
# A log created after snapshot is also recognized.
$cursor=New-DlssgLogCursor $testRoot 'C:\fixture\Game.exe'; $cursor.StartedAt=[DateTime]::Now.AddSeconds(-1)
Write-Text $path "$time  pid 41  Game.exe  I  {}`r`n"
Assert-Test ((Get-DlssgSessionLog $cursor).Records -eq 1) 'New log missed.'
# Hold with FileShare.None; status must warn rather than break recovery.
$stream=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
try { Show-DlssgSessionLogStatus $cursor -WarningVariable warnings; Assert-Test ($warnings.Count -gt 0) 'Locked log not reported.' }
finally { $stream.Dispose() }
Write-Host "PASS: log offsets, time/EXE filtering, multiple PIDs, truncation, missing/new/locked logs and legacy hints. Artifacts: $testRoot"
