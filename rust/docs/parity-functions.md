# PARITY 函数级清单（生成物）

> 本文件由 `rust/tools/gen-parity-inventory.ps1` 生成，**不要手工编辑**。
> 契约与决策见 [PARITY.md](PARITY.md)。

- 生成时间（UTC）：2026-11-22T13:57:42.0131532Z
- 源文件：20 个，合计 5187 行，193 个函数
- PowerShell：5.1.26100.9711

## 1. 逐文件清单

### scripts/common/DeploymentJournal.ps1  →  core::journal / core::restore

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Get-JournalItem` | 8 | 5 | `F2F2D7B14844` | core::journal / core::restore |
| `Assert-JournalOrdinaryParents` | 14 | 13 | `E7D896DBC57B` | core::journal / core::restore |
| `Save-DeploymentJournal` | 28 | 22 | `55E54D23996A` | core::journal / core::restore |
| `Test-JournalDlssgOnly` | 51 | 3 | `A3717BCE39DA` | core::journal / core::restore |
| `Open-DeploymentJournal` | 55 | 64 | `392F60B0A284` | core::journal / core::restore |
| `Close-DeploymentJournal` | 120 | 12 | `8BDE7635FBAC` | core::journal / core::restore |
| `Assert-DeploymentJournal` | 133 | 50 | `FEA3C72E495A` | core::journal / core::restore |
| `New-DeploymentJournal` | 184 | 22 | `2DD5037E9F47` | core::journal / core::restore |
| `Set-DeploymentPhase` | 207 | 4 | `A7D4EE92B3B3` | core::journal / core::restore |
| `Test-DeploymentInputs` | 212 | 15 | `F47F56ABEC6B` | core::journal / core::restore |
| `Get-JournalTextHash` | 228 | 6 | `A8B731CA376A` | core::journal / core::restore |
| `Resolve-LegacyEngineDeployment` | 235 | 9 | `09290108AD6C` | core::journal / core::restore |
| `Start-DeploymentOperation` | 245 | 9 | `2D0157319B88` | core::journal / core::restore |
| `Complete-DeploymentOperation` | 255 | 5 | `31246E708B67` | core::journal / core::restore |
| `Register-ReusedDeploymentLink` | 261 | 11 | `9C7E3A1DC242` | core::journal |
| `Assert-DeploymentLinkSpec` | 273 | 18 | `0D96C80BC2B1` | core::journal / core::restore |
| `Register-OwnedStorageDll` | 292 | 12 | `3CD7005D93AD` | core::journal / core::restore |
| `Import-ReusedDeploymentLinks` | 305 | 18 | `7641D5047DD6` | core::journal / core::restore |
| `Restore-LegacyResourceDeployment` | 324 | 44 | `55423086ADB9` | core::journal / core::restore |
| `Assert-DeploymentResourcesRemoved` | 369 | 11 | `32E93E2640A7` | core::journal / core::restore |
| `Assert-RecoveryProcessesStopped` | 381 | 11 | `50787962444A` | core::journal / core::restore |
| `Restore-RecordedDeployment` | 393 | 106 | `F2C94E7ACB63` | core::restore |
| `Resolve-PreviousDeployment` | 500 | 7 | `0E21F84B944A` | core::journal / core::restore |

### scripts/common/LauncherConfig.ps1  →  core::config / core::settings

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Get-LauncherRoot` | 2 | 3 | `52D3A4365C0A` | core::config / core::settings |
| `Read-LauncherConfigDocument` | 6 | 38 | `55EE8AF060EA` | core::config / core::settings |
| `Read-LauncherConfigText` | 45 | 4 | `176093D9AE3C` | core::config / core::settings |
| `Read-LauncherConfigSection` | 50 | 46 | `8EB21904E18E` | core::config / core::settings |
| `Remove-LauncherValueQuotes` | 97 | 12 | `63619D071E41` | core::config / core::settings |
| `Get-LauncherText` | 110 | 11 | `E68783C69CD7` | core::config / core::settings |
| `Resolve-LauncherConfigPath` | 122 | 47 | `D28F0AF6F01E` | core::config / core::settings |
| `Save-LauncherDetectedPath` | 170 | 59 | `4B92A4436036` | core::config / core::settings |
| `Save-LauncherDetectedConfigDirectory` | 230 | 17 | `61AE515A0A8D` | core::config / core::settings |
| `Find-LauncherExecutable` | 248 | 17 | `0BA59C04AEC6` | core::config / core::settings |
| `Get-LauncherGameSettings` | 266 | 42 | `AC3291D72916` | core::config / core::settings |
| `Select-LauncherNvidiaDevice` | 309 | 14 | `B449BF778777` | core::config / core::settings |
| `Get-LauncherGpuSettings` | 324 | 12 | `B02E1D427344` | core::config / core::settings |
| `Resolve-LauncherAdvancedValue` | 338 | 15 | `31A75BB272AA` | core::settings |
| `Get-LauncherAdvancedSettings` | 354 | 11 | `E5A831D6B9D5` | core::config / core::settings |
| `Initialize-PortableAdvanced` | 366 | 10 | `D9F57C117242` | core::config / core::settings |
| `Set-LauncherDiagnosticOptions` | 377 | 9 | `5B71EEA3F443` | core::settings::diagnostic |
| `Get-LauncherAdvancedChoices` | 387 | 10 | `BCFDF200EC75` | core::settings |
| `Get-LauncherAdvancedDefault` | 398 | 10 | `697A0489C507` | core::settings |
| `Test-LauncherDiagnosticOptionsText` | 409 | 13 | `2FFF344D7FE2` | core::settings::diagnostic |

