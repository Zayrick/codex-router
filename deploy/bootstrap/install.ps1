# Requires Windows PowerShell 5.1 or PowerShell 7 on Windows.
& {
    $ErrorActionPreference = 'Stop'
    $serviceUrl = '@@PUBLIC_URL@@'
    $expectedHash = '@@CLIENT_SHA256@@'
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $workDir = Join-Path ([IO.Path]::GetTempPath()) ('codex-bootstrap-' + [Guid]::NewGuid().ToString('N'))
    $stagedPaths = @()
    try {
        if (-not $env:USERPROFILE) { throw '请在 Windows 中运行此脚本，并确保 USERPROFILE 环境变量已设置。' }
        $name = (Read-Host '请输入你的名字').Trim()
        if ([string]::IsNullOrWhiteSpace($name)) { throw '名字不能为空。' }
        $codexDir = Join-Path $env:USERPROFILE '.codex'
        if ($env:CODEX_HOME -and [IO.Path]::GetFullPath($env:CODEX_HOME).TrimEnd('\', '/') -ne [IO.Path]::GetFullPath($codexDir).TrimEnd('\', '/')) {
            throw 'CODEX_HOME 指向了其他目录。请先清除此环境变量，再安装到 %USERPROFILE%\.codex。'
        }
        New-Item -ItemType Directory -Path $workDir -Force | Out-Null
        New-Item -ItemType Directory -Path $codexDir -Force | Out-Null
        $helper = Join-Path $workDir 'codex-config-edit.exe'
        Invoke-WebRequest -UseBasicParsing -Uri "$serviceUrl/codex-config-edit.exe" -OutFile $helper
        $hasher = [Security.Cryptography.SHA256]::Create()
        $stream = [IO.File]::OpenRead($helper)
        try {
            $actualHash = [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-', '')
        } finally {
            $stream.Dispose()
            $hasher.Dispose()
        }
        if ($actualHash -ne $expectedHash) { throw '下载的配置助手校验失败，请重新运行安装脚本。' }

        $authPath = Join-Path $codexDir 'auth.json'
        $configPath = Join-Path $codexDir 'config.toml'
        $suffix = [Guid]::NewGuid().ToString('N')
        $configStage = Join-Path $codexDir ('.config-' + $suffix + '.tmp')
        $authStage = Join-Path $codexDir ('.auth-' + $suffix + '.tmp')
        $stagedPaths = @($configStage, $authStage)
        $requestBody = $utf8.GetBytes((@{ name = $name } | ConvertTo-Json -Compress))
        $result = Invoke-RestMethod -Method Post -Uri "$serviceUrl/register" -ContentType 'application/json; charset=utf-8' -Body $requestBody
        if ($result.api_key -notmatch '^sk-[a-f0-9]{64}$' -or -not $result.base_url) { throw '服务器返回的注册信息无效，请联系管理员。' }
        & $helper $configPath $configStage ([string]$result.base_url)
        if ($LASTEXITCODE -ne 0) { throw '准备 config.toml 失败，请根据上方提示处理后重试。原有文件未修改。' }

        Write-Host '正在等待代理服务器启用你的 API key……'
        $ready = $false
        for ($attempt = 0; $attempt -lt 30; $attempt++) {
            try {
                $null = Invoke-RestMethod -Uri ($result.base_url.TrimEnd('/') + '/models') -Headers @{ Authorization = 'Bearer ' + $result.api_key } -TimeoutSec 10
                $ready = $true
                break
            } catch {
                if ($attempt -lt 29) { Start-Sleep -Milliseconds 300 }
            }
        }
        if (-not $ready) { throw '代理服务器尚未接受此 API key，请检查服务器状态后重试。' }
        $auth = [ordered]@{ auth_mode = 'apikey'; OPENAI_API_KEY = [string]$result.api_key } | ConvertTo-Json
        [IO.File]::WriteAllText($authStage, $auth + "`n", $utf8)

        $timestamp = Get-Date -Format 'yyyyMMdd-HHmmss-fffffff'
        $files = @(
            @{ Path = $authPath; Stage = $authStage; Backup = $null; Written = $false },
            @{ Path = $configPath; Stage = $configStage; Backup = $null; Written = $false }
        )
        foreach ($file in $files) {
            if (Test-Path -LiteralPath $file.Path -PathType Leaf) {
                $file.Backup = $file.Path + '.bak-' + $timestamp
                [IO.File]::Copy($file.Path, $file.Backup, $false)
            }
        }
        try {
            foreach ($file in $files) {
                if ($file.Backup) {
                    [IO.File]::Replace($file.Stage, $file.Path, [NullString]::Value)
                } else {
                    [IO.File]::Move($file.Stage, $file.Path)
                }
                $file.Written = $true
            }
        } catch {
            foreach ($file in $files) {
                if ($file.Written) {
                    if ($file.Backup) { [IO.File]::Copy($file.Backup, $file.Path, $true) }
                    else { [IO.File]::Delete($file.Path) }
                }
            }
            throw
        }
        Write-Host "已为 $name 完成 Codex 配置。" -ForegroundColor Green
        Write-Host ('API 地址：' + $result.base_url)
        foreach ($file in $files) { if ($file.Backup) { Write-Host ('原文件备份：' + $file.Backup) } }
        Write-Host '请重启 ChatGPT 客户端 / Codex，使新配置生效。' -ForegroundColor Yellow
    } finally {
        foreach ($path in $stagedPaths) { if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force } }
        if (Test-Path -LiteralPath $workDir) {
            # Only remove the unique temporary directory created by this invocation.
            $resolved = [IO.Path]::GetFullPath($workDir)
            $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
            if ($resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved).StartsWith('codex-bootstrap-')) {
                Remove-Item -LiteralPath $resolved -Recurse -Force
            }
        }
    }
}
