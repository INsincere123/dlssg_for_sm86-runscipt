#Requires -Version 5.1
<#
.SYNOPSIS
    M0 黄金样本生成器：调用现有 PowerShell 实现，在隔离临时目录里产出 rust/fixtures 的字节级样本。

.DESCRIPTION
    只读现有源码、只在 %TEMP% 下写临时文件；不启动游戏、不写真实游戏目录、不碰仓库 .state。
    产出的样本是 M1 差分测试的判据：Rust 实现必须复现同样的输出。

.PARAMETER FixtureRoot
    样本输出目录，默认 rust/fixtures。

.PARAMETER KeepTemporary
    保留临时目录用于人工核对。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File rust\tools\gen-fixtures.ps1
#>
[CmdletBinding()]
param(
    [string]$FixtureRoot,
    [switch]$KeepTemporary
)

$ErrorActionPreference = 'Stop'

$toolsDir = $PSScriptRoot
$rustRoot = Split-Path -Parent $toolsDir
$repoRoot = Split-Path -Parent $rustRoot
$launcherRoot = Join-Path $repoRoot '启动脚本'
if (-not $FixtureRoot) { $FixtureRoot = Join-Path $rustRoot 'fixtures' }

if (-not (Test-Path -LiteralPath (Join-Path $launcherRoot 'scripts\common\DeploymentJournal.ps1'))) {
    throw "找不到现有实现：$launcherRoot"
}

$written = [Collections.Generic.List[object]]::new()
$sha256 = [Security.Cryptography.SHA256]::Create()

function Get-Sha256Text {
    param([byte[]]$Bytes)
    return [BitConverter]::ToString($sha256.ComputeHash($Bytes)).Replace('-', '')
}

function Write-FixtureBytes {
    param([string]$RelativePath, [byte[]]$Bytes)
    $target = Join-Path $FixtureRoot $RelativePath
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $target))
    [IO.File]::WriteAllBytes($target, $Bytes)
    $written.Add([pscustomobject]@{
        Path = ($RelativePath -replace '\\', '/')
        Bytes = $Bytes.Length
        Sha256 = (Get-Sha256Text $Bytes)
    })
}

function Write-FixtureText {
    param([string]$RelativePath, [string]$Text)
    Write-FixtureBytes $RelativePath ([Text.UTF8Encoding]::new($false).GetBytes($Text))
}

function Get-ScriptStringVariable {
    param([string]$Path, [string]$Name)
    $tokens = $null; $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$errors)
    if ($errors.Count -gt 0) { throw "语法错误：$Path（$($errors[0].Message)）" }
    $values = @()
    foreach ($node in $ast.FindAll({ param($n) $n -is [Management.Automation.Language.AssignmentStatementAst] }, $true)) {
        if ($node.Left -isnot [Management.Automation.Language.VariableExpressionAst]) { continue }
        if ($node.Left.VariablePath.UserPath -ne $Name) { continue }
        $right = $node.Right
        # 赋值右侧的实际 AST 是 CommandExpressionAst（`$x = @'...'@`）。
        if ($right -is [Management.Automation.Language.CommandExpressionAst]) { $right = $right.Expression }
        if ($right -isnot [Management.Automation.Language.StringConstantExpressionAst]) { continue }
        $values += , $right.Value
    }
    if ($values.Count -ne 1) { throw "变量 $Name 在 $Path 中匹配到 $($values.Count) 处字面量赋值，期望 1 处。" }
    return [string]$values[0]
}

# ---------------------------------------------------------------- 1. 诊断 INI
. (Join-Path $launcherRoot 'scripts\common\LauncherConfig.ps1')