### scripts/common/LauncherResources.ps1  →  core::resources

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `New-DlssgLogCursor` | 2 | 13 | `359D2A5F98B9` | core::resources |
| `Get-DlssgSessionLog` | 16 | 30 | `798D0991B859` | core::resources |
| `Show-DlssgSessionLogStatus` | 47 | 16 | `11A56599456C` | core::resources |
| `Select-DlssgProxy` | 64 | 9 | `FBA308064CD4` | core::resources |
| `New-DlssgDeployment` | 74 | 9 | `19114703E527` | core::resources |
| `Assert-DlssgDeployment` | 84 | 39 | `EF54A27CA086` | core::resources |
| `Install-DlssgResources` | 124 | 21 | `A782D068150F` | core::resources |
| `Undo-DlssgResources` | 146 | 20 | `5D826A3BEB0E` | core::resources |

### scripts/common/PortableLauncher.ps1  →  core::prefs / core::session

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Get-PortableDataDirectory` | 5 | 7 | `71280B9DBAF8` | core::prefs / core::session |
| `Read-PortablePreferences` | 13 | 22 | `637B2BD1F534` | core::prefs / core::session |
| `Save-PortablePreferences` | 36 | 18 | `5AF12F4AEC83` | core::prefs / core::session |
| `Get-PortableSteamPath` | 55 | 7 | `58EC7DC9E46A` | core::prefs / core::session |
| `Get-PortableProfiles` | 63 | 26 | `908346669695` | core::prefs / core::session |
| `New-PortableDiscoveredProfile` | 90 | 22 | `8997280EF960` | core::prefs / core::session |
| `Set-PortableProfile` | 113 | 4 | `C73D3F90C192` | core::prefs / core::session |
| `Get-PortableGameModel` | 118 | 63 | `A1CF20D60ED6` | core::prefs / core::session |
| `Write-PortableLaunchConfig` | 182 | 36 | `55E185545260` | core::prefs / core::session |
| `New-PortableWorkerCommand` | 219 | 33 | `7E37DEC9D50C` | core::prefs / core::session |
| `New-PortableAdvancedControls` | 254 | 31 | `352D64A1F5C8` | core::prefs / core::session |
| `Set-PortableAdvancedControls` | 286 | 16 | `58AF00A96D3B` | core::prefs / core::session |
| `Save-PortableAdvancedControls` | 303 | 9 | `DA548EA2BD20` | core::prefs / core::session |
| `Get-PortableSelectedIndex` | 314 | 9 | `B9518D75687A` | core::prefs / core::session |
| `Save-PortableSelectedProfile` | 324 | 14 | `967F23D80185` | core::prefs / core::session |

### scripts/common/GameDiscovery.ps1  →  core::discovery

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Get-TrustedGameRenderer` | 2 | 7 | `CFFE6BC04657` | core::discovery |
| `Get-UnrealConfigDirectoryCandidates` | 11 | 46 | `3C58FCEB533E` | core::discovery |
| `Test-DiscoveryCancelled` | 58 | 4 | `B9BA87224F03` | core::discovery |
| `Test-GameX64Executable` | 63 | 16 | `BDC124871137` | core::discovery |
| `Assert-GameOrdinaryExecutable` | 80 | 10 | `F1B857C56CB1` | core::discovery |
| `Test-GameAuxiliaryExecutable` | 91 | 9 | `6C9A36B6510F` | core::discovery |
| `Get-GameRendererCandidates` | 101 | 38 | `558AA9DAA5DC` | core::discovery |
| `Get-InstalledGameRecords` | 140 | 84 | `7B5910B1369E` | core::discovery |

