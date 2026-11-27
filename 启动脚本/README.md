# 启动脚本：实现与开发

基线：Windows PowerShell 5.1、Windows 10/11 x64、0.4.0资源布局。普通用户操作见 [使用说明](使用说明.md)；完整配置接口见 [配置与命令行参考](docs/配置与命令行参考.md)。本文只保留代码结构、行为约束和开发入口。

## 入口与模块

Rust 启动器的实现见 [Rust README](../rust/README.md)。两个实现的配置互斥均使用实际
`Engine.ini` 路径计算。Rust 若读到旧的 `启动脚本/.state` 记录，会在命中路径上恢复，
并在残留检查通过后删除记录。

### 独立 DLSSG 通用启动器

`通用启动器.bat` 打开 `scripts/ui/Show-DlssgLauncher.ps1`。界面与游戏列表独立，首次为空；本机偏好和运行请求保存在 `%LOCALAPPDATA%\DlssgOnlyLauncher\<目录路径哈希>`。模型及请求在 `scripts/common/DlssgOnlyLauncher.ps1`，安装发现和渲染候选在 `scripts/common/GameDiscovery.ps1`。

支持 Steam、Epic、GOG 的本机安装发现：Steam 使用登记库及严格清单匹配；Epic 读取 ProgramData 安装清单，排除 DLC/无效入口；GOG 读取安装注册信息并结合 `goggame-*.info`。不做全盘扫描。所选游戏目录的候选扫描在后台执行，可取消；不穿过目录链接，跳过工具、辅助程序及非 x64 PE。目录深度最多12层、访问目录最多12000个；跳过原因可在“扫描详情”查看。多个候选展示相对路径和证据，由用户确认，平台启动入口与渲染程序可以不同。

