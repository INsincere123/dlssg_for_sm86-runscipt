param([switch]$SmokeTest, [string]$PreviewPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
try {
    . (Join-Path $PSScriptRoot '..\common\LauncherConfig.ps1')
    . (Join-Path $PSScriptRoot '..\common\SteamLauncher.ps1')
    . (Join-Path $PSScriptRoot '..\common\PortableLauncher.ps1')
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    [Windows.Forms.Application]::EnableVisualStyles()
    $root = Get-LauncherRoot
    $script:preferences = Read-PortablePreferences
    $script:worker = $null
    $script:model = $null
    $script:busy = $false
    $script:updating = $false
    $script:lastStatus = ''
    $script:customSourceSelected = $false
    $script:engineSaveFailed = @{}
    $script:scanTask = $null
    $script:discovery=$null
    $script:cancellation=$null
    $script:issues=@()
    $script:pendingRecord=$null
    $script:pendingProfile=$null
    $script:repairConfig=$false
    $script:qualityIds = @('Current','Tier1','Tier2','Tier3','Tier4','Tier5')

    $form = New-Object Windows.Forms.Form
    $form.Text = '虚幻游戏启动器'
    $form.Size = New-Object Drawing.Size(790,555)
    $form.MinimumSize = $form.Size
    $form.MaximumSize = New-Object Drawing.Size(790,780)
    $form.StartPosition = 'CenterScreen'
    $form.Font = New-Object Drawing.Font('Microsoft YaHei UI',10)
    $form.AutoScaleMode = 'Font'
    $form.BackColor = [Drawing.Color]::FromArgb(247,248,250)
    $form.FormBorderStyle = 'FixedDialog'
    $form.MaximizeBox = $false

    function New-Label {
        param($Parent,[string]$Text,[int]$X,[int]$Y,[int]$Width,[int]$Height=28)
        $control = New-Object Windows.Forms.Label
        $control.Text=$Text; $control.SetBounds($X,$Y,$Width,$Height)
        $Parent.Controls.Add($control)
        return $control
    }
    function New-Button {
        param($Parent,[string]$Text,[int]$X,[int]$Y,[int]$Width=130)
        $control = New-Object Windows.Forms.Button
        $control.Text=$Text; $control.SetBounds($X,$Y,$Width,38)
        $Parent.Controls.Add($control)
        return $control
    }
    function Show-Failure {
        param([string]$Message)
        [void][Windows.Forms.MessageBox]::Show($form,$Message,'需要处理','OK','Warning')
    }
    function Select-Folder {
        param([string]$Description)
        $dialog = New-Object Windows.Forms.FolderBrowserDialog
        $dialog.Description=$Description; $dialog.ShowNewFolderButton=$false
        try { if ($dialog.ShowDialog($form) -eq 'OK') { return $dialog.SelectedPath } }
        finally { $dialog.Dispose() }
        return ''
    }
    function Select-File {
        param([string]$Title,[string]$Filter,[string]$InitialDirectory='')
        $dialog = New-Object Windows.Forms.OpenFileDialog
        $dialog.Title=$Title; $dialog.Filter=$Filter; $dialog.CheckFileExists=$true
        if ($InitialDirectory -and [IO.Directory]::Exists($InitialDirectory)) { $dialog.InitialDirectory=$InitialDirectory }
        try { if ($dialog.ShowDialog($form) -eq 'OK') { return $dialog.FileName } }
        finally { $dialog.Dispose() }
        return ''
    }
    $title = New-Label $form '选好游戏，直接开始。' 26 22 700 42
    $title.Font = New-Object Drawing.Font('Microsoft YaHei UI',21,[Drawing.FontStyle]::Bold)
    $subtitle = New-Label $form '支持 Steam启动和EXE启动；退出后恢复原文件。' 28 73 700
    $subtitle.ForeColor = [Drawing.Color]::DimGray
    [void](New-Label $form '游戏' 28 119 40)
    $games = New-Object Windows.Forms.ComboBox
    $games.SetBounds(76,115,240,34); $games.DropDownStyle='DropDownList'; $games.DisplayMember='Name'
    $form.Controls.Add($games)
    [void](New-Label $form '画质' 328 119 44)
    $qualityBox=New-Object Windows.Forms.ComboBox
    $qualityBox.SetBounds(376,115,200,34); $qualityBox.DropDownStyle='DropDownList'
    foreach ($item in @('当前配置','高性能均衡','主流均衡','性能优先','入门流畅','05-自定义')) { [void]$qualityBox.Items.Add($item) }
    $form.Controls.Add($qualityBox)
    $add = New-Button $form '添加其他游戏' 588 111 132
    $script:status = New-Label $form '正在查找…' 28 171 690 56
    $script:status.Font = New-Object Drawing.Font('Microsoft YaHei UI',12,[Drawing.FontStyle]::Bold)
    $script:details = New-Object Windows.Forms.TextBox
    $script:details.SetBounds(28,234,690,74); $script:details.Multiline=$true; $script:details.ReadOnly=$true
    $script:details.BorderStyle='None'; $script:details.BackColor=$form.BackColor
    $script:details.Font = New-Object Drawing.Font('Microsoft YaHei UI',9)
    $form.Controls.Add($script:details)
    $launch = New-Button $form '启动游戏' 28 327 210
    $launch.BackColor=[Drawing.Color]::FromArgb(35,100,210); $launch.ForeColor=[Drawing.Color]::White
    $launch.FlatStyle='Flat'
    $restore = New-Button $form '恢复原状' 254 327 150
    $check = New-Button $form '检查环境' 420 327 140
    $refresh = New-Button $form '重新查找' 576 327 144
    $choose = New-Button $form '重选当前游戏目录' 28 383 210
    $advancedToggle = New-Button $form '高级设置' 254 383 150
    $logs = New-Button $form '查看日志' 420 383 140
    $cancelScan = New-Button $form '取消扫描' 576 383 144
    $cancelScan.Enabled=$false
    $script:note = New-Label $form '启动后本界面自动退出以释放内存；请保留负责恢复的独立脚本窗口。' 28 450 695 50
    $script:note.ForeColor = [Drawing.Color]::DimGray

    $advanced = New-Object Windows.Forms.GroupBox
    $advanced.Text='高级设置（通常无需修改）'; $advanced.SetBounds(28,447,692,225); $advanced.Visible=$false
    $form.Controls.Add($advanced)
    [void](New-Label $advanced 'Steam' 12 30 65)
    $steamBox = New-Object Windows.Forms.TextBox
    $steamBox.SetBounds(88,27,478,28); $advanced.Controls.Add($steamBox)
    $browseSteam = New-Button $advanced '选择' 576 23 100
    [void](New-Label $advanced '代理 DLL' 12 76 75)
    $proxyBox = New-Object Windows.Forms.ComboBox
    $proxyBox.SetBounds(88,72,260,30); $proxyBox.DropDownStyle='DropDownList'
    foreach ($item in @('1 · version.dll（默认）','2 · winmm.dll','3 · dbghelp.dll','4 · dinput8.dll','5 · dxgi.dll','6 · d3d12.dll')) { [void]$proxyBox.Items.Add($item) }
    $advanced.Controls.Add($proxyBox)
    [void](New-Label $advanced '画质配置' 12 122 75)
    $sourceBox = New-Object Windows.Forms.TextBox
    $sourceBox.SetBounds(88,118,478,28); $advanced.Controls.Add($sourceBox)
    $browseSource = New-Button $advanced '选择' 576 114 100
    [void](New-Label $advanced '引擎版本' 370 76 76)
    $engineBox=New-Object Windows.Forms.ComboBox
    $engineBox.SetBounds(450,72,226,30); $engineBox.DropDownStyle='DropDownList'
    foreach ($item in @('自动识别','UE4','UE5')) { [void]$engineBox.Items.Add($item) }
    $advanced.Controls.Add($engineBox)
    $save = New-Button $advanced '保存本机设置' 88 167 185
    [void](New-Label $advanced '选择新的INI并保存后使用“当前配置”。' 289 175 385)

    function Refresh-Game {
        param([switch]$PreserveEditors)
        if ($script:busy -or $null -eq $games.SelectedItem) { return }
        $selectedSteam=if ($null -ne $games.SelectedItem.PSObject.Properties['LaunchMode'] -and $games.SelectedItem.LaunchMode -eq 'Direct') { '' } else { Get-PortableSteamPath $script:preferences }
        $script:model = Get-PortableGameModel -Profile $games.SelectedItem -SteamPath $selectedSteam
        $script:scanTask=$null
        if (-not $script:model.Recovery -and $script:model.Executable -and $games.SelectedItem.EngineOverride -eq 'Auto' -and $null -eq (Get-PortableSavedEngine $games.SelectedItem)) {
            $task=[DlssgUnrealDetector]::Start($script:model.GameDirectory,$script:model.Executable)
            if ($task.IsCompleted) {
                $detection=if ($task.IsFaulted -or $task.IsCanceled) { [pscustomobject]@{Major=0;Source='识别失败，请手动校准'} } else { $task.Result }
                if ($detection.Major -in @(4,5) -and -not $script:engineSaveFailed.ContainsKey($games.SelectedItem.Id)) {
                    try { Save-PortableEngineDetection $script:preferences $games.SelectedItem $detection }
                    catch {
                        $script:engineSaveFailed[$games.SelectedItem.Id]=$true
                        Show-Failure ('本次引擎已识别，但保存失败，下次可能需要重新识别：'+$_.Exception.Message)
                    }
                }
                $script:model=Get-PortableGameModel -Profile $games.SelectedItem -SteamPath $selectedSteam -EngineDetection $detection
            } else {
                $script:scanTask=$task
                $script:model.EngineSource='正在后台识别…'
            }
        }
        $script:status.Text = $script:model.Status
        $script:status.ForeColor = if ($script:model.Ready) { [Drawing.Color]::FromArgb(25,125,75) } else { [Drawing.Color]::FromArgb(155,100,25) }
        $script:details.Text = "游戏位置：$($script:model.GameDirectory)`r`n配置位置：$($script:model.ConfigDirectory)`r`n$(if ($null -ne $games.SelectedItem.PSObject.Properties['LaunchMode'] -and $games.SelectedItem.LaunchMode -eq 'Direct') { 'EXE' } else { 'Steam' }) · 引擎：$(if ($script:model.EngineMajor) { "UE$($script:model.EngineMajor)" } else { "未知" })（$($script:model.EngineSource)） · 代理：$($script:model.Profile.ProxyDll)"
        $launch.Enabled=$script:model.Ready
        $restore.Enabled=$script:model.Recovery
        $check.Enabled=$script:model.Ready
        $choose.Enabled=-not $script:model.Recovery
        $advancedToggle.Enabled=-not $script:model.Recovery
        if (-not $PreserveEditors) {
            $steamBox.Text=$script:model.SteamPath
            $script:customSourceSelected=$false
            $sourceBox.Text=Resolve-LauncherConfigPath $games.SelectedItem.EngineConfigSource '配置源' -Root $root
            $proxyBox.SelectedIndex=[int]$script:model.Profile.ProxyDll-1
            $script:updating=$true
            try {
                $qualityBox.SelectedIndex=[Array]::IndexOf($script:qualityIds,[string]$games.SelectedItem.QualityPreset)
                $engineBox.SelectedIndex=[Array]::IndexOf(@('Auto','UE4','UE5'),[string]$games.SelectedItem.EngineOverride)
            } finally { $script:updating=$false }
        }
        $qualityBox.Enabled=-not $script:model.Recovery
        $engineBox.Enabled=-not $script:model.Recovery
        $isDirect=$null -ne $games.SelectedItem.PSObject.Properties['LaunchMode'] -and $games.SelectedItem.LaunchMode -eq 'Direct'
        $steamBox.Enabled=-not $isDirect -and -not $script:model.Recovery
        $browseSteam.Enabled=$steamBox.Enabled
    }
    function Load-Games {
        param([string]$SelectedId='')
        $script:updating=$true
        try {
            $games.Items.Clear()
            foreach ($profile in @(Get-PortableProfiles $script:preferences)) { [void]$games.Items.Add($profile) }
            $games.SelectedIndex=0
            foreach ($profile in $games.Items) { if ($profile.Id -eq $SelectedId) { $games.SelectedItem=$profile; break } }
        } finally { $script:updating=$false }
        Refresh-Game
    }
    function Set-Working {
        param([bool]$Working)
        $script:busy=$Working
        foreach ($control in @($qualityBox,$engineBox,$games,$add,$launch,$restore,$check,$refresh,$choose,$advancedToggle,$save,$browseSteam,$browseSource,$proxyBox,$steamBox,$sourceBox)) { $control.Enabled=-not $Working }
        if (-not $Working) { Refresh-Game }
    }
    function Start-Action {
        param([string]$Action)
        try {
            Refresh-Game
            if ($Action -eq 'Restore' -and -not $script:model.Recovery) { return }
            if ($Action -ne 'Restore' -and -not $script:model.Ready) { Show-Failure $script:model.Status; return }
            if ($Action -ne 'Restore') {
                $script:model.Profile.GameDirectory=$script:model.GameDirectory
                Set-PortableProfile $script:preferences $script:model.Profile
                Save-PortablePreferences $script:preferences
            }
            $requestDirectory = Join-Path (Get-PortableDataDirectory) ('runs\'+[guid]::NewGuid().ToString('N'))
            $config = Write-PortableLaunchConfig $script:model $requestDirectory
            $script:resultPath = Join-Path $requestDirectory 'result.json'
            $workerArguments=@{ScriptPath=(Join-Path $root 'scripts\games\Run-SteamUE5-DLSSG.ps1');ConfigPath=$config;AppId=$script:model.Profile.AppId;ResultPath=$script:resultPath;Action=$Action}
            if ($null -ne $script:model.Profile.PSObject.Properties['LaunchMode'] -and $script:model.Profile.LaunchMode -eq 'Direct') {
                $workerArguments.LaunchMode='Direct';$workerArguments.GameKey=$script:model.Profile.Id.Substring(7)
            }
            $command=New-PortableWorkerCommand @workerArguments
            $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
            $arguments = @('-NoLogo','-NoProfile','-ExecutionPolicy','Bypass','-EncodedCommand',$encoded)
            $parameters = @{ FilePath=(Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'); ArgumentList=$arguments; PassThru=$true }
            if ($Action -eq 'Check') { $parameters.WindowStyle='Hidden' } else { $parameters.Verb='RunAs'; $parameters.WindowStyle='Normal' }
            $script:worker = Start-Process @parameters
            $script:action = $Action
            if ($Action -eq 'Launch') {
                # The elevated worker owns the entire game/recovery lifetime.
                # Disposing its local Process handle does not terminate it.
                $form.Hide()
                $script:worker.Dispose()
                $script:worker=$null
                $form.Close()
                return
            }
            Set-Working $true
            $script:status.Text = switch ($Action) { 'Restore' { '正在恢复原状，请稍候…' } 'Check' { '正在检查环境，不会启动游戏…' } default { '游戏启动与自动恢复由独立窗口负责，请保留该窗口。' } }
        } catch { Show-Failure $_.Exception.Message }
    }

    $qualityBox.Add_SelectedIndexChanged({
        if ($script:updating -or $script:busy -or $null -eq $games.SelectedItem) { return }
        try {
            if ($script:model.Recovery) { Refresh-Game; return }
            $profile=$games.SelectedItem | ConvertTo-Json -Depth 6 | ConvertFrom-Json
            $profile.QualityPreset=$script:qualityIds[$qualityBox.SelectedIndex]
            Save-PortableProfileChange $script:preferences $profile
            Load-Games $profile.Id
        } catch { Refresh-Game; Show-Failure $_.Exception.Message }
    })
    $games.Add_SelectedIndexChanged({ if (-not $script:updating) { Refresh-Game } })
    $refresh.Add_Click({
        try {
            Refresh-Game
            if ($null -eq $script:model -or $script:model.Recovery -or -not $script:model.Executable) { return }
            if ([IO.Directory]::Exists($script:model.ConfigDirectory)) { return }
            $script:issues=@(); $script:repairConfig=$true
            $script:pendingProfile=$games.SelectedItem | ConvertTo-Json -Depth 6 | ConvertFrom-Json
            $script:pendingProfile.GameDirectory=$script:model.GameDirectory
            Start-Discovery 'Config'
        } catch { Show-Failure $_.Exception.Message }
    })
    $launch.Add_Click({ Start-Action Launch })
    $restore.Add_Click({ Start-Action Restore })
    $check.Add_Click({ Start-Action Check })
    $choose.Add_Click({
        try {
            $directory=Select-Folder '选择当前游戏的安装文件夹（例如 BlackMythWukong）'
            if (-not $directory) { return }
            if ($games.SelectedItem.LaunchMode -eq 'Steam') {
                $id=Resolve-SteamAppId $directory (Get-PortableSteamPath $script:preferences)
                if ($id.AppId -ne $games.SelectedItem.AppId) { throw '这个文件夹不是当前选择的游戏。' }
            } else {
                foreach ($relative in @($games.SelectedItem.RenderExecutable,$games.SelectedItem.LaunchExecutable)) {
                    $path=Resolve-DirectExecutable $directory $relative
                    if (-not [IO.File]::Exists($path)) { throw '所选目录不包含当前游戏原有的启动/渲染EXE相对路径。' }
                }
            }
            $games.SelectedItem.GameDirectory=$directory
            Set-PortableProfile $script:preferences $games.SelectedItem
            Save-PortablePreferences $script:preferences
            Refresh-Game
        } catch { Show-Failure $_.Exception.Message }
    })
    $advancedToggle.Add_Click({
        $advanced.Visible=-not $advanced.Visible
        $form.Height=if ($advanced.Visible) { 765 } else { 555 }
        $script:note.Top=if ($advanced.Visible) { 688 } else { 450 }
    })
    $browseSteam.Add_Click({ $file=Select-File '选择 Steam 程序' 'Steam (steam.exe)|steam.exe'; if ($file) { $steamBox.Text=$file } })
    $browseSource.Add_Click({ $file=Select-File '选择明文 Engine.ini 配置源' 'INI 配置 (*.ini)|*.ini'; if ($file) { $sourceBox.Text=$file; $script:customSourceSelected=$true } })
    $save.Add_Click({
        try {
            if ($games.SelectedItem.LaunchMode -eq 'Steam' -and -not [IO.File]::Exists($steamBox.Text)) { throw '请选择有效的 steam.exe。' }
            if (-not [IO.File]::Exists($sourceBox.Text)) { throw '请选择有效的明文 INI 配置。' }
            if ($script:model.Recovery) { throw '请先恢复上次部署。' }
            $profile=$games.SelectedItem | ConvertTo-Json -Depth 6 | ConvertFrom-Json
            $profile.ProxyDll=$proxyBox.SelectedIndex+1
            $profile.EngineOverride=@('Auto','UE4','UE5')[$engineBox.SelectedIndex]
            $oldSource=Resolve-LauncherConfigPath $profile.EngineConfigSource '配置源' -Root $root
            if ($script:customSourceSelected -or $sourceBox.Text -ne $oldSource) { $profile.QualityPreset='Current' }
            $profile.EngineConfigSource=$sourceBox.Text
            $oldSteam=$script:preferences.SteamPath
            try {
                if ($profile.LaunchMode -eq 'Steam') { $script:preferences.SteamPath=$steamBox.Text }
                Save-PortableProfileChange $script:preferences $profile
            } catch { $script:preferences.SteamPath=$oldSteam; throw }
            Load-Games $profile.Id
        } catch { Show-Failure $_.Exception.Message }
    })
    $logs.Add_Click({
        try {
            $directory=Join-Path $root 'logs'
            if (-not [IO.Directory]::Exists($directory)) { Show-Failure '还没有运行日志。'; return }
            Start-Process explorer.exe -ArgumentList ('"{0}"' -f $directory)
        } catch { Show-Failure $_.Exception.Message }
    })
    function Select-DiscoveryRecord($Rows,[string]$Title) {
        $dialog=[Windows.Forms.Form]::new(); $dialog.Text=$Title; $dialog.ClientSize=[Drawing.Size]::new(850,420)
        $dialog.StartPosition='CenterParent'; $dialog.Font=$form.Font
        $list=[Windows.Forms.ListBox]::new(); $list.SetBounds(15,15,820,335); $list.DisplayMember='Display'; $list.HorizontalScrollbar=$true
        foreach ($row in @($Rows)) { [void]$list.Items.Add($row) }
        if ($list.Items.Count) { $list.SelectedIndex=0 }
        $dialog.Controls.Add($list)
        $ok=New-Button $dialog '选择' 585 367 120; $ok.DialogResult='OK'; $dialog.AcceptButton=$ok
        $back=New-Button $dialog '取消' 715 367 120; $back.DialogResult='Cancel'; $dialog.CancelButton=$back
        try { if ($dialog.ShowDialog($form) -eq 'OK') { return $list.SelectedItem } }
        finally { $dialog.Dispose() }
        return $null
    }
    function Start-Discovery([string]$Mode) {
        $script:cancellation=[Threading.CancellationTokenSource]::new()
        $script:discovery=[powershell]::Create()
        [void]$script:discovery.AddScript({param($Common,$Mode,$Steam,$Directory,$Declared,$Render,$Cancellation)
            $ErrorActionPreference='Stop'; Set-StrictMode -Version Latest
            . (Join-Path $Common 'LauncherConfig.ps1'); . (Join-Path $Common 'SteamLauncher.ps1')
            . (Join-Path $Common 'DirectLauncher.ps1'); . (Join-Path $Common 'GameDiscovery.ps1')
            $issues=[Collections.Generic.List[string]]::new()
            $rows=switch ($Mode) {
                'Installed' { @(Get-InstalledGameRecords -SteamExecutable $Steam -Cancellation $Cancellation -Issues $issues) }
                'Renderers' { @(Get-GameRendererCandidates $Directory $Declared -Cancellation $Cancellation -Issues $issues) }
                'Config' { @(Get-UnrealConfigDirectoryCandidates $Directory $Render -Cancellation $Cancellation -Issues $issues) }
            }
            [pscustomobject]@{Rows=@($rows);Issues=@($issues.ToArray());Cancelled=$Cancellation.IsCancellationRequested}
        })
        $directory=''; $declared=''; $render=''
        if ($Mode -eq 'Renderers') { $directory=$script:pendingRecord.GameDirectory; $declared=$script:pendingRecord.DeclaredExecutable }
        if ($Mode -eq 'Config') { $directory=$script:pendingProfile.GameDirectory; $render=Join-Path $directory $script:pendingProfile.RenderExecutable }
        $steam=''
        if ($Mode -eq 'Installed') { $steam=if ([IO.File]::Exists($steamBox.Text)) { $steamBox.Text } else { Get-PortableSteamPath $script:preferences } }
        [void]$script:discovery.AddArgument((Join-Path $root 'scripts\common')).AddArgument($Mode).AddArgument($steam).AddArgument($directory).AddArgument($declared).AddArgument($render).AddArgument($script:cancellation)
        $script:discoveryMode=$Mode
        $script:discoveryHandle=$script:discovery.BeginInvoke()
        Set-Working $true; $cancelScan.Enabled=$true
        $script:status.Text='正在扫描，可取消；不会启动或部署游戏…'
    }
    function Complete-Discovery([string]$Mode,$Rows) {
        if ($Mode -eq 'Installed') {
            if (-not @($Rows).Count) { $script:status.Text='未找到安装记录，可手动选择目录；跳过原因见扫描详情。'; return }
            $record=Select-DiscoveryRecord $Rows '选择要添加的已安装游戏'
            if ($null -eq $record) { return }
            $script:pendingRecord=$record; Start-Discovery 'Renderers'; return
        }
        if ($Mode -eq 'Renderers') {
            $record=$script:pendingRecord
            $candidate=Get-TrustedGameRenderer -Candidates @($Rows)
            if ($null -eq $candidate -and @($Rows).Count) { $candidate=Select-DiscoveryRecord $Rows '选择真实渲染程序（兼容性需确认）'; if ($null -eq $candidate) { return } }
            $render=if ($null -ne $candidate) { $candidate.Path } else { Select-File '未找到候选，请选择x64渲染程序' '游戏程序 (*.exe)|*.exe' $record.GameDirectory }
            if (-not $render) { return }
            $profile=New-PortableDiscoveredProfile $record $render
            if (@($games.Items | Where-Object { ($_.LaunchMode -eq 'Steam' -and $profile.LaunchMode -eq 'Steam' -and $_.AppId -eq $profile.AppId) -or ($_.GameDirectory -and [IO.Path]::GetFullPath($_.GameDirectory).TrimEnd('\') -ieq $profile.GameDirectory) }).Count) { throw '该游戏已在列表中，请修改原条目并保留恢复身份。' }
            $script:pendingProfile=$profile
            Start-Discovery 'Config'; return
        }
        if (@($Rows).Count -eq 1) { $config=$Rows[0].Path }
        elseif (@($Rows).Count -gt 1) {
            $candidate=Select-DiscoveryRecord $Rows '找到多个配置目录，请选择游戏实际使用的目录'
            if ($null -eq $candidate) { return }; $config=$candidate.Path
        } else {
            $config=Select-Folder '未找到配置目录；请先原版启动一次，或选择包含Engine.ini / GameUserSettings.ini的文件夹'
            if (-not $config) { return }
        }
        # Recheck recovery immediately before saving; never redirect an active deployment.
        $profile=$script:pendingProfile
        if ($script:repairConfig) {
            $steam=if ($profile.LaunchMode -eq 'Steam') { Get-PortableSteamPath $script:preferences } else { '' }
            $model=Get-PortableGameModel $profile $steam
            if ($model.Recovery -or [IO.File]::Exists($model.Journal)) { throw '发现遗留部署，请先恢复原状。' }
        }
        $profile.ConfigDirectory=$config
        Save-PortableProfileChange $script:preferences $profile
        Load-Games $profile.Id
    }
    function Add-LauncherGame {
        param([ValidateSet('Steam','Direct')][string]$Mode)
        try {
            $directory=Select-Folder '选择UE4/UE5游戏的安装文件夹'
            if (-not $directory) { return }
            $appId=''
            if ($Mode -eq 'Steam') {
                $steam=Get-PortableSteamPath $script:preferences
                if (-not $steam) { throw '请先在高级设置中选择Steam。' }
                $appId=(Resolve-SteamAppId $directory $steam).AppId
            }
            $script:issues=@(); $script:repairConfig=$false
            $script:pendingRecord=[pscustomobject]@{Platform=$Mode;Name=(Split-Path -Leaf $directory);GameDirectory=$directory;AppId=$appId;DeclaredExecutable=''}
            Start-Discovery 'Renderers'
        } catch { Show-Failure $_.Exception.Message }
    }
    $addMenu=New-Object Windows.Forms.ContextMenuStrip
    $steamItem=$addMenu.Items.Add('添加 Steam 游戏')
    $directItem=$addMenu.Items.Add('添加EXE启动游戏')
    $discoverItem=$addMenu.Items.Add('扫描已安装游戏（Steam / Epic / GOG）')
    $detailsItem=$addMenu.Items.Add('扫描详情')
    $discoverItem.Add_Click({ try { $script:issues=@(); $script:repairConfig=$false; Start-Discovery 'Installed' } catch { Show-Failure $_.Exception.Message } })
    $detailsItem.Add_Click({ [void][Windows.Forms.MessageBox]::Show($form,$(if ($script:issues.Count) { $script:issues -join "`r`n" } else { '没有扫描跳过记录。' }),'扫描详情') })
    $cancelScan.Add_Click({ if ($null -ne $script:cancellation) { $script:cancellation.Cancel(); $cancelScan.Enabled=$false } })
    $steamItem.Add_Click({ Add-LauncherGame Steam })
    $directItem.Add_Click({ Add-LauncherGame Direct })
    $add.Add_Click({ $addMenu.Show($add,0,$add.Height) })
    $timer = New-Object Windows.Forms.Timer
    $timer.Interval=250
    $timer.Add_Tick({
        if ($null -ne $script:discovery) {
            if (-not $script:discoveryHandle.IsCompleted) { return }
            try {
                $results=@($script:discovery.EndInvoke($script:discoveryHandle))
                if ($script:discovery.HadErrors) { throw ($script:discovery.Streams.Error | Out-String) }
                $result=$results[0]; $mode=$script:discoveryMode
                $cancelled=$result.Cancelled -or $script:cancellation.IsCancellationRequested
                $script:issues+=@($result.Issues)
                $script:discovery.Dispose(); $script:discovery=$null
                $script:cancellation.Dispose(); $script:cancellation=$null
                $cancelScan.Enabled=$false; Set-Working $false
                if ($cancelled) { $script:pendingProfile=$null; $script:status.Text='扫描已取消；未保存游戏设置。'; return }
                Complete-Discovery $mode @($result.Rows)
            } catch {
                if ($null -ne $script:discovery) { $script:discovery.Dispose(); $script:discovery=$null }
                if ($null -ne $script:cancellation) { $script:cancellation.Dispose(); $script:cancellation=$null }
                $script:pendingProfile=$null; $cancelScan.Enabled=$false; Set-Working $false; Show-Failure $_.Exception.Message
            }
            return
        }
        if (-not $script:busy) {
            if ($null -ne $script:scanTask -and $script:scanTask.IsCompleted) { Refresh-Game -PreserveEditors }
            return
        }
        try {
            if (-not $script:worker.HasExited) { return }
            $script:worker.Dispose(); $script:worker=$null
            if (-not [IO.File]::Exists($script:resultPath)) { throw '启动窗口意外结束。请点击“恢复原状”检查遗留部署。' }
            $result=[IO.File]::ReadAllText($script:resultPath) | ConvertFrom-Json
            Set-Working $false
            if ($result.ExitCode -ne 0) { Show-Failure ('操作未完成，文件和恢复记录会保留。可点击“查看日志”获取详情。'+"`r`n"+$result.Error) }
            elseif ($script:action -eq 'Check') { $script:status.Text='环境检查通过，可以启动。' }
            elseif ($script:action -eq 'Restore') { $script:status.Text='恢复完成。' }
            else { $script:status.Text='本次运行已结束，自动恢复完成。' }
        } catch { Set-Working $false; Show-Failure $_.Exception.Message }
    })
    $form.Add_FormClosing({ param($sender,$eventArgs)
        if ($null -ne $script:discovery) { $script:cancellation.Cancel(); $eventArgs.Cancel=$true; $script:status.Text='正在取消扫描，完成后可关闭界面。'; return }
        if ($script:busy) { $eventArgs.Cancel=$true; Show-Failure '请等待游戏退出和恢复完成，再关闭启动器。' }
    })
    Load-Games
    if ($SmokeTest) {
        foreach ($pair in @(@($games,$qualityBox),@($qualityBox,$add),@($proxyBox,$engineBox))) {
            if ($pair[0].Bounds.IntersectsWith($pair[1].Bounds)) { throw '界面控件重叠。' }
        }
        if ($engineBox.Right -gt $advanced.ClientSize.Width -or $add.Right -gt $form.ClientSize.Width) { throw '界面控件超出边界。' }
        # Materialize child handles for DrawToBitmap without displaying a window.
        $form.ShowInTaskbar=$false; $form.Opacity=0
        $form.Show(); [Windows.Forms.Application]::DoEvents()
        if ($PreviewPath) {
            [void][IO.Directory]::CreateDirectory((Split-Path -Parent $PreviewPath))
            foreach ($expanded in @($false,$true)) {
                $advanced.Visible=$expanded
                $form.Height=if ($expanded) { 765 } else { 555 }
                $script:note.Top=if ($expanded) { 688 } else { 450 }
                [Windows.Forms.Application]::DoEvents()
                $outputPath=if ($expanded) { [IO.Path]::Combine([IO.Path]::GetDirectoryName($PreviewPath),[IO.Path]::GetFileNameWithoutExtension($PreviewPath)+'.advanced.png') } else { $PreviewPath }
                $bitmap=New-Object Drawing.Bitmap($form.Width,$form.Height)
                try { $form.DrawToBitmap($bitmap,(New-Object Drawing.Rectangle(0,0,$form.Width,$form.Height))); $bitmap.Save($outputPath,[Drawing.Imaging.ImageFormat]::Png) }
                finally { $bitmap.Dispose() }
            }
        }
        Write-Output ("GUI smoke test passed: {0}; {1}" -f $games.SelectedItem.Name,$script:model.Status)
    } else { $timer.Start(); [void]$form.ShowDialog() }
    $timer.Dispose(); $addMenu.Dispose(); $form.Dispose()
} catch {
    if ($SmokeTest) { throw }
    Add-Type -AssemblyName System.Windows.Forms
    [void][Windows.Forms.MessageBox]::Show($_.Exception.Message,'启动器无法打开','OK','Error')
    exit 1
}