### scripts/common/SteamLauncher.ps1  →  core::steam

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Get-SteamLibraryDirectories` | 2 | 13 | `614F9FAF62F5` | core::steam |
| `Find-SteamGameDirectory` | 16 | 20 | `3B48466A8319` | core::steam |
| `Resolve-SteamAppId` | 37 | 38 | `F9A89C91F251` | core::steam |
| `Get-SteamUE5Settings` | 76 | 55 | `0AA870FB55CF` | core::steam |
| `Get-SteamGameProcesses` | 132 | 14 | `3642D466B6EA` | core::steam |
| `Assert-SteamGameStopped` | 147 | 4 | `76FF6D11EB69` | core::steam |
| `Wait-SteamGameSession` | 152 | 14 | `3115B871A335` | core::steam |

### scripts/common/DirectLauncher.ps1  →  core::direct

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Initialize-PortableLaunchMode` | 2 | 7 | `DCCCA17DB4EA` | core::direct |
| `Get-PortableGameId` | 10 | 7 | `2484A88B07DE` | core::direct |
| `Resolve-DirectExecutable` | 18 | 8 | `4D5C2B3995E8` | core::direct |
| `Get-DirectUESettings` | 27 | 24 | `DA9FF2AE7893` | core::direct |
| `Start-ConfiguredUEGame` | 52 | 8 | `61CBC0527483` | core::direct |

### scripts/common/DlssgOnlyLauncher.ps1  →  core::session

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Get-DlssgOnlyDataDirectory` | 4 | 4 | `7BA678D6943F` | core::session |
| `New-DlssgOnlyProfile` | 9 | 10 | `1990C24D70CB` | core::session |
| `Get-DlssgOnlyModel` | 20 | 60 | `BF59635CF7A5` | core::session |
| `Write-DlssgOnlyLaunchConfig` | 81 | 36 | `A228BE4C685B` | core::session |

### scripts/common/LauncherSession.ps1  →  core::session

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Get-LauncherProcessNames` | 3 | 11 | `56DC738B3B0D` | core::session |
| `Wait-ForGameProcess` | 15 | 47 | `D3A1BD164EE5` | core::session |

### scripts/common/UnrealQuality.ps1  →  core::quality（重写：删探测/校准）

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Initialize-PortableQuality` | 86 | 9 | `30B4B383494E` | core::quality（改写：EngineHost） |
| `Resolve-PortableQuality` | 96 | 12 | `43D18B93477A` | core::quality（改写：EngineHost） |
| `Save-PortableProfileChange` | 110 | 6 | `87E5A53507AE` | core::quality（重写：删探测/校准） |
| `Get-PortableSavedEngine` | 117 | 7 | `DBF1BD60137E` | 删除 |
| `Save-PortableEngineDetection` | 125 | 13 | `95B48422E460` | 删除 |

### scripts/common/ScriptLogging.ps1  →  core::log

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Start-LauncherLog` | 2 | 18 | `25555C57FD34` | core::log |
| `Stop-LauncherLog` | 21 | 7 | `FC433B172EE4` | core::log |

