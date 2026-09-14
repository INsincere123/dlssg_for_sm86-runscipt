# 备用代理 DLL（alternatives/）

代理 DLL 靠"游戏会加载一个和它同名的系统 DLL"来进入进程。默认的 `version.dll` 放在发布根目录，是首选。如果目标游戏不导入 `version.dll`，就从本目录挑一个游戏确实会加载的名字，放到渲染 EXE 旁边。

**任何时候只启用一个代理。** 六个名字内嵌的运行库、后端和 `dlssg_sm86.ini` 完全相同，区别只在于文件名和转发的目标系统 DLL。每个代理都会把自己全部导出转发给 `C:\Windows\System32\` 里的同名真实 DLL，只拦截 `nvngx_dlssg.dll` 的加载——所以换名字不改变行为。

## 工具类代理（最安全，优先用）

这些 DLL 不在 D3D12 渲染热路径上，转发开销可以忽略：

| 名字 | 放置位置 | 说明 |
|---|---|---|
| `version.dll` | 发布根目录 | 首选。绝大多数游戏都会加载 `version.dll`。 |
| `alternatives/winmm.dll` | EXE 旁 | 第二选择。多媒体计时 API，几乎所有游戏都导入。 |
| `alternatives/dbghelp.dll` | EXE 旁 | 崩溃/符号处理库。游戏或反作弊常加载。 |
| `alternatives/dinput8.dll` | EXE 旁 | DirectInput8。老一些的输入栈会加载。 |

## 渲染路径代理（可用，但风险更高）

`dxgi.dll` 和 `d3d12.dll` 是 D3D12 渲染管线本身的入口，游戏每帧都密集调用它们，而且加载顺序敏感（游戏可能在我们的代理就位之前就已按系统路径解析了真实 DLL）。转发是完整的，功能正确，但只有在 `version.dll` / `winmm.dll` 都无法被目标加载时才建议使用：

| 名字 | 放置位置 | 说明 |
|---|---|---|
| `alternatives/dxgi.dll` | EXE 旁 | 仅当上面几个都不行时使用。 |
| `alternatives/d3d12.dll` | EXE 旁 | 同上；与 `dxgi.dll` 二选一，不要同时放。 |

## 使用步骤

1. 从上表选一个名字，把对应 DLL 复制到渲染 EXE 所在目录（`version.dll` 用根目录那份，其余用 `alternatives/` 里的那份）。
2. 把 `dlssg_sm86.ini` 复制到同一目录。
3. 确认同目录没有第二个本项目的代理 DLL。
4. 启动游戏。日志（`dlssg_sm86\logs\loader_*.jsonl`）里出现 `runtime_redirect` 即代理已生效。

签名与信任见 `docs/SIGNING.md`。完整安装说明见根目录 `README.md`（即 `docs/INSTALL.md`）。
