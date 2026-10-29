# 迁移到新设备时，主要修改本文件。请用 UTF-8 BOM 保存。
# 路径可写绝对路径或相对于“启动脚本”的路径，支持 %LOCALAPPDATA% 等环境变量。
# 游戏目录指包含 Client 子目录的位置；LauncherPath 填官方启动器 EXE 的完整路径。
# 显式命令行参数优先于本文件；不要将部署记录或设备状态备份作为新设备配置复制使用。
@{
    Wuthering = @{
        GameDirectory = 'C:\Games\Wuthering Waves Game'
        LauncherPath = 'C:\Program Files\Wuthering Waves\launcher.exe'
        # 鸣潮明文画面配置源：迁移时同时复制该源文件并填写新位置。
        EngineConfigSource = 'D:\Games\WuWa-Configs\当前配置\Engine.ini'
        # 留空：从游戏 Saved 目录查找；多个候选时需要明确填写。
        ConfigDirectory = ''
        RestartGraceSeconds = 10
    }
    Nte = @{
        GameDirectory = 'C:\Games\Neverness To Everness'
        LauncherPath = 'C:\Games\Neverness To Everness\NTELauncher.exe'
        EngineConfigSource = '异环配置\Engine.ini'
        # 留空：使用当前用户的 %LOCALAPPDATA%\HT\Saved\Config\Windows。
        ConfigDirectory = ''
    }
    Gpu = @{
        # 留空：仅有一张当前存在的 NVIDIA 显卡时自动选择。
        # 多张时按错误信息填写 PCI\VEN_10DE&... 设备实例标识（不是 HKLM 路径）。
        DeviceInstanceId = ''
        TargetName = 'NVIDIA GeForce RTX 5070'
    }
}