### scripts/common/LauncherConsole.ps1  →  删除（无控制台）

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Initialize-LauncherConsole` | 2 | 72 | `2586279C40CE` | 删除（无控制台） |
| `Restore-LauncherConsole` | 75 | 6 | `9818043CEE7A` | 删除（无控制台） |

### scripts/common/LaunchFromBat.ps1  →  由 EXE 自提权替代

（无函数定义；整文件行为见 PARITY.md）

### scripts/games/Run-WutheringWaves-DLSSG.ps1  →  core::session + games::wuthering

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Write-Log` | 152 | 4 | `CCADEBD873EB` | core::session + games::wuthering |
| `Write-Detail` | 157 | 4 | `AA6B5D7FE0C1` | core::session + games::wuthering |
| `Get-HashOrNull` | 162 | 5 | `2102336B0C0E` | core::path |
| `Get-NormalizedPath` | 168 | 4 | `3C0CFE56D753` | core::path（三处重复，合并） |
| `Assert-FileExists` | 173 | 6 | `FE7BA0C51EC4` | core::session + games::wuthering |
| `Assert-NoGameProcess` | 180 | 11 | `6552AC1609DE` | core::session + games::wuthering |
| `Resolve-EngineConfigDirectory` | 192 | 20 | `434781E49599` | core::session + games::wuthering |
| `Assert-OwnedEngineRuntime` | 213 | 8 | `3A0627E08EEE` | core::session + games::wuthering |
| `Restore-EngineConfig` | 222 | 20 | `6DDFA0080054` | core::session + games::wuthering |
| `Prepare-EngineConfig` | 243 | 49 | `FFF6ED639234` | core::session + games::wuthering |
| `Undo-EngineConfigSetup` | 293 | 11 | `51D83AB11415` | core::session + games::wuthering |
| `Wait-ForGameSession` | 305 | 30 | `AC397AB9BDF3` | core::session + games::wuthering |
| `Test-StaleDiagnosticIniTarget` | 336 | 11 | `444B45A608E5` | core::session + games::wuthering |
| `Assert-LinkOrAbsent` | 348 | 30 | `820DF9AD33A2` | core::resources |
| `Ensure-Link` | 379 | 19 | `B183E90CD630` | core::resources |
| `Remove-ExpectedLink` | 399 | 18 | `47D85644CDE4` | core::session + games::wuthering |
| `Remove-GeneratedStorageDll` | 418 | 21 | `42E6F2FA2ADC` | core::session + games::wuthering |
| `Show-PostRunStatus` | 440 | 3 | `080A9EBAB84A` | core::session + games::wuthering |

### scripts/games/Run-NevernessToEverness-DLSSG.ps1  →  core::session + games::neverness

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Write-Log` | 124 | 4 | `CCADEBD873EB` | core::session + games::neverness |
| `Write-Detail` | 129 | 4 | `AA6B5D7FE0C1` | core::session + games::neverness |
| `Get-HashOrNull` | 134 | 5 | `2102336B0C0E` | core::path |
| `Get-NormalizedPath` | 140 | 4 | `3C0CFE56D753` | core::path（三处重复，合并） |
| `Assert-FileExists` | 145 | 6 | `FE7BA0C51EC4` | core::session + games::neverness |
| `Assert-NoRunningProcess` | 152 | 7 | `2FC2F7388E5C` | core::session + games::neverness |
| `Assert-NteEnginePaths` | 160 | 7 | `589CA456F60F` | core::session + games::neverness |
| `Assert-OwnedEngineRuntime` | 168 | 8 | `3A0627E08EEE` | core::session + games::neverness |
| `Restore-EngineConfig` | 177 | 20 | `6DDFA0080054` | core::session + games::neverness |
| `Prepare-EngineConfig` | 198 | 50 | `0A888D6C199D` | core::session + games::neverness |
| `Undo-EngineConfigSetup` | 249 | 11 | `51D83AB11415` | core::session + games::neverness |
| `Test-StaleDiagnosticIniTarget` | 261 | 11 | `AB91549887EB` | core::session + games::neverness |
| `Assert-LinkOrAbsent` | 273 | 36 | `092FC3229374` | core::resources |
| `Ensure-Link` | 310 | 19 | `B183E90CD630` | core::resources |
| `Remove-ExpectedLink` | 330 | 18 | `47D85644CDE4` | core::session + games::neverness |
| `Remove-GeneratedStorageDll` | 349 | 21 | `B3955A5D5C0B` | core::session + games::neverness |
| `Show-PostRunStatus` | 371 | 3 | `080A9EBAB84A` | core::session + games::neverness |

### scripts/games/Run-SteamUE5-DLSSG.ps1  →  core::session + games::steam

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Write-Log` | 153 | 4 | `CCADEBD873EB` | core::session + games::steam |
| `Write-Detail` | 158 | 4 | `AA6B5D7FE0C1` | core::session + games::steam |
| `Get-HashOrNull` | 163 | 5 | `2102336B0C0E` | core::path |
| `Get-NormalizedPath` | 169 | 4 | `3C0CFE56D753` | core::path（三处重复，合并） |
| `Assert-FileExists` | 174 | 6 | `FE7BA0C51EC4` | core::session + games::steam |
| `Assert-SteamEnginePaths` | 181 | 8 | `46E52A3B5E6E` | core::session + games::steam |
| `Assert-OwnedEngineRuntime` | 190 | 8 | `3A0627E08EEE` | core::session + games::steam |
| `Restore-EngineConfig` | 199 | 20 | `6DDFA0080054` | core::session + games::steam |
| `Prepare-EngineConfig` | 220 | 50 | `FC5925A6EBAE` | core::session + games::steam |
| `Test-StaleDiagnosticIniTarget` | 271 | 12 | `F3E61F9DC2C1` | core::session + games::steam |
| `Assert-LinkOrAbsent` | 284 | 36 | `092FC3229374` | core::resources |
| `Ensure-Link` | 321 | 19 | `B183E90CD630` | core::resources |
| `Show-PostRunStatus` | 341 | 3 | `080A9EBAB84A` | core::session + games::steam |

