# M0 / Phase 0 门禁报告

- 日期：2026-11-22
- 状态：**规格与样本完成；spike 4 项达标、1 项未达标（GUI 常驻内存）→ 需决策后再进 M1**
- 计划基线：[rust开发.md](../plan/plan.md) §5 Phase 0、§9 性能验收
- 契约与样本：[PARITY.md](PARITY.md)、[parity-functions.md](parity-functions.md)、[../fixtures/](../fixtures/README.md)

## 1. 环境

| 项 | 值 |
|---|---|
| OS | Windows 11 专业版 build 26300 |
| CPU | 12th Gen Intel i7-12700H |
| GPU | NVIDIA GeForce RTX 3080 Ti Laptop（驱动 32.0.16.2101；另有 OrayIddDriver 虚拟显示器） |
| 显示 | 3840×2160 @ 150%（DPI 144） |
| 工具链 | rustc / cargo 1.99.1，`x86_64-pc-windows-msvc`，VS2022 VC 工具 |
| 构建 | `--release`，`opt-level="s"`、`lto=true`、`codegen-units=1`、`panic="abort"`、`strip=true` |

## 2. 交付物

| 文件 | 内容 | 数量 |
|---|---|---|
| `docs/PARITY.md` | 14 条兼容契约 + 8 条新增规则 + 8 条有意偏离 + 未验证清单 | 1 份 |
| `docs/parity-functions.md` | 20 个 PS1 的函数级映射、正文哈希、跨文件重复分析 | 193 个函数 |
| `fixtures/**` | 模板 / 参数矩阵 / 识别边界 / journal 阶段 / 路径用例 / 清单 | 33 个文件 |
| `tools/gen-fixtures.ps1` | 调用现有 PS 实现产出样本（只在 `%TEMP%` 写入） | 1 |
| `tools/gen-parity-inventory.ps1` | 重新生成函数清单 | 1 |
| `tools/measure-spike.ps1` | spike 实测（冷启动/内存/无控制台/DPI/中文/截图） | 1 |
| `spike/` | egui + glow 最小工程（`--report` / `--hold` / `--child` / `--elevate` / `--font` / `--no-font`） | 1 crate |
| `docs/m0-spike-window.png` | 中文 + 150% DPI 实测截图 | 1 |

## 3. 门禁判定

| 门禁项 | 目标 | 实测 | 判定 |
|---|---|---|---|
| 样本与现有 PS 产物一致 | 逐字节 | 诊断 INI 12 份、模板 8 份、识别 8 条全部由真实 PS 函数产出；journal 5 阶段 | ✅（真实软链接样本缺，见 §6） |
| GUI 常驻内存 | ≤60 MB | **83.8–84.0 MB**（msyh）/ 74.3 MB（simhei）/ 65.2 MB（无字体） | ❌ |
| 引擎常驻内存 | ≤10 MB | 引擎尚未实现 | ⏳ |
| 冷启动到首帧 | ≤300 ms | **138.5–159.9 ms** | ✅ |
| 二进制体积 | ≤12 MB | 3.84 MB（动态 CRT）/ **4.00 MB**（静态 CRT） | ✅ |
| 无控制台窗口 | 必须 | `GetConsoleWindow()=0`；`--child` 子进程同样为 0 | ✅ |
| PerMonitorV2 + 150% DPI | 正常 | `SetProcessDpiAwarenessContext(-4)` 成功，`pixels_per_point=1.500`，无模糊 | ✅ |
| 中文渲染 | 必须 | 载入 `msyh.ttc`（19,704,352 B），截图无豆腐块 | ✅ |
| UAC 提权隐藏子进程 | 端到端 | 代码路径已写出，**未执行**（会弹 UAC） | ⏳ |

## 4. spike 实测明细

三次独立运行，`--hold 4000`，每 350 ms 采样一次 WorkingSet（均取稳定后的值）：

| 配置 | 字体文件 | 冷启动 | 首帧 WS | 稳定 WS | 稳定 Private |
|---|---|---:|---:|---:|---:|
| `msyh.ttc`（默认） | 18.8 MB | 145–160 ms | 76.9 MB | **83.8–84.0 MB** | 92.2–93.2 MB |
| `simhei.ttf` | 9.3 MB | 147.4 ms | 67.4 MB | **74.3 MB** | — |
| `--no-font` | — | 138.5 ms | 58.1 MB | **65.2 MB** | — |

