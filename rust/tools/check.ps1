#Requires -Version 5.1
<#
.SYNOPSIS
    launcher-core 的统一检查：格式化、clippy、全量测试（可选真实软链接用例）。

.DESCRIPTION
    一个命令跑完 Phase 1 的门：`cargo fmt --check`、`cargo clippy -D warnings`、`cargo test`。
    任何一步失败就返回非零，便于脚本或 CI 直接使用。

    cargo 会把进度写到 stderr，PowerShell 5.1 会把它包成 NativeCommandError；
    这里统一把两个流合并成字符串，输出干净、退出码仍然准确。

.PARAMETER IncludeIgnored
    额外运行真实软链接用例。**必须在管理员终端里跑**：设了该开关后，
    用例不再允许"无权限静默跳过"，拿不到创建符号链接的特权就算失败。

.EXAMPLE
    # 普通检查
    powershell -NoProfile -ExecutionPolicy Bypass -File rust\tools\check.ps1

.EXAMPLE
    # 管理员终端里跑，含真实软链接
    powershell -NoProfile -ExecutionPolicy Bypass -File rust\tools\check.ps1 -IncludeIgnored
#>
[CmdletBinding()]
param(
    [switch]$IncludeIgnored
)

$ErrorActionPreference = 'Continue'
# cargo / Rust 测试输出是 UTF-8；不设这一行，中文在 GBK 控制台里会变乱码。
try { [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false) } catch { }

$toolsDir = $PSScriptRoot
$rustRoot = Split-Path -Parent $toolsDir

function Invoke-Cargo {
    param([string[]]$Arguments)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'SilentlyContinue'
    try {
        # 2>&1 后逐项字符串化：既拿到 stderr 文本，又不让它变成 NativeCommandError。
        $text = (& cargo @Arguments 2>&1 |
            ForEach-Object {
                if ($_ -is [System.Management.Automation.ErrorRecord]) { $_.ToString() } else { "$_" }
            } | Out-String)
        return [pscustomobject]@{
            ExitCode = $LASTEXITCODE
            Text = $text
        }
    }
    finally {
        $ErrorActionPreference = $previous
    }
}

Push-Location $rustRoot
try {
    $failed = [Collections.Generic.List[string]]::new()

    Write-Host '=== cargo fmt --check ==='
    $fmt = Invoke-Cargo @('fmt', '--all', '--', '--check')
    if ($fmt.ExitCode -ne 0) {
        Write-Host $fmt.Text
        $failed.Add('fmt')
    }
    else {
        Write-Host '格式化检查通过。'
    }

    Write-Host ''
    Write-Host '=== cargo clippy -D warnings ==='
    $clippy = Invoke-Cargo @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')
    if ($clippy.ExitCode -ne 0) {
        Write-Host $clippy.Text
        $failed.Add('clippy')
    }
    else {
        Write-Host 'clippy 无告警。'
    }

    Write-Host ''
    Write-Host '=== cargo test ==='
    $test = Invoke-Cargo @('test', '--workspace')
    ($test.Text -split "`r?`n") |
        Where-Object { $_ -match '^test result|^error' } |
        ForEach-Object { Write-Host $_ }
    if ($test.ExitCode -ne 0) {
        Write-Host $test.Text
        $failed.Add('test')
    }

    if ($IncludeIgnored) {
        Write-Host ''
        Write-Host '=== cargo test --test real_symlinks -- --ignored（要求真实创建软链接）==='
        $env:DLSSG_REQUIRE_SYMLINK_TESTS = '1'
        try {
            $ignored = Invoke-Cargo @('test', '--test', 'real_symlinks', '--', '--ignored', '--nocapture')
        }
        finally {
            Remove-Item Env:DLSSG_REQUIRE_SYMLINK_TESTS -ErrorAction SilentlyContinue
        }
        (($ignored.Text -split "`r?`n") | Where-Object { $_ -match '^test |^test result|^error|panicked|跳过' }) |
            ForEach-Object { Write-Host $_ }
        if ($ignored.ExitCode -ne 0) {
            $failed.Add('real-symlinks')
        }
    }

    Write-Host ''
    if ($failed.Count -gt 0) {
        Write-Host ('失败：' + ($failed -join '、'))
        if ($failed -contains 'real-symlinks') {
            Write-Host '真实软链接用例需要 SeCreateSymbolicLinkPrivilege：请用管理员终端重跑本脚本。'
        }
        exit 1
    }
    Write-Host '全部通过。'
}
finally {
    Pop-Location
}