### scripts/tools/Export-PortableLauncher.ps1  →  core::export

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Get-PortableFileHash` | 4 | 6 | `9BFA071DEFF8` | core::export |
| `Assert-PortableZipArchive` | 11 | 26 | `F30DE0299489` | core::export |
| `Remove-PortableStagingDirectory` | 38 | 17 | `851F6F61125A` | core::export |

### scripts/tools/Toggle-NvidiaDeviceName.ps1  →  core::gpu

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Write-Log` | 22 | 4 | `CCADEBD873EB` | core::gpu |
| `Show-Message` | 27 | 14 | `75A1BA18C911` | core::gpu |
| `Show-CurrentStateDialog` | 42 | 16 | `652F3CB79D37` | core::gpu |
| `Assert-Administrator` | 59 | 15 | `39F89DA52592` | core::gpu |
| `Get-DeviceProperties` | 75 | 7 | `4C8F7E913C07` | core::gpu |
| `Split-DeviceDescription` | 83 | 16 | `60E36BDEC61D` | core::gpu |
| `Set-DeviceDescription` | 100 | 4 | `2DD8FA48A515` | core::gpu |
| `Get-FallbackOriginalDescription` | 105 | 15 | `6B8312F7F762` | core::gpu |
| `Show-Status` | 121 | 16 | `1E4C7B9A148E` | core::gpu |
| `Restore-FromState` | 138 | 16 | `C311406DAF13` | core::gpu |
| `Reset-SpoofState` | 155 | 11 | `5512145785FC` | core::gpu |
| `Enable-Spoof` | 167 | 37 | `BEC166A08683` | core::gpu |

### scripts/ui/Show-DlssgLauncher.ps1  →  launcher::ui

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `Add-Label` | 21 | 3 | `5315800D9FF9` | launcher::ui |
| `Add-Button` | 24 | 3 | `4C43455E26ED` | launcher::ui |
| `Add-Box` | 27 | 3 | `F75C2E3A2929` | launcher::ui |
| `Show-Failure` | 30 | 1 | `0DE04429288A` | launcher::ui |
| `Select-Exe` | 31 | 6 | `CDDC8BE8BEE7` | launcher::ui |
| `Select-Directory` | 37 | 5 | `CB7D1BB60B52` | launcher::ui |
| `Set-AdvancedVisible` | 77 | 8 | `4916E3AA40CE` | launcher::ui |
| `Refresh-Game` | 87 | 28 | `6D914C99E42D` | launcher::ui |
| `Reload-Games` | 115 | 13 | `8D54C72F16F7` | launcher::ui |
| `Set-Working` | 128 | 6 | `5F760AB5C42F` | launcher::ui |
| `Save-Editors` | 134 | 20 | `1267EB53B203` | launcher::ui |
| `Start-Action` | 154 | 22 | `9F828037B3B0` | launcher::ui |
| `Select-Record` | 176 | 11 | `5675904B9562` | launcher::ui |
| `Add-ScannedGame` | 187 | 20 | `DD2DB7AC4A52` | launcher::ui |
| `Start-Scan` | 207 | 20 | `24F003B52E96` | launcher::ui |