- 字体开销 ≈ 字体文件大小（18.8 MB → +18.7 MB；9.3 MB → +9.1 MB），因为 egui 在 `FontDefinitions` 里持有整份字节以支持重建图集。
  > 2026-11-24 后续：字体改为**只读内存映射**（`launcher-win/src/fontmap.rs`），这笔开销不再计入专用内存，见 [M4-REPORT §6](M4-REPORT.md)。
- 去掉字体后仍有 **65.2 MB**：这部分是 egui + glow + NVIDIA OpenGL 驱动（已确认加载 `nvoglv64.dll`，**不是**软件渲染）+ 1350×870 物理像素的双缓冲交换链。
- 现有基线（旧实测）：空 PowerShell 63.0 MB、PowerShell + WinForms **75.9 MB**。

**结论：egui + glow 无法兑现计划 §1.1 目标 5「内存开销显著低于基线」。**
- 用 `msyh` 时比 WinForms 基线高约 8 MB；
- 用 `simhei` 时与基线基本持平（低 1.6 MB）；
- 即使不加载任何中文字体也已经 65.2 MB，**≤60 MB 在这个技术栈上不可达**。

## 5. 可选路径

| 方案 | 做法 | 预期常驻 | 代价 |
|---|---|---|---|
| **A · 接受现状** | 目标改为「交互期 GUI ≤85 MB（含中文字体）」；文档删掉"显著低于基线"的表述 | 84 MB | 0 |
| **B · 换小字体** | 用 `simhei.ttf`（9.3 MB），目标 ≤80 MB | 74 MB | 字形不如雅黑；不能自制子集随包（字体授权） |
| **C · 换 UI 技术栈** | Direct2D + DirectWrite（`windows-rs` 自绘） | 预期 20–35 MB（未实测） | M4 从 13 天涨到 21–25 天；布局/输入法/无障碍全部重做 |
| **D · 界面不全时驻留** | 交接给引擎后 GUI 退出（与现有 PS 行为一致），会话期只有引擎；重开界面可接管 | 交互期 84 MB + 会话期约 0 | 与"启动后最小化到托盘"冲突，Phase 4 托盘项要改 |
| **E · A+D 组合（建议）** | 交互期按 A 接受；会话开始后界面退出，常驻只剩引擎（目标 ≤10 MB，M2 实测） | 同上 | 同上；`rust开发.md` §1.1 目标 2/5 与 §5 Phase 4 需同步改 |

- C 是唯一能把常驻压到 40 MB 以下的路径，但它把交付风险从"复刻行为"转移到"重写 UI"。
- 若采纳 E，建议把 §9 的性能表改为：引擎 ≤10 MB（待 M2）、交互期 GUI ≤85 MB、会话期常驻 = 引擎、冷启动 ≤300 ms、二进制 ≤12 MB。

## 6. 未验证项

1. **UAC 提权隐藏子进程**：`spike --elevate <输出路径>` 未在自动化里执行（会弹 UAC 且无人确认）。手工命令：
   `rust\spike\target\release\spike.exe --elevate %TEMP%\elev.json`，出现 UAC 后确认，检查 `%TEMP%\elev.json` 是否写入 `console_window=0`。
2. **真实软链接**：当前会话无 `SeCreateSymbolicLinkPrivilege`；`fixtures/journal/06` 用"记录为 done + 普通文件"模拟归档分支。以管理员身份重跑 `gen-fixtures.ps1` 可补上。
3. **引擎内存**：引擎尚未实现，`≤10 MB` 无从实测；纯 Rust 无 GUI 进程预期 2–4 MB，但需 M2 实测。
4. **4K/150% 缩放对交换链内存的影响**：窗口 1350×870 物理像素；换更小窗口是否显著降低基线未测（可作为 M0.1 的一条命令）。这不改变"≤60 MB 不可达"的结论，但影响 65 MB 里驱动占比的判断。

## 7. 下一步

1. 等你定 §5 的方案（A / B / C / D / E）。
2. 定了之后我改 `rust开发.md` §1.1 目标、§5 Phase 4 与 §9 性能表，把 M0 的实测结论写回计划。
3. M1（core）与 UI 选型解耦，**不阻塞**：可以先做 `config`/`journal`/`resources`/`restore`，UI 到 M4 再落地。
