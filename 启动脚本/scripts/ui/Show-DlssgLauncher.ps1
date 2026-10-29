param([switch]$SmokeTest,[string]$PreviewPath)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
try {
    . (Join-Path $PSScriptRoot '..\common\LauncherConfig.ps1')
    . (Join-Path $PSScriptRoot '..\common\SteamLauncher.ps1')
    . (Join-Path $PSScriptRoot '..\common\DlssgOnlyLauncher.ps1')
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    [Windows.Forms.Application]::EnableVisualStyles()
    $root=Get-LauncherRoot
    $dataDirectory=Get-DlssgOnlyDataDirectory
    $script:preferences=Read-PortablePreferences -Directory $dataDirectory
    $script:worker=$null; $script:scan=$null; $script:scanHandle=$null; $script:cancellation=$null
    $script:busy=$false; $script:updating=$false; $script:model=$null; $script:pendingRecord=$null
    $script:issues=@(); $script:resultPath=''
    $form=[Windows.Forms.Form]::new()
    $form.Text='DLSSG通用启动器'; $form.ClientSize=[Drawing.Size]::new(900,650)
    $form.Font=[Drawing.Font]::new('Microsoft YaHei UI',10)
    $form.StartPosition='CenterScreen'; $form.FormBorderStyle='FixedDialog'; $form.MaximizeBox=$false
    function Add-Label($Parent,$Text,$X,$Y,$Width=120,$Height=27) {
        $c=[Windows.Forms.Label]::new(); $c.Text=$Text; $c.SetBounds($X,$Y,$Width,$Height); $Parent.Controls.Add($c); return $c
    }
    function Add-Button($Parent,$Text,$X,$Y,$Width=130) {
        $c=[Windows.Forms.Button]::new(); $c.Text=$Text; $c.SetBounds($X,$Y,$Width,35); $Parent.Controls.Add($c); return $c
    }
    function Add-Box($Parent,$X,$Y,$Width=400) {
        $c=[Windows.Forms.TextBox]::new(); $c.SetBounds($X,$Y,$Width,29); $Parent.Controls.Add($c); return $c
    }
    function Show-Failure([string]$Message) { [void][Windows.Forms.MessageBox]::Show($form,$Message,'需要处理','OK','Warning') }
    function Select-Exe([string]$Title,[string]$Directory='') {
        $dialog=[Windows.Forms.OpenFileDialog]::new(); $dialog.Title=$Title; $dialog.Filter='程序 (*.exe)|*.exe'
        if ([IO.Directory]::Exists($Directory)) { $dialog.InitialDirectory=$Directory }
        try { if ($dialog.ShowDialog($form) -eq 'OK') { return $dialog.FileName } } finally { $dialog.Dispose() }
        return ''
    }
    function Select-Directory {
        $dialog=[Windows.Forms.FolderBrowserDialog]::new(); $dialog.Description='选择游戏安装目录'; $dialog.ShowNewFolderButton=$false
        try { if ($dialog.ShowDialog($form) -eq 'OK') { return $dialog.SelectedPath } } finally { $dialog.Dispose() }
        return ''
    }
    $title=Add-Label $form 'DLSSG通用启动器' 22 16 650 40
    $title.Font=[Drawing.Font]::new('Microsoft YaHei UI',21,[Drawing.FontStyle]::Bold)
    [void](Add-Label $form '保留游戏的 Engine.ini；启用帧生成及现有 INT8 超分功能。' 24 64 840)
    $discover=Add-Button $form '发现已安装游戏' 24 103 165
    $manual=Add-Button $form '手动添加游戏' 201 103 150
    $cancel=Add-Button $form '取消扫描' 363 103 110; $cancel.Enabled=$false
    $scanDetails=Add-Button $form '扫描详情' 485 103 110
    $games=[Windows.Forms.ListBox]::new(); $games.SetBounds(24,155,245,350); $games.DisplayMember='Name'; $form.Controls.Add($games)
    [void](Add-Label $form '游戏名称' 290 156 90); $nameBox=Add-Box $form 390 153 480
    [void](Add-Label $form '来源 / 方式' 290 198 100); $modeLabel=Add-Label $form '' 390 198 480
    [void](Add-Label $form '安装目录' 290 239 95); $directoryBox=Add-Box $form 390 235 380
    $browseDirectory=Add-Button $form '选择' 780 232 90
    [void](Add-Label $form '渲染程序' 290 280 95); $renderBox=Add-Box $form 390 277 380
    $browseRender=Add-Button $form '选择' 780 274 90
    [void](Add-Label $form '启动程序' 290 321 95); $launchBox=Add-Box $form 390 318 380
    $browseLaunch=Add-Button $form '选择' 780 315 90
    [void](Add-Label $form 'Steam程序' 290 362 95); $steamBox=Add-Box $form 390 359 380
    $steamBox.Text=Get-PortableSteamPath $script:preferences
    $browseSteam=Add-Button $form '选择' 780 356 90
    [void](Add-Label $form '代理 DLL' 290 403 95)
    $proxyBox=[Windows.Forms.ComboBox]::new(); $proxyBox.SetBounds(390,400,280,30); $proxyBox.DropDownStyle='DropDownList'
    foreach ($text in @('1 · version.dll（默认）','2 · winmm.dll','3 · dbghelp.dll','4 · dinput8.dll','5 · dxgi.dll','6 · d3d12.dll')) { [void]$proxyBox.Items.Add($text) }
    $form.Controls.Add($proxyBox)
    $save=Add-Button $form '保存设置' 690 441 180
    $status=Add-Label $form '尚未添加游戏，请发现或手动添加。' 290 482 580 62
    $launch=Add-Button $form '启动游戏' 24 551 180
    $restore=Add-Button $form '恢复原状' 216 551 160
    $check=Add-Button $form '检查环境' 388 551 150
    $logs=Add-Button $form '查看日志' 550 551 150
    [void](Add-Label $form '启动后界面退出；请保留负责等待游戏及恢复的独立脚本窗口。' 24 604 850)

    function Refresh-Game {
        $p=$games.SelectedItem
        $script:model=$null
        $launch.Enabled=$false; $restore.Enabled=$false; $check.Enabled=$false
        if ($null -eq $p) {
            $status.Text='尚未添加游戏，请发现或手动添加。'
            foreach ($c in @($nameBox,$directoryBox,$renderBox,$launchBox,$proxyBox,$save,$browseDirectory,$browseRender,$browseLaunch)) { $c.Enabled=$false }
            return
        }
        $script:updating=$true
        try {
            $nameBox.Text=$p.Name; $directoryBox.Text=$p.GameDirectory; $renderBox.Text=$p.RenderExecutable; $launchBox.Text=$p.LaunchExecutable
            $proxyBox.SelectedIndex=[int]$p.ProxyDll-1
            $mode=if ($p.LaunchMode -eq 'Steam') { 'Steam启动' } else { 'EXE启动' }
            $modeLabel.Text=$p.Platform+' / '+$mode
            $script:model=Get-DlssgOnlyModel $p $steamBox.Text
            $status.Text=$script:model.Status
            $locked=[IO.File]::Exists($script:model.Journal)
            foreach ($c in @($nameBox,$directoryBox,$renderBox,$launchBox,$proxyBox,$save,$browseDirectory,$browseRender,$browseLaunch)) { $c.Enabled=(-not $locked -and -not $script:busy) }
            $launchBox.Enabled=$launchBox.Enabled -and $p.LaunchMode -eq 'Direct'
            $browseLaunch.Enabled=$launchBox.Enabled
            $launch.Enabled=$script:model.Ready -and -not $script:busy
            $check.Enabled=$launch.Enabled; $restore.Enabled=$script:model.Recovery -and -not $script:busy
        } finally { $script:updating=$false }
    }
    function Reload-Games([string]$SelectedId='') {
        $script:updating=$true
        try {
            $games.Items.Clear()
            foreach ($p in @($script:preferences.Profiles)) { Initialize-PortableLaunchMode $p; [void]$games.Items.Add($p) }
            if ($games.Items.Count) { $games.SelectedIndex=0 }
            foreach ($p in $games.Items) { if ($p.Id -eq $SelectedId) { $games.SelectedItem=$p; break } }
        } finally { $script:updating=$false }
        Refresh-Game
    }
    function Set-Working([bool]$Working) {
        $script:busy=$Working
        foreach ($c in @($games,$discover,$manual,$save,$browseDirectory,$browseRender,$browseLaunch,$browseSteam,$steamBox,$nameBox,$directoryBox,$renderBox,$launchBox,$proxyBox,$launch,$restore,$check)) { $c.Enabled=-not $Working }
        if (-not $Working) { Refresh-Game }
    }
    function Save-Editors {
        $p=$games.SelectedItem
        if ($null -eq $p) { return }
        $id=Get-PortableGameId $p
        if ([IO.File]::Exists((Join-Path $root ".state\$id\active.json"))) { throw '部署尚未恢复，请先恢复再修改设置。' }
        if ([string]::IsNullOrWhiteSpace($nameBox.Text) -or $nameBox.Text -match '[\r\n]') { throw '请填写游戏名称。' }
        $directory=Resolve-LauncherConfigPath $directoryBox.Text '安装目录'
        [void](Resolve-DirectExecutable $directory $renderBox.Text '渲染程序')
        [void](Resolve-DirectExecutable $directory $launchBox.Text '启动程序')
        $copy=$p | ConvertTo-Json | ConvertFrom-Json
        $copy.Name=$nameBox.Text; $copy.GameDirectory=$directory; $copy.RenderExecutable=$renderBox.Text; $copy.LaunchExecutable=$launchBox.Text; $copy.ProxyDll=$proxyBox.SelectedIndex+1
        $model=Get-DlssgOnlyModel $copy $steamBox.Text
        if (-not $model.Ready) { throw $model.Status }
        $copy.GameDirectory=$model.GameDirectory
        $script:preferences.SteamPath=$steamBox.Text
        Set-PortableProfile $script:preferences $copy
        Save-PortablePreferences $script:preferences -Directory $dataDirectory
        Reload-Games $copy.Id
    }
    function Start-Action([string]$Action) {
        try {
            if ($null -eq $games.SelectedItem) { return }
            if ($Action -ne 'Restore') { Save-Editors }
            Refresh-Game
            if ($Action -eq 'Restore' -and -not $script:model.Recovery) { return }
            if ($Action -ne 'Restore' -and -not $script:model.Ready) { throw $script:model.Status }
            $requestDirectory=Join-Path $dataDirectory ('runs\'+[guid]::NewGuid().ToString('N'))
            $config=Write-DlssgOnlyLaunchConfig $script:model $requestDirectory
            $script:resultPath=Join-Path $requestDirectory 'result.json'
            $p=$script:model.Profile
            $args=@{ScriptPath=(Join-Path $root 'scripts\games\Run-SteamUE5-DLSSG.ps1');ConfigPath=$config;ResultPath=$script:resultPath;Action=$Action;LaunchMode=$p.LaunchMode;DlssgOnly=$true}
            if ($p.LaunchMode -eq 'Steam') { $args.AppId=$p.AppId } else { $args.GameKey=$p.Id.Substring(7) }
            $command=New-PortableWorkerCommand @args
            $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
            $parameters=@{FilePath=(Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe');ArgumentList=@('-NoLogo','-NoProfile','-ExecutionPolicy','Bypass','-EncodedCommand',$encoded);PassThru=$true}
            if ($Action -eq 'Check') { $parameters.WindowStyle='Hidden' } else { $parameters.Verb='RunAs'; $parameters.WindowStyle='Normal' }
            $script:worker=Start-Process @parameters
            if ($Action -eq 'Launch') { $form.Hide(); $script:worker.Dispose(); $script:worker=$null; $form.Close(); return }
            Set-Working $true; $status.Text='正在'+$(if ($Action -eq 'Check') { '检查环境' } else { '恢复原状' })+'，请稍候…'
        } catch { Show-Failure $_.Exception.Message }
    }
    function Select-Record($Rows,[string]$Title,[string]$Member='Display') {
        $dialog=[Windows.Forms.Form]::new(); $dialog.Text=$Title; $dialog.ClientSize=[Drawing.Size]::new(850,420)
        $dialog.StartPosition='CenterParent'; $dialog.Font=$form.Font
        $list=[Windows.Forms.ListBox]::new(); $list.SetBounds(15,15,820,335); $list.DisplayMember=$Member; $list.HorizontalScrollbar=$true
        foreach ($row in @($Rows)) { [void]$list.Items.Add($row) }; if ($list.Items.Count) { $list.SelectedIndex=0 }
        $dialog.Controls.Add($list)
        $ok=Add-Button $dialog '选择' 585 367 120; $ok.DialogResult='OK'; $dialog.AcceptButton=$ok
        $back=Add-Button $dialog '取消' 715 367 120; $back.DialogResult='Cancel'; $dialog.CancelButton=$back
        try { if ($dialog.ShowDialog($form) -eq 'OK' -and $null -ne $list.SelectedItem) { return $list.SelectedItem } } finally { $dialog.Dispose() }
        return $null
    }
    function Add-ScannedGame($Rows) {
        $record=$script:pendingRecord
        if (@($Rows).Count) { $candidate=Select-Record $Rows '选择真实渲染程序（按证据排序，兼容性待确认）'; if ($null -eq $candidate) { return }; $render=$candidate.Path }
        else { $render=Select-Exe '未找到候选，请手动选择游戏的x64渲染程序' $record.GameDirectory; if (-not $render) { return } }
        $base=[IO.Path]::GetFullPath($record.GameDirectory).TrimEnd('\')+'\'
        if (-not $render.StartsWith($base,[StringComparison]::OrdinalIgnoreCase)) { throw '渲染程序必须位于所选游戏目录。' }
        if (-not (Test-GameX64Executable $render)) { throw '请选择有效的x64渲染程序。' }
        $relative=$render.Substring($base.Length)
        $launchRelative=$relative
        if ($record.DeclaredExecutable -and [IO.File]::Exists($record.DeclaredExecutable)) { $launchRelative=$record.DeclaredExecutable.Substring($base.Length) }
        $profile=New-DlssgOnlyProfile $record $relative $launchRelative
        $existing=@($script:preferences.Profiles | Where-Object { [IO.Path]::GetFullPath($_.GameDirectory).TrimEnd('\') -ieq $base.TrimEnd('\') })
        if ($existing.Count) { throw '该安装目录已添加，请在列表中修改原游戏设置。' }
        $sameId=@($script:preferences.Profiles | Where-Object Id -eq $profile.Id)
        if ($sameId.Count) { throw '该Steam游戏已添加，请修改原游戏目录，保留恢复身份。' }
        Set-PortableProfile $script:preferences $profile
        $script:preferences.SteamPath=$steamBox.Text
        Save-PortablePreferences $script:preferences -Directory $dataDirectory
        Reload-Games $profile.Id
    }
    function Start-Scan([string]$Mode) {
        $script:cancellation=[Threading.CancellationTokenSource]::new()
        $script:scan=[powershell]::Create()
        [void]$script:scan.AddScript({param($Common,$Mode,$Steam,$Directory,$Declared,$Cancellation)
            $ErrorActionPreference='Stop'; Set-StrictMode -Version Latest
            . (Join-Path $Common 'LauncherConfig.ps1'); . (Join-Path $Common 'SteamLauncher.ps1')
            . (Join-Path $Common 'DirectLauncher.ps1'); . (Join-Path $Common 'GameDiscovery.ps1')
            $issues=[Collections.Generic.List[string]]::new()
            $rows=if ($Mode -eq 'Installed') { @(Get-InstalledGameRecords -SteamExecutable $Steam -Cancellation $Cancellation -Issues $issues) }
                else { @(Get-GameRendererCandidates -GameDirectory $Directory -DeclaredExecutable $Declared -Cancellation $Cancellation -Issues $issues) }
            [pscustomobject]@{Rows=@($rows);Issues=@($issues.ToArray());Cancelled=$Cancellation.IsCancellationRequested}
        })
        [void]$script:scan.AddArgument((Join-Path $root 'scripts\common')).AddArgument($Mode).AddArgument($steamBox.Text)
        $directory=''; $declared=''
        if ($Mode -eq 'Renderers') { $directory=$script:pendingRecord.GameDirectory; $declared=$script:pendingRecord.DeclaredExecutable }
        [void]$script:scan.AddArgument($directory).AddArgument($declared).AddArgument($script:cancellation)
        $script:scanMode=$Mode
        $script:scanHandle=$script:scan.BeginInvoke()
        Set-Working $true; $cancel.Enabled=$true; $status.Text='正在扫描，可取消；不会启动或部署游戏…'
    }
    $timer=[Windows.Forms.Timer]::new(); $timer.Interval=180
    $timer.Add_Tick({
        if ($null -ne $script:scan -and $script:scanHandle.IsCompleted) {
            try {
                $result=@($script:scan.EndInvoke($script:scanHandle))
                if ($script:scan.HadErrors) { throw ($script:scan.Streams.Error | Out-String) }
                if ($script:scanMode -eq 'Installed') { $script:issues=@($result[0].Issues) }
                else { $script:issues+=@($result[0].Issues) }
                $mode=$script:scanMode; $cancelled=$result[0].Cancelled; $rows=@($result[0].Rows)
                $script:scan.Dispose(); $script:scan=$null; $script:cancellation.Dispose(); $script:cancellation=$null
                $cancel.Enabled=$false; Set-Working $false
                if ($cancelled) { $status.Text='扫描已取消；未添加游戏。'; return }
                if ($mode -eq 'Installed') {
                    if (-not $rows.Count) { $status.Text='没有发现可用安装；可手动添加，扫描详情包含跳过原因。'; return }
                    $record=Select-Record $rows '选择要添加的已安装游戏'
                    if ($null -ne $record) { $script:pendingRecord=$record; Start-Scan 'Renderers' }
                } else { Add-ScannedGame $rows }
            } catch {
                if ($null -ne $script:scan) { $script:scan.Dispose(); $script:scan=$null }
                if ($null -ne $script:cancellation) { $script:cancellation.Dispose(); $script:cancellation=$null }
                $cancel.Enabled=$false; Set-Working $false; Show-Failure $_.Exception.Message
            }
        }
        if ($null -ne $script:worker -and $script:worker.HasExited) {
            try {
                $script:worker.Dispose(); $script:worker=$null
                if (-not [IO.File]::Exists($script:resultPath)) { throw '工作脚本未返回结果，请检查日志与部署状态。' }
                $result=[IO.File]::ReadAllText($script:resultPath) | ConvertFrom-Json
                if ($result.ExitCode -ne 0) { throw ('操作未完成，请查看日志。'+$result.Error) }
                Set-Working $false; $status.Text='操作完成。'+$status.Text
            } catch { Set-Working $false; Show-Failure $_.Exception.Message }
        }
    })
    $games.Add_SelectedIndexChanged({ if (-not $script:updating) { Refresh-Game } })
    $discover.Add_Click({ try { Start-Scan 'Installed' } catch { Show-Failure $_.Exception.Message } })
    $manual.Add_Click({ try {
        $path=Select-Directory
        if ($path) { $script:issues=@(); $script:pendingRecord=[pscustomobject]@{Platform='手动';Name=(Split-Path -Leaf $path);GameDirectory=$path;AppId='';DeclaredExecutable=''}; Start-Scan 'Renderers' }
    } catch { Show-Failure $_.Exception.Message } })
    $cancel.Add_Click({ if ($null -ne $script:cancellation) { $script:cancellation.Cancel(); $cancel.Enabled=$false } })
    $scanDetails.Add_Click({ [void][Windows.Forms.MessageBox]::Show($form, $(if ($script:issues.Count) { $script:issues -join "`r`n" } else { '没有扫描跳过记录。' }),'扫描详情') })
    $browseDirectory.Add_Click({ $path=Select-Directory; if ($path) { $directoryBox.Text=$path } })
    $browseRender.Add_Click({ try { $path=Select-Exe '选择真实渲染程序' $directoryBox.Text; if ($path) {
        $base=[IO.Path]::GetFullPath($directoryBox.Text).TrimEnd('\')+'\'
        if (-not $path.StartsWith($base,[StringComparison]::OrdinalIgnoreCase)) { throw '程序必须位于安装目录。' }
        $renderBox.Text=$path.Substring($base.Length)
    } } catch { Show-Failure $_.Exception.Message } })
    $browseLaunch.Add_Click({ try { $path=Select-Exe '选择启动程序' $directoryBox.Text; if ($path) {
        $base=[IO.Path]::GetFullPath($directoryBox.Text).TrimEnd('\')+'\'
        if (-not $path.StartsWith($base,[StringComparison]::OrdinalIgnoreCase)) { throw '程序必须位于安装目录。' }
        $launchBox.Text=$path.Substring($base.Length)
    } } catch { Show-Failure $_.Exception.Message } })
    $browseSteam.Add_Click({ $path=Select-Exe '选择 steam.exe'; if ($path) { $steamBox.Text=$path } })
    $save.Add_Click({ try { Save-Editors } catch { Show-Failure $_.Exception.Message } })
    $launch.Add_Click({ Start-Action 'Launch' }); $restore.Add_Click({ Start-Action 'Restore' }); $check.Add_Click({ Start-Action 'Check' })
    $logs.Add_Click({ try {
        $logPath=Join-Path $root 'logs'; if (-not [IO.Directory]::Exists($logPath)) { throw '尚未生成脚本日志。' }
        Start-Process -FilePath explorer.exe -ArgumentList @($logPath)
    } catch { Show-Failure $_.Exception.Message } })
    $form.Add_FormClosing({param($sender,$e)
        if ($null -ne $script:worker) { $e.Cancel=$true; Show-Failure '请等待检查或恢复完成。'; return }
        if ($null -ne $script:scan) { $script:cancellation.Cancel(); $e.Cancel=$true; $status.Text='正在取消扫描，完成后可关闭界面。' }
    })
    Reload-Games
    if ($SmokeTest) {
        if ($PreviewPath) { $form.Show(); [Windows.Forms.Application]::DoEvents(); $bitmap=[Drawing.Bitmap]::new($form.Width,$form.Height); try { $form.DrawToBitmap($bitmap,[Drawing.Rectangle]::new(0,0,$form.Width,$form.Height)); $bitmap.Save($PreviewPath) } finally { $bitmap.Dispose() }; $form.Hide() }
        Write-Output ('PASS: DLSSG launcher UI; profiles='+$games.Items.Count)
    } else { $timer.Start(); [void]$form.ShowDialog() }
    $timer.Dispose(); $form.Dispose()
} catch {
    if ($SmokeTest) { throw }
    Add-Type -AssemblyName System.Windows.Forms
    [void][Windows.Forms.MessageBox]::Show($_.Exception.Message,'DLSSG启动器无法打开','OK','Error')
    exit 1
}
