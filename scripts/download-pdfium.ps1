# 下载 bblanchon/pdfium-binaries 的 Windows 动态库到 src-tauri/resources/，供打包使用。
# 用法: powershell -ExecutionPolicy Bypass -File scripts\download-pdfium.ps1 [-Version 7961]
param(
    [string]$Version = "7961"
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Dest = Join-Path $ProjectRoot "src-tauri\resources"
New-Item -ItemType Directory -Force -Path $Dest | Out-Null

$Base = "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F$Version"
$Archive = "pdfium-win-x64.tgz"
$Mirrors = @("https://gh-proxy.com/", "https://ghfast.top/", "")

$Tgz = Join-Path $env:TEMP $Archive
$Downloaded = $false
foreach ($m in $Mirrors) {
    $Url = "$m$Base/$Archive"
    Write-Host ">> 尝试: $Url"
    try {
        Invoke-WebRequest -Uri $Url -OutFile $Tgz -UseBasicParsing -TimeoutSec 600
        $Downloaded = $true
        break
    } catch {
        Write-Host "   失败: $($_.Exception.Message)"
    }
}
if (-not $Downloaded) { throw "下载失败: $Archive（请检查网络或更换镜像）" }

# tgz 解压（Windows 自带 tar）
$ExtractDir = Join-Path $env:TEMP "pdfium-extract"
if (Test-Path $ExtractDir) { Remove-Item -Recurse -Force $ExtractDir }
New-Item -ItemType Directory -Force -Path $ExtractDir | Out-Null
tar -xzf $Tgz -C $ExtractDir bin/pdfium.dll
Copy-Item (Join-Path $ExtractDir "bin\pdfium.dll") (Join-Path $Dest "pdfium.dll") -Force

Write-Host "== 完成: $(Join-Path $Dest 'pdfium.dll')"
Get-ChildItem $Dest