$games = @(
    @{ Id = 'wuthering'; Script = 'Run-WutheringWaves-DLSSG.ps1'; Templates = @{ '0.4.2' = 'diagnosticIni'; '0.4.0' = 'diagnosticIni040'; '0.3.2' = 'legacyDiagnosticIni' } },
    @{ Id = 'neverness'; Script = 'Run-NevernessToEverness-DLSSG.ps1'; Templates = @{ '0.4.2' = 'diagnosticIni'; '0.4.0' = 'diagnosticIni040' } },
    @{ Id = 'steam-ue5'; Script = 'Run-SteamUE5-DLSSG.ps1'; Templates = @{ '0.4.2' = 'diagnosticIni'; '0.4.0' = 'diagnosticIni040' } }
)

$combos = @(
    @{ Name = 'default'; Mode = 'Off'; Preset = 'A'; Optimized = '2'; Level = '2' },
    @{ Name = 'sr-m-int8'; Mode = 'M-INT8'; Preset = 'C'; Optimized = '3'; Level = '3' },
    @{ Name = 'sr-l-fp16'; Mode = 'L-FP16'; Preset = 'B'; Optimized = '0'; Level = '0' },
    @{ Name = 'fair'; Mode = 'Off'; Preset = 'B'; Optimized = '1'; Level = '1' }
)

