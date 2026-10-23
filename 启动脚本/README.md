# 启动脚本：实现与开发

基线：Windows PowerShell 5.1、Windows 10/11 x64、0.4.0资源布局。普通用户操作见 [使用说明](使用说明.md)；完整配置接口见 [配置与命令行参考](docs/配置与命令行参考.md)。本文只保留代码结构、行为约束和开发入口。

## 入口与模块

| 位置 | 职责 |
|---|---|
| `启动器.bat`、`scripts/ui/Show-SteamLauncher.ps1` | WinForms界面，保存选择、触发检查/恢复、交接独立工作脚本 |
| `scripts/common/PortableLauncher.ps1` | 共享预设与本机偏好合并、模型校验、运行请求INI、工作进程命令 |
| `scripts/common/SteamLauncher.ps1` | Steam库/清单解析、AppID匹配、完整EXE路径进程识别与会话等待 |
| `scripts/common/DirectLauncher.ps1` | Steam/Direct模式兼容、稳定游戏标识、直接启动设置与EXE调度 |
| `scripts/common/UnrealQuality.ps1` | 后台UE主版本识别、缓存、画质路径映射及保存失败回退 |
| `scripts/games/Run-SteamUE5-DLSSG.ps1` | Steam与Direct共用部署、启动、监控和恢复工作脚本 |
| `scripts/games/Run-WutheringWaves-DLSSG.ps1`、`Run-NevernessToEverness-DLSSG.ps1` | 鸣潮、异环专用会话流程 |
| `scripts/common/LauncherConfig.ps1` | INI编码兼容、别名、路径规范化及显式参数覆盖 |
| `scripts/common/LauncherResources.ps1`、`DeploymentJournal.ps1` | 共用资源部署、互斥、持久化记录与恢复 |
| `scripts/common/LauncherConsole.ps1`、`ScriptLogging.ps1` | 控制台保护、脚本日志 |
| `scripts/tools/Export-PortableLauncher.ps1` | 白名单导出便携目录及可选ZIP |

## 配置与状态接口

- 图形预设为 `presets/SteamUE5.json`；本机偏好/请求位于 `%LOCALAPPDATA%\DlssgSteamLauncher\<目录路径哈希>`，不读取个人 `启动配置.ini`。旧偏好缺少 `LaunchMode`、`QualityPreset`、`EngineOverride` 时默认 `Steam`、`Current`、`Auto`。
- 模式字段为 `LaunchMode=Steam|Direct`。Steam使用AppID；Direct使用稳定的 `Id=direct-<32位GUID>`，`LaunchExecutable` 与 `RenderExecutable` 均相对游戏根目录，允许二者不同，不接受目录外EXE。Direct不查询Steam清单或注册表。
- CLI入口读取统一INI，显式命令行覆盖配置。Steam请求使用 `[SteamUE5:desktop]`；Direct请求使用 `[DirectUE:desktop]`，工作脚本通过 `-LaunchMode Direct -GameKey <32位GUID>` 选择。Direct仅增加“启动程序 / LaunchExecutable”，不传Steam的 `-applaunch`。
- `EngineConfigSource` 保留“当前配置”的源；`QualityPreset=Tier1..Tier5` 根据有效UE主版本映射 `配置/虚幻通用分类/UE4|UE5/<档位>/Engine.ini`，不合并配置；Tier5对应UE4/UE5各自的 `05-自定义/Engine.ini`，不是性能第五档。新添加游戏默认Tier2，旧游戏Current；高级设置选择新INI后恢复Current。
- 引擎证据为标准 `Engine/Build/Build.version`、明确Unreal版本元数据、EXE内ASCII/UTF-16LE发行标记。首次分块后台扫描；成功结果以 `DetectedEngineMajor/DetectedEngineSource` 按游戏写入本机偏好，重开界面、检查或启动时直接复用，不因EXE更新或搬盘自动重扫。未成功识别仍可重试；单次进程中的扫描缓存包含路径、大小和修改时间，旧异步结果不覆盖当前选择。`EngineOverride=UE4|UE5` 优先，冲突/未知不允许档位启动；`.utoc/.ucas`不能单独证明UE5。
- 恢复记录为 `.state/wuthering|nte|steam-<AppID>|direct-<GUID>/active.json`。保留稳定身份，优先恢复旧记录，不用新偏好重定向原配置位置。

## 部署与会话约束