### scripts/ui/Show-SteamLauncher.ps1  →  launcher::ui

| 函数 | 行 | 行数 | 正文哈希 | Rust 归宿 |
|---|---:|---:|---|---|
| `New-Label` | 40 | 7 | `0DBAE3C1F44F` | launcher::ui |
| `New-Button` | 47 | 7 | `0768C3C55746` | launcher::ui |
| `Show-Failure` | 54 | 4 | `FFAC492CF358` | launcher::ui |
| `Select-Folder` | 58 | 8 | `2BEEB70CBC6E` | launcher::ui |
| `Select-File` | 66 | 9 | `8D9B556BE2BC` | launcher::ui |
| `Refresh-Game` | 131 | 49 | `BCF6675851CF` | launcher::ui |
| `Load-Games` | 180 | 14 | `AF146BC4A67B` | launcher::ui |
| `Set-Working` | 194 | 7 | `B4E8DCDA617D` | launcher::ui |
| `Start-Action` | 201 | 39 | `1F67CD0C3841` | launcher::ui |
| `Save-Editors` | 297 | 14 | `8F5E912CD05D` | launcher::ui |
| `Select-DiscoveryRecord` | 319 | 13 | `8D526945A83E` | launcher::ui |
| `Start-Discovery` | 332 | 26 | `3605F3EC8E9C` | launcher::ui |
| `Complete-Discovery` | 358 | 37 | `739B30435767` | launcher::ui |
| `Add-LauncherGame` | 395 | 16 | `2DD5FD6B189F` | launcher::ui |

## 2. 跨文件重复函数

正文哈希相同的副本可以只保留一份实现；哈希不同的必须逐条比对，不能默认等价。

| 函数 | 副本数 | 正文全同 | 行数 | 出现文件 |
|---|---:|---|---:|---|
| `Assert-LinkOrAbsent` | 3 | **否** | 30 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Prepare-EngineConfig` | 3 | **否** | 49 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Refresh-Game` | 2 | **否** | 28 | Show-DlssgLauncher.ps1 / Show-SteamLauncher.ps1 |
| `Remove-GeneratedStorageDll` | 2 | **否** | 21 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 |
| `Save-Editors` | 2 | **否** | 20 | Show-DlssgLauncher.ps1 / Show-SteamLauncher.ps1 |
| `Set-Working` | 2 | **否** | 6 | Show-DlssgLauncher.ps1 / Show-SteamLauncher.ps1 |
| `Show-Failure` | 2 | **否** | 1 | Show-DlssgLauncher.ps1 / Show-SteamLauncher.ps1 |
| `Start-Action` | 2 | **否** | 22 | Show-DlssgLauncher.ps1 / Show-SteamLauncher.ps1 |
| `Test-StaleDiagnosticIniTarget` | 3 | **否** | 11 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Assert-FileExists` | 3 | 是 | 6 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Assert-OwnedEngineRuntime` | 3 | 是 | 8 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Ensure-Link` | 3 | 是 | 19 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Get-HashOrNull` | 3 | 是 | 5 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Get-NormalizedPath` | 3 | 是 | 4 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Remove-ExpectedLink` | 2 | 是 | 18 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 |
| `Restore-EngineConfig` | 3 | 是 | 20 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Show-PostRunStatus` | 3 | 是 | 3 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Undo-EngineConfigSetup` | 2 | 是 | 11 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 |
| `Write-Detail` | 3 | 是 | 4 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 |
| `Write-Log` | 4 | 是 | 4 | Run-WutheringWaves-DLSSG.ps1 / Run-NevernessToEverness-DLSSG.ps1 / Run-SteamUE5-DLSSG.ps1 / Toggle-NvidiaDeviceName.ps1 |