Steam 使用“Steam启动”；Epic、GOG 和手动添加使用“EXE启动”，不增加其他平台协议，也不绕过游戏的平台验证。该入口不限制 UE 主版本；识别或部署成功不代表支持帧生成。候选排序参考 [RTX-FG-Manager](https://github.com/pandaligx/RTX-FG-Manager/blob/646486bc85ec9773c374b691f4bd22467aa179d8/rust/src/scanner.rs) 的多证据筛选思路，以 PowerShell 实现；仅有超分组件不构成帧生成支持证据。

工作进程仍使用 `Run-SteamUE5-DLSSG.ps1`，固定传 `-DlssgOnly`。该模式不读取、备份、部署或恢复 Engine.ini，不要求配置目录或配置源；继续部署完整资源和临时 `dlssg_sm86.ini`，超分Mode=Off，保留游戏自身DLSS超分，代理选择和会话恢复沿用既有流程。记录使用 `DeploymentMode=DlssgOnly`、空 Engine/Runtime/Backup，不允许配置类操作；旧记录缺少模式时按原有模式解释。两种模式不能交叉恢复；同渲染目录的其他遗留记录会阻止新部署，须回原入口恢复。共用渲染目录互斥，新模式不使用配置路径互斥。

验证：`powershell.exe -NoProfile -File 启动脚本/tests/DlssgOnlyLauncher.Tests.ps1`；界面支持 `-SmokeTest` 和可选 `-PreviewPath`。测试使用隔离安装记录、伪程序及资源，不启动真实游戏。

### 虚幻启动器自动扫描

虚幻入口的“添加其他游戏”复用 `GameDiscovery.ps1`，支持 Steam/Epic/GOG 安装发现及手动选目录。渲染候选唯一最高分且至少60分时自动采用；同分或弱证据提示选择。Epic/GOG使用Direct，优先保留平台声明的启动EXE，其缺失时用渲染EXE。扫描成功不等于UE主版本或组件兼容性验证。

配置扫描根据安装目录、Binaries/Win64上方项目目录、去除平台/构建后缀的EXE名，检查安装内 `Saved/Config`、`%LOCALAPPDATA%/<项目名>/Saved/Config` 及系统文档 `My Games/<项目名>/Saved/Config` 下的 Windows、WindowsNoEditor、WindowsClient。已有普通 Engine.ini 或 GameUserSettings.ini 才形成候选，不穿过链接；去重后唯一候选自动填入，多候选选择、无候选手选。不改变优化源，不创建游戏配置、不全盘搜索。

扫描后台执行，可取消，问题见“添加其他游戏 → 扫描详情”；完成选择前不保存设置。“重新查找”仅在当前配置目录失效且无遗留部署时补查，保存前再次核对恢复记录，沿用已有标识。隔离测试为 `tests/UnrealDiscovery.Tests.ps1`；虚幻便携包也包含共用GameDiscovery模块。

### 共用模块与原入口

| 位置 | 职责 |
|---|---|
| `虚幻启动器.bat`、`scripts/ui/Show-SteamLauncher.ps1` | WinForms界面，保存选择、触发检查/恢复、交接独立工作脚本 |
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

main 已整合上游0.4.2，资源布局沿用0.4.0。各入口新诊断INI默认使用 `Optimized=2`、`Preset=A`、超分 `Mode=Off`；不改变各入口的生成帧数及退出策略。Off仅关闭本项目超分接管，游戏内DLSS超分照常。旧0.4.0诊断文本（Auto/On、鸣潮更早版本）继续精确识别，不通过放宽匹配认领未知配置；已有部署仍优先按记录哈希恢复。两个SR大文件仍由本地提供，不纳入Git。0.4.2要求R580+，真实游戏兼容性另行验证。

共用日志定位在 `LauncherResources.ps1`：运行前记录 `dlssg_sm86/logs/dlssg_sm86.log` 文件偏移和时间，运行后只统计新追加且时间、EXE名匹配的行并列出PID；日志截断或重建时从头读取但仍按时间筛选。热重启可匹配同一EXE多个PID。日志缺失、无本次记录或读取失败不影响恢复结果；旧loader/backend日志仅作为历史线索展示。定位不代表功能成功，不把帧数统计当FPS。隔离验证见 `tests/Diagnostic041.Tests.ps1`。

显卡名称工具支持 `-ResetState`（简写 `-r`）：当前不是目标伪装名称时，删除旧 ProgramData 状态备份，再走原有备份及切换流程；仍为伪装名称时拒绝重置。该参数与只读 `-Status` 互斥，提权时继续传递。

- 图形预设为 `presets/SteamUE5.json`；本机偏好/请求位于 `%LOCALAPPDATA%\DlssgSteamLauncher\<目录路径哈希>`，不读取个人 `启动配置.ini`。图形界面旧偏好缺少 `LaunchMode`、`QualityPreset`、`EngineOverride` 时默认 `Steam`、`Tier2`、`Auto`。旧 Current 偏好在读取时一次性改为 Tier2 并原子保存；底层不支持 Current 档位。
- 模式字段为 `LaunchMode=Steam|Direct`。Steam使用AppID；Direct使用稳定的 `Id=direct-<32位GUID>`，`LaunchExecutable` 与 `RenderExecutable` 均相对游戏根目录，允许二者不同，不接受目录外EXE。Direct不查询Steam清单或注册表。
- CLI入口读取统一INI，显式命令行覆盖配置。Steam请求使用 `[SteamUE5:desktop]`；Direct请求使用 `[DirectUE:desktop]`，工作脚本通过 `-LaunchMode Direct -GameKey <32位GUID>` 选择。Direct仅增加“启动程序 / LaunchExecutable”，不传Steam的 `-applaunch`。
- 图形界面只提供 `QualityPreset=Tier1..Tier5`，根据有效UE主版本映射 `配置/虚幻通用分类/UE4|UE5/<档位>/Engine.ini`；Tier5对应各自的 `05-自定义/Engine.ini`。缺少画质字段直接默认 Tier2，旧 Current 偏好在读取时一次性改为 Tier2 并原子保存，原 EngineConfigSource 字段保留供兼容，不删除配置文件；命令行配置源和旧部署恢复不变。
- 引擎证据为标准 `Engine/Build/Build.version`、明确Unreal版本元数据、EXE内ASCII/UTF-16LE发行标记。首次分块后台扫描；成功结果以 `DetectedEngineMajor/DetectedEngineSource` 按游戏写入本机偏好，重开界面、检查或启动时直接复用，不因EXE更新或搬盘自动重扫。未成功识别仍可重试；单次进程中的扫描缓存包含路径、大小和修改时间，旧异步结果不覆盖当前选择。`EngineOverride=UE4|UE5` 优先，冲突/未知不允许档位启动；`.utoc/.ucas`不能单独证明UE5。
- 恢复记录为 `.state/wuthering|nte|steam-<AppID>|direct-<GUID>/active.json`。保留稳定身份，优先恢复旧记录，不用新偏好重定向原配置位置。

## 部署与会话约束

- 图形界面仅在成功创建独立提权工作进程后隐藏并退出，释放句柄不能终止工作进程；取消提权保留界面。检查与恢复保留界面等待结果。
- Steam通过 `steam.exe -applaunch`；Direct以启动EXE所在目录为工作目录运行EXE，无自定义参数。二者按真实渲染EXE完整路径监控，等待启动上限120秒，全部目标进程退出后恢复。鸣潮保留默认10秒热重启窗口，异环首次退出后清理，不统一策略。
- 六种代理按version、winmm、dbghelp、dinput8、dxgi、d3d12映射1–6，默认1。一次部署一个，记录实际代理及storage哈希；本启动器遗留的其他代理链接或目标变化的链接保留并报冲突，第三方代理文件（如 mod 注入器）记录后共存加载、不修改对方文件；代理名需在游戏 EXE 静态导入表中才会被加载，部分 UE 游戏不含 dinput8。资源为runtime、backend、kernels、xefg、sr。
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

所有类型均生成 `package-manifest.json`：`FormatVersion=1`、`PackageType`、`Files` 数组；每项记录包内相对路径 `Path`、字节数 `Length` 和 `SHA256`，包含生成的说明/配置，不包含清单自身。不记录本机绝对路径，清单用于核对包内容，不证明资源版本或运行兼容性。SR两个文件仍按完整资源布局打包，缺失时提示从匹配的完整发行包或本地备份补齐，Git不提供它们。

内置ZIP写入临时文件后，逐条解压读取并与导出目录比对路径、数量、大小及SHA256，拒绝缺失、重复、额外或内容不符的条目；只有校验成功才发布正式ZIP并删除临时目录。失败不发布，保留导出目录，清理失败的临时压缩文件。Bandizip保持原有压缩及 `t` 校验。导出说明同步0.4.2/R580+、Preset=A和超分Off、扫描选择流程及按需使用显卡名称工具。测试见 `tests/PortableExport.Tests.ps1`，均使用微型隔离资源。

可选Bandizip模式：运行 `打包给朋友.bat -b`，或给导出PS1传 `-b`。调用 `C:\Bandizip\bz.exe` 输出7z，使用本机7.32版默认LZMA2及最高等级 `-l:9`；压缩及完整性检查成功后只保留7z，失败保留导出目录。四种类型均可使用；不加 `-b` 仍用内置ZIP。打包脚本和Bandizip本身不包含在分享包内。参数依据[Bandizip官方说明](https://www.bandisoft.com/bandizip/help/parameter/)。

压缩包内只有一个顶层文件夹：类型1为“帧生成启动器”，类型2为“通用启动器”，类型3为“虚幻启动器”，类型4为“鸣潮异环帧生成解锁”；资源、说明和启动脚本均在其中，解压后进入该文件夹使用。

双击 `打包给朋友.bat`，输入 `1=完整`、`2=通用启动器`、`3=虚幻启动器` 或 `4=鸣潮+异环`。在仓库 `dist/DLSSG-<类型>-日期时间.zip`（类型标记分别为Full、General、Unreal、WuWa-NTE）生成压缩包，成功后自动删除同名临时目录，仅保留ZIP。无需管理员权限、Git、Python 或第三方压缩软件；已有目的目录或 ZIP 不覆盖。

也可指定导出位置，不复制使用中的工作树：

```powershell
powershell.exe -NoProfile -File 启动脚本/scripts/tools/Export-PortableLauncher.ps1 -Destination D:/分享/DLSSG便携启动器 -Zip
```

白名单按类型包含实际依赖模块、游戏入口、配置及说明，四种均保留完整0.4.0六个资源DLL。通用包包含通用界面、游戏发现、六种代理及必要模块，不包含UE画质配置和预设；虚幻包包含虚幻界面、六种代理、UE候选/自定义配置及预设，不包含通用界面或游戏发现模块。通用模型复用PortableLauncher，因此仍包含UnrealQuality代码模块，但不包含UE配置文件。鸣潮+异环只包含version/winmm代理、专用配置、显卡工具和入口。完整包为三类并集，另含Steam命令行BAT。含专用入口的包现场生成干净的启动配置模板，不读取个人启动INI，收件人需填写游戏目录。排除本机偏好、日志、.state、备份、测试、子模块、打包工具和Git。ZIP使用.NET内置Optimal压缩，先写临时文件再发布，输出大小和SHA256。源文件/入口增减时同步白名单，旧导出包不代表当前代码。非交互调用用 `-PackageType 1/2/3/4`；省略时为2，导出通用启动器。 菜单末尾的 `0 全部依次打包` 按1→2→3→4分别生成四个包，也可传 `-PackageType 0`；沿用ZIP或 `-b` 模式。指定 `-Destination` 时将其作为前缀，分别加上 `-Full`、`-General`、`-Unreal`、`-WuWa-NTE`。整批先检查输出冲突，执行中失败则停止后续类型，保留此前成功的包和失败现场。

## 参考文档

- [配置与命令行参考](docs/配置与命令行参考.md)：分组/别名、参数、编码、路径写法、上游子模块与迁移。
- [画质档位与来源](配置/虚幻通用分类/README.md)：八档候选及来源哈希；参考子模块更新须明确触发，不能替换个人默认源。
- [日志检查规范](../docs/启动日志检查规范.md)：按实际游戏PID核对加载、执行和恢复；组件计数不当作FPS。
- [历史运行记录](docs/历史运行记录.md)：2026-10-16的现场证据，不代表当前版本验收。
- [跨设备兼容性检查](docs/跨设备兼容性检查.md)：Windows与硬件验证边界。

## 通用与虚幻启动器高级设置

两套图形入口分别按游戏在本机偏好保存 `SuperResolutionMode`、`DlssgPreset`、`Optimized` 和 `Level`，缺失字段默认 `Off` / `A` / `2` / `2`，不改共享预设或根目录 `dlssg_sm86.ini`。超分模式为 `Off`、`M-INT8`、`L-INT8`、`M-FP16`、`L-FP16`；DLSSG 预设为 `A`、`B`、`C`。Off 不接管游戏超分；INT8 使用项目内核，FP16 使用 NVIDIA 内核。B 的 UI 重组需要游戏提供相应输入，C 为轻量插帧；兼容性与收益需实测。

命令行 `Run-SteamUE5-DLSSG.ps1`（包括 `-DlssgOnly` 及 Direct 模式）接受 `-SuperResolutionMode M-INT8 -DlssgPreset B`；所选 `[SteamUE5]`、`[SteamUE5:名称]` 或 `[DirectUE:名称]` 组可写同名键。优先级：显式命令行 > 所选组 > 缺省 Off/A。图形入口通过独立运行请求交接，不读取个人启动配置 INI。

旧 `DlssgPreset=Auto` 提示并规范化为 A；`SuperResolutionMode=On` 和其他非法枚举原样传给运行库，由其记录 `configuration_invalid` 并禁用对应功能，不阻止游戏启动、不静默回退。界面显示警告并留空对应下拉框，明确选择有效项后才替换。控制字符及 INI 结构字符输入会被拒绝。恢复按旧部署记录路径和哈希执行；无记录诊断配置仅精确识别历史模板及合法的 240 种新组合，未知内容保留。鸣潮与异环也支持这四个高级参数，退出等待与恢复策略保持各自原有行为。

高级设置同时提供 `Optimized=0/1/2/3`（原厂／逐位一致／快、有损／最快、有损）和日志 `Level=0/1/2/3`（关闭／仅错误／配置与能力／详细追踪）。沿用启动器既有默认 `2/2`，与资源出厂 INI 的 `1/1` 区分；旧偏好缺字段时也沿用 2/2。命令行参数为 `-Optimized`、`-Level`，INI 所选游戏组使用同名键，优先级与另外两个高级选项一致。非法数值原样交由运行库处理，控制字符等结构输入仍拒绝。Level=0 不生成组件日志，无法依靠组件日志核实加载。

两套启动器在各自本机偏好中以 `LastSelectedProfileId` 保存上次选择的游戏，切换时立即原子保存；重开按稳定条目 ID 恢复，不依赖列表顺序。旧偏好没有该字段或该条目已移除时选第一个可用条目，空列表不选择；保存或新增游戏后的显式选择优先。该字段随其他偏好写入保留，不进入共享预设或便携导出；写入失败提示并保留原记录。

鸣潮／异环入口支持在 `启动配置.ini` 的 `[鸣潮]`、`[异环]` 中分别设置 `SuperResolutionMode`、`DlssgPreset`、`Optimized`、`Level`，缺失时默认 `Off / A / 2 / 2`。也可在对应 BAT 后传 `-SuperResolutionMode L-INT8 -DlssgPreset A -Optimized 1 -Level 2`；显式命令行优先于 INI。Mode=Off 保留游戏自身超分，Level=0 不生成组件日志。Auto 预设提示后按 A，其余非法枚举／数值保留给运行库处理；控制字符等 INI 结构输入拒绝。超分资源须与代理版本匹配。恢复使用原部署记录，不受新选择影响；鸣潮的热重启等待和异环首次退出清理策略不变。