- 图形界面仅在成功创建独立提权工作进程后隐藏并退出，释放句柄不能终止工作进程；取消提权保留界面。检查与恢复保留界面等待结果。
- Steam通过 `steam.exe -applaunch`；Direct以启动EXE所在目录为工作目录运行EXE，无自定义参数。二者按真实渲染EXE完整路径监控，等待启动上限120秒，全部目标进程退出后恢复。鸣潮保留默认10秒热重启窗口，异环首次退出后清理，不统一策略。
- 六种代理按version、winmm、dbghelp、dinput8、dxgi、d3d12映射1–6，默认1。一次部署一个，记录实际代理及storage哈希；未知文件、其他代理或变化的链接保留并报冲突。资源为runtime、backend、kernels、xefg、sr。
- Engine.ini从明文源生成带标记的只读临时副本，以链接部署；原件按字节备份/恢复，不解析、合并或加解密。当前默认源不使用历史test目录。加密实现仅保留于历史分支 `0.4.0带加密`。
- 记录原子写入，操作前intent、成功后done；写入失败停止部署。恢复顺序为 `engineLink → backup → runtime → storage → link → diagnostic → directory`，依赖未恢复时保留必要文件。被游戏替换的普通Engine.ini经备份校验后归档到recovered，不覆盖冲突。
- `-RestoreDeployment` 完整恢复，`-RestoreEngineConfig` 仅恢复画面配置并更新记录；`-KeepLinks` 保留部署。DryRun不启动或部署，但写日志；遇未完成部署只展示恢复计划并停止。只有恢复与残留校验均成功才删除记录。

## 开发验证

在仓库根目录按修改范围选择测试，不重复运行无关游戏检查：

```powershell
powershell.exe -NoProfile -File 启动脚本/tests/PortableLauncher.Tests.ps1
powershell.exe -NoProfile -File 启动脚本/tests/PortableLauncherHandoff.Tests.ps1
powershell.exe -NoProfile -File 启动脚本/tests/UnrealQuality.Tests.ps1
powershell.exe -NoProfile -File 启动脚本/tests/DirectLauncher.Tests.ps1
powershell.exe -NoProfile -File 启动脚本/tests/SteamUE5.Tests.ps1
powershell.exe -NoProfile -File 启动脚本/tests/DeploymentJournal.Tests.ps1
powershell.exe -NoProfile -File 启动脚本/scripts/ui/Show-SteamLauncher.ps1 -SmokeTest
```

配置/编码改动另查 `tests/LauncherConfig.Tests.ps1`、`tests/Compatibility.Tests.ps1`。隔离测试不得触碰真实游戏部署；语法、模型、DryRun与界面冒烟不能证明游戏加载、画面或FPS。原生符号链接因权限跳过时需明确说明。

## 便携导出

可选Bandizip模式：运行 `打包给朋友.bat -b`，或给导出PS1传 `-b`。调用 `C:\Bandizip\bz.exe` 输出7z，使用本机7.32版默认LZMA2及最高等级 `-l:9`；压缩及完整性检查成功后只保留7z，失败保留导出目录。三种类型与顶层文件夹命名不变；不加 `-b` 仍用内置ZIP。打包脚本和Bandizip本身不包含在分享包内。参数依据[Bandizip官方说明](https://www.bandisoft.com/bandizip/help/parameter/)。

压缩包内只有一个顶层文件夹：类型1、2为“帧生成启动器”，类型3为“鸣潮异环帧生成解锁”；资源、说明和启动脚本均在其中，解压后进入该文件夹使用。

双击 `打包给朋友.bat`，输入 `1=完整（启动器+鸣潮+异环）`、`2=启动器` 或 `3=鸣潮+异环`。在仓库 `dist/DLSSG-Full/Launcher/WuWa-NTE-日期时间.zip`（按类型选择名称）生成压缩包，成功后自动删除同名临时目录，仅保留ZIP。无需管理员权限、Git、Python 或第三方压缩软件；已有目的目录或 ZIP 不覆盖。

也可指定导出位置，不复制使用中的工作树：

```powershell
powershell.exe -NoProfile -File 启动脚本/scripts/tools/Export-PortableLauncher.ps1 -Destination D:/分享/DLSSG便携启动器 -Zip
```

白名单按类型包含实际依赖模块、游戏入口、配置及说明，三种均保留完整0.4.0六个资源DLL；启动器包含六种代理、UE候选/自定义配置及预设，鸣潮+异环只包含version/winmm代理、专用配置、显卡工具和入口。完整包为两者并集，另含Steam命令行BAT。含专用入口的包现场生成干净的启动配置模板，不读取个人启动INI，收件人需填写游戏目录。排除本机偏好、日志、.state、备份、测试、子模块、打包工具和Git。ZIP使用.NET内置Optimal压缩，先写临时文件再发布，输出大小和SHA256。源文件/入口增减时同步白名单，旧导出包不代表当前代码。非交互调用用 `-PackageType 1/2/3`；省略时兼容原有行为，导出启动器。

## 参考文档

- [配置与命令行参考](docs/配置与命令行参考.md)：分组/别名、参数、编码、路径写法、上游子模块与迁移。
- [画质档位与来源](配置/虚幻通用分类/README.md)：八档候选及来源哈希；参考子模块更新须明确触发，不能替换个人默认源。
- [日志检查规范](../docs/启动日志检查规范.md)：按实际游戏PID核对加载、执行和恢复；组件计数不当作FPS。
- [历史运行记录](docs/历史运行记录.md)：2026-10-16的现场证据，不代表当前版本验收。
- [跨设备兼容性检查](docs/跨设备兼容性检查.md)：Windows与硬件验证边界。