$templateIndex = [Collections.Generic.List[object]]::new()
$mainTemplate = $null
foreach ($game in $games) {
    $scriptPath = Join-Path $launcherRoot ('scripts\games\' + $game.Script)
    foreach ($version in ($game.Templates.Keys | Sort-Object)) {
        $variable = $game.Templates[$version]
        $template = Get-ScriptStringVariable $scriptPath $variable
        $relative = 'templates/{0}/{1}.ini' -f $game.Id, $version
        Write-FixtureText $relative $template
        $templateIndex.Add([pscustomobject]@{
            Game = $game.Id; Version = $version; Variable = $variable
            Source = '启动脚本/scripts/games/' + $game.Script
            Fixture = ($relative -replace '\\', '/')
        })
        if ($version -eq '0.4.2') {
            if ($game.Id -eq 'wuthering') { $mainTemplate = $template }
            foreach ($combo in $combos) {
                $text = Set-LauncherDiagnosticOptions -Template $template `
                    -SuperResolutionMode $combo.Mode -DlssgPreset $combo.Preset `
                    -Optimized $combo.Optimized -Level $combo.Level
                Write-FixtureText ('diagnostic/{0}/{1}.ini' -f $game.Id, $combo.Name) $text
            }
        }
    }
}
Write-FixtureText 'templates/index.json' (ConvertTo-Json -InputObject @($templateIndex) -Depth 5)

# 派生模板：鸣潮用 0.4.0 模板的 Mode=On → Mode=Off 变体识别更早的部署
# （Run-WutheringWaves-DLSSG.ps1:140）。逐字复刻同一条替换。
$wutheringScript = Join-Path $launcherRoot 'scripts\games\Run-WutheringWaves-DLSSG.ps1'
$board040 = Get-ScriptStringVariable $wutheringScript 'diagnosticIni040'
$derived = $board040.Replace('[SuperResolution]' + "`r`n" + 'Mode=On', '[SuperResolution]' + "`r`n" + 'Mode=Off')
Write-FixtureText 'templates/wuthering/0.4.0-mode-off.ini' $derived
Write-FixtureText 'templates/derived-index.json' (ConvertTo-Json -InputObject @([pscustomobject]@{
    Game = 'wuthering'; Version = '0.4.0-mode-off'
    Derivation = "diagnosticIni040.Replace('[SuperResolution]`r`nMode=On', '[SuperResolution]`r`nMode=Off')"
    Source = '启动脚本/scripts/games/Run-WutheringWaves-DLSSG.ps1:140'
    Fixture = 'templates/wuthering/0.4.0-mode-off.ini'
}) -Depth 4)

# ------------------------------------------------- 2. 诊断文本识别（契约 7）
$exact = Set-LauncherDiagnosticOptions -Template $mainTemplate -SuperResolutionMode 'Off' -DlssgPreset 'A' -Optimized '2' -Level '2'
$recognitionSource = @(
    @{ Name = 'exact'; Text = $exact },
    @{ Name = 'value-changed'; Text = $exact.Replace('Optimized=2', 'Optimized=3') },
    @{ Name = 'extra-section'; Text = ($exact + "`r`n`r`n[Extra]`r`nFoo=1") },
    @{ Name = 'duplicate-key'; Text = [regex]::Replace($exact, '(?m)^Level=2\r?$', "Level=2`r`nLevel=2") },
    @{ Name = 'duplicate-key-same-value'; Text = [regex]::Replace($exact, '(?m)^Optimized=2\r?$', "Optimized=2`r`nOptimized=2") },
    @{ Name = 'comment-changed'; Text = $exact.Replace('; 鸣潮 0.4.1 临时诊断配置。', '; changed comment') },
    @{ Name = 'unsupported-value'; Text = $exact.Replace('Level=2', 'Level=9') },
    @{ Name = 'trailing-space'; Text = $exact.Replace('Level=2', 'Level=2 ') },
    @{ Name = 'crlf-to-lf'; Text = $exact.Replace("`r`n", "`n") }
)
$recognition = foreach ($case in $recognitionSource) {
    Write-FixtureText ('diagnostic/recognition/{0}.ini' -f $case.Name) $case.Text
    [pscustomobject]@{
        Case = $case.Name
        Accepted = (Test-LauncherDiagnosticOptionsText -Contents $case.Text -Template $mainTemplate)
    }
}
Write-FixtureText 'diagnostic/recognition.json' (ConvertTo-Json -InputObject ([pscustomobject]@{
    Source = '启动脚本/scripts/common/LauncherConfig.ps1:409-421（Test-LauncherDiagnosticOptionsText）'
    Template = 'templates/wuthering/0.4.2.ini'
    Cases = @($recognition)
}) -Depth 5)

# ------------------------------------------------------------ 3. 路径归一化
$pathExpression = "[System.IO.Path]::GetFullPath(`$Path).TrimEnd('\').ToLowerInvariant()"
$pathInputs = @(
    'D:\Games\Foo\',
    'd:\games\foo',
    'D:\Games\..\Games\.\Foo\\',
    'D:\Games\\Foo\\Bar',
    'C:\',
    'C:',
    '\\?\D:\Games\Foo',
    '\\server\share\Game',
    'C:\游戏\黑神话悟空',
    '%TEMP%\dlssg_sm86-1234.ini',
    '.\relative\path',
    'D:\Games\Foo\Engine.ini',
    'D:\Games\FOO\engine.INI'
)
$pathCases = foreach ($input in $pathInputs) {
    $expected = [System.IO.Path]::GetFullPath($input).TrimEnd('\').ToLowerInvariant()
    [pscustomobject]@{ Input = $input; Expected = $expected }
}
Write-FixtureText 'paths/normalized-path.json' (ConvertTo-Json -InputObject ([pscustomobject]@{
    Source = 'Run-WutheringWaves-DLSSG.ps1:168 / Run-NevernessToEverness-DLSSG.ps1:140 / Run-SteamUE5-DLSSG.ps1:169（三处定义逐字相同）'
    Expression = $pathExpression
    WorkingDirectory = (Get-Location).Path
    PowerShellVersion = $PSVersionTable.PSVersion.ToString()
    Cases = @($pathCases)
}) -Depth 6)

# ------------------------------------------------- 2b. 配置编码样本（契约 C10）
$configText = @(
    '; 编码兼容样本：逻辑内容相同，仅文件编码不同',
    '# 井号注释同样跳过',
    '[鸣潮]',
    '游戏目录=D:\Games\WuWa',
    '配置源="配置\鸣潮\Engine.ini"',
    '启动器路径=C:\a#b=c\launcher.exe',
    '游戏配置目录=',
    '重启等待秒数=10',
    '[Gpu]',
    '设备实例标识=HDAUDIO\FUNC_01',
    ''
) -join "`r`n"

$utf8 = [Text.UTF8Encoding]::new($false)
$utf8Bom = [Text.UTF8Encoding]::new($true)
$utf16Le = [Text.UnicodeEncoding]::new($false, $true)
$utf16Be = [Text.UnicodeEncoding]::new($true, $true)
$utf32Le = [Text.UTF32Encoding]::new($false, $true)
$utf32Be = [Text.UTF32Encoding]::new($true, $true)
$gbk = [Text.Encoding]::GetEncoding(936)

$encodingSamples = @(
    @{ Name = 'utf8'; Bytes = $utf8.GetBytes($configText) },
    @{ Name = 'utf8-bom'; Bytes = $utf8Bom.GetPreamble() + $utf8Bom.GetBytes($configText) },
    @{ Name = 'utf16-le'; Bytes = $utf16Le.GetPreamble() + $utf16Le.GetBytes($configText) },
    @{ Name = 'utf16-be'; Bytes = $utf16Be.GetPreamble() + $utf16Be.GetBytes($configText) },
    @{ Name = 'utf32-le'; Bytes = $utf32Le.GetPreamble() + $utf32Le.GetBytes($configText) },
    @{ Name = 'utf32-be'; Bytes = $utf32Be.GetPreamble() + $utf32Be.GetBytes($configText) },
    @{ Name = 'gbk'; Bytes = $gbk.GetBytes($configText) }
)
foreach ($sample in $encodingSamples) {
    Write-FixtureBytes ('config/encodings/启动配置-{0}.ini' -f $sample.Name) ([byte[]]$sample.Bytes)
}
Write-FixtureText 'config/encodings/index.json' (ConvertTo-Json -InputObject ([pscustomobject]@{
    Source = '手写样本：逻辑内容相同，仅编码不同（对齐 LauncherConfig.ps1:6-43 的 BOM 顺序与 GBK 回退）'
    Text = $configText
    Files = @($encodingSamples | ForEach-Object { 'config/encodings/启动配置-' + $_.Name + '.ini' })
}) -Depth 4)

# ------------------------------------------------------------ 4. journal 快照
. (Join-Path $launcherRoot 'scripts\common\DeploymentJournal.ps1')
function Get-NormalizedPath { param([string]$Path) [IO.Path]::GetFullPath($Path).TrimEnd('\').ToLowerInvariant() }
function Get-HashOrNull { param([string]$Path) if ([IO.File]::Exists($Path)) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash } }
function Assert-GenCheck { param([bool]$Condition, [string]$Message) if (-not $Condition) { throw $Message } }

$temporaryRoot = Join-Path $env:TEMP ('dlssg-fixtures-' + [guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temporaryRoot)
$DryRun = $false
$script:deploymentJournal = $null

function New-Phase {
    param([string]$Name, [switch]$NoOriginal)
    $root = Join-Path $temporaryRoot $Name
    $render = Join-Path $root 'render'
    [void][IO.Directory]::CreateDirectory($render)
    $script:renderDir = $render
    $script:engineConfigPath = Join-Path $root 'Engine.ini'
    $script:engineRuntimePath = Join-Path $root 'Engine.runtime.ini'
    $script:engineBackupPath = Join-Path $root 'Engine.backup.ini'
    $script:storageDll = Join-Path $render 'storage.dll'
    $script:diagnosticIniPath = Join-Path $root 'dlssg_fixture-9080.ini'
    $script:linkSpecs = @(
        @{ Path = (Join-Path $render 'version.dll'); Target = (Join-Path $root 'source.dll') },
        @{ Path = (Join-Path $render 'dlssg_sm86.ini'); Target = $script:diagnosticIniPath }
    )
    $script:deployment = @{
        Root = (Join-Path $render 'dlssg_sm86'); Proxy = 'version.dll'
        Specs = @(@{ Path = (Join-Path $render 'dlssg_sm86\runtime'); Target = (Join-Path $root 'resource\runtime') })
    }
    [IO.File]::WriteAllText($script:linkSpecs[0].Target, 'proxy-payload')
    if (-not $NoOriginal) { [IO.File]::WriteAllText($script:engineConfigPath, 'fixture original engine.ini') }
    $script:journalPath = Join-Path $root 'state\active.json'
    $script:journalContext = @{
        GameId = 'fixture'; GameProcess = 'DLSSG_NoSuchGame'; LauncherProcess = 'DLSSG_NoSuchLauncher'
        Render = $render; Engine = $script:engineConfigPath; Runtime = $script:engineRuntimePath
        Backup = $script:engineBackupPath; DiagnosticPrefix = 'dlssg_fixture-'
    }
    $script:deploymentJournal = $null
    return $root
}

function Save-JournalFixture {
    param([string]$Name)
    $raw = [IO.File]::ReadAllText($script:journalPath)
    $journal = $raw | ConvertFrom-Json
    $normalized = $raw.Replace($temporaryRoot.Replace('\', '\\'), '<TEMPROOT>')
    $normalized = $normalized.Replace($journal.Session, '<SESSION>')
    $normalized = [regex]::Replace($normalized, '("ScriptPid":\s*)\d+', '${1}0')
    $normalized = [regex]::Replace($normalized, '("ScriptStarted":\s*)"[^"]*"', '${1}"<STARTED>"')
    Write-FixtureText ('journal/{0}.json' -f $Name) $normalized
    return $journal
}

try {
    # 阶段 1：新建记录（原 Engine.ini 存在）
    [void](New-Phase '01')
    New-DeploymentJournal
    [void](Save-JournalFixture '01-new')

    # 阶段 2：runtime 副本已写入
    $runtimeOp = Start-DeploymentOperation runtime $engineRuntimePath
    [IO.File]::WriteAllText($engineRuntimePath, 'fixture runtime copy')
    Complete-DeploymentOperation $runtimeOp
    [void](Save-JournalFixture '02-runtime-done')

    # 阶段 3：备份完成、engineLink 处于 intent（部署中断的窗口）
    $backupOp = Start-DeploymentOperation backup $engineConfigPath $engineBackupPath (Get-HashOrNull $engineConfigPath)
    Move-Item -LiteralPath $engineConfigPath -Destination $engineBackupPath
    Complete-DeploymentOperation $backupOp
    [void](Start-DeploymentOperation engineLink $engineConfigPath $engineRuntimePath)
    [void](Save-JournalFixture '03-engine-link-intent')

    # 阶段 4：部分失败（storage 只到 intent；哈希必须是合法 SHA256 形式）
    [void](Start-DeploymentOperation storage $storageDll '' (Get-JournalTextHash 'fixture-storage'))
    [void](Save-JournalFixture '04-partial-failure')

    # 阶段 5：完整恢复后记录被删除
    [void](Restore-RecordedDeployment)
    Assert-GenCheck (-not (Test-Path -LiteralPath $script:journalPath)) '恢复后 journal 应当被删除。'
    Write-FixtureText 'journal/05-recovered.txt' (@(
        '恢复完成后的不变量：.state/<gameId>/active.json 不存在。'
        '恢复顺序 engineLink → backup → runtime → storage → link → diagnostic → directory，同类逆序。'
        '本文件是断言说明，不是 JSON 样本。'
    ) -join "`r`n")

    # 阶段 6：Engine.ini 被游戏替换成普通文件 → 归档 recovered/<Session>-Engine.ini
    # 说明：真实软链接需要管理员权限；此样本把 engineLink 记录为 done 并放置普通文件，
    # 走的是 Restore-RecordedDeployment 的同一分支（Run-*.ps1 与 DeploymentJournal.ps1:438-450）。
    [void](New-Phase '06')
    New-DeploymentJournal
    $backupOp = Start-DeploymentOperation backup $engineConfigPath $engineBackupPath (Get-HashOrNull $engineConfigPath)
    Move-Item -LiteralPath $engineConfigPath -Destination $engineBackupPath
    Complete-DeploymentOperation $backupOp
    $linkOp = Start-DeploymentOperation engineLink $engineConfigPath $engineRuntimePath
    Complete-DeploymentOperation $linkOp
    [IO.File]::WriteAllText($engineConfigPath, 'fixture replaced by game')
    $session = $script:deploymentJournal.Session
    [void](Restore-RecordedDeployment)
    $recoveredPath = Join-Path (Split-Path -Parent $script:journalPath) ('recovered\' + $session + '-Engine.ini')
    Assert-GenCheck (Test-Path -LiteralPath $recoveredPath) '被替换的 Engine.ini 应归档到 recovered/<Session>-Engine.ini。'
    Write-FixtureText 'journal/06-recovered-archive.json' (ConvertTo-Json -InputObject ([pscustomobject]@{
        Pattern = 'recovered/<Session>-Engine.ini'
        SessionFormat = '^[a-fA-F0-9]{32}$'
        LinkSimulated = $true
        ArchivedContent = [IO.File]::ReadAllText($recoveredPath)
        RestoredContent = [IO.File]::ReadAllText($engineConfigPath)
        RestoredIsOriginalBytes = ([IO.File]::ReadAllText($engineConfigPath) -ceq 'fixture original engine.ini')
    }) -Depth 4)
    Write-FixtureText 'journal/06-recovered-archive.name.txt' ('recovered/<Session>-Engine.ini（本次实跑：recovered/' + $session + '-Engine.ini）')

    # 阶段 7：原件本来不存在（DlssgOnly / 首次部署）→ OriginalExisted=false
    [void](New-Phase '07' -NoOriginal)
    New-DeploymentJournal
    [void](Save-JournalFixture '07-no-original')

    # ------------------------------------------------ 5. 清单
    $sourceFiles = @(
        '启动脚本/scripts/common/LauncherConfig.ps1',
        '启动脚本/scripts/common/DeploymentJournal.ps1',
        '启动脚本/scripts/common/LauncherResources.ps1',
        '启动脚本/scripts/games/Run-WutheringWaves-DLSSG.ps1',
        '启动脚本/scripts/games/Run-NevernessToEverness-DLSSG.ps1',
        '启动脚本/scripts/games/Run-SteamUE5-DLSSG.ps1',
        'dlssg_sm86.ini'
    )
    $sources = foreach ($relative in $sourceFiles) {
        $path = Join-Path $repoRoot $relative
        [pscustomobject]@{
            Path = $relative
            Sha256 = (Get-Sha256Text ([IO.File]::ReadAllBytes($path)))
        }
    }
    $gitHead = try { (& git -C $repoRoot rev-parse HEAD 2>$null) } catch { $null }
    $gitBranch = try { (& git -C $repoRoot rev-parse --abbrev-ref HEAD 2>$null) } catch { $null }
    $manifest = [pscustomobject]@{
        FormatVersion = 1
        GeneratedAt = (Get-Date).ToUniversalTime().ToString('o')
        Generator = 'rust/tools/gen-fixtures.ps1'
        PowerShellVersion = $PSVersionTable.PSVersion.ToString()
        GitHead = $gitHead
        GitBranch = $gitBranch
        Sources = @($sources)
        Fixtures = @($written | Sort-Object Path)
    }
    Write-FixtureText 'manifest.json' (ConvertTo-Json -InputObject $manifest -Depth 8)

    Write-Host ('样本已生成：{0}' -f $FixtureRoot)
    Write-Host ('文件数：{0}' -f $written.Count)
}
finally {
    if ($KeepTemporary) {
        Write-Host ('临时目录保留：{0}' -f $temporaryRoot)
    }
    elseif (Test-Path -LiteralPath $temporaryRoot) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
