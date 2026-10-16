# 由异环启动脚本加载；密钥对应已验证的国服配置样本。
function Convert-NteEngineConfig {
    param([string]$OriginalPath, [string]$SourcePath)
    $aes = [Security.Cryptography.Aes]::Create()
    $aes.Key = [Text.Encoding]::ASCII.GetBytes('1zh6IOlIohrR88UNPjiLisrkWACUQYuz')
    $aes.Mode = 'ECB'
    $aes.Padding = 'PKCS7'
    try {
        $plain = [Collections.Generic.List[string]]::new()
        if (Test-Path -LiteralPath $OriginalPath -PathType Leaf) {
            foreach ($line in [IO.File]::ReadAllLines($OriginalPath)) {
                if (-not $line.Trim()) { continue }
                $bytes = [Convert]::FromBase64String($line)
                $transform = $aes.CreateDecryptor()
                try { $decoded = [Text.UTF8Encoding]::new($false, $true).GetString($transform.TransformFinalBlock($bytes, 0, $bytes.Length)) }
                finally { $transform.Dispose() }
                if (-not $decoded.EndsWith('|SPLIT|')) { throw '原 Engine.ini 加密格式不符，停止部署。' }
                foreach ($entry in ($decoded -split '\|SPLIT\|')) {
                    if ($entry.Trim()) { $plain.Add($entry) }
                }
            }
        }
        $sectionFound = $false
        foreach ($line in [IO.File]::ReadAllLines($SourcePath)) {
            $entry = $line.Trim()
            if (-not $entry -or $entry.StartsWith(';')) { continue }
            if ($entry -match '^\[[^\]]+\]$') { $sectionFound = $true }
            elseif (-not $sectionFound -or $entry -notmatch '^[^=]+=') { throw "配置源不是有效的明文INI：$entry" }
            $plain.Add($entry)
        }
        if (-not $sectionFound) { throw '配置源没有有效节，停止部署。' }
        # 原内容在前，用户配置在后，保留原配置并覆盖同名键。
        foreach ($entry in $plain) {
            $payload = [Text.Encoding]::UTF8.GetBytes($entry + '|SPLIT|')
            $transform = $aes.CreateEncryptor()
            try { $cipher = $transform.TransformFinalBlock($payload, 0, $payload.Length) }
            finally { $transform.Dispose() }
            $transform = $aes.CreateDecryptor()
            try { $roundtrip = [Text.Encoding]::UTF8.GetString($transform.TransformFinalBlock($cipher, 0, $cipher.Length)) }
            finally { $transform.Dispose() }
            if ($roundtrip -cne ($entry + '|SPLIT|')) { throw '加密回读校验失败。' }
            [Convert]::ToBase64String($cipher)
        }
    }
    finally { $aes.Dispose() }
}

function Assert-NteEnginePaths {
    foreach ($path in @($engineConfigPath, $engineRuntimePath, $engineBackupPath)) {
        if ((Get-NormalizedPath $engineConfigSource) -eq (Get-NormalizedPath $path)) {
            throw '配置源不能使用游戏原文件、临时文件或备份路径。'
        }
    }
}
