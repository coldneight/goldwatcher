# 金瞳 GoldWatcher — Windows 当前用户安装脚本
# 在 PowerShell 中运行：powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root "target" }
$Exe = Join-Path $TargetDir "release\gold-watcher.exe"
$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\GoldWatcher"
$InstalledExe = Join-Path $InstallDir "GoldWatcher.exe"

Set-Location $Root
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "未找到 cargo。请先安装 Rust：https://rustup.rs"
}
if (-not (Test-Path $Exe)) {
    Write-Host "==> 构建 release 版本"
    cargo build --release --target-dir $TargetDir
}
if (-not (Test-Path $Exe)) { throw "构建失败：找不到 $Exe" }

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path $StartMenu | Out-Null
Copy-Item -Force $Exe $InstalledExe

$Shell = New-Object -ComObject WScript.Shell
$Desktop = [Environment]::GetFolderPath("Desktop")
$StartMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"

function New-Shortcut([string]$Path) {
    $Shortcut = $Shell.CreateShortcut($Path)
    $Shortcut.TargetPath = $InstalledExe
    $Shortcut.WorkingDirectory = $InstallDir
    $Shortcut.IconLocation = "$InstalledExe,0"
    $Shortcut.Description = "金瞳 GoldWatcher - 全球黄金监控"
    $Shortcut.Save()
}

New-Shortcut (Join-Path $StartMenu "金瞳 GoldWatcher.lnk")
New-Shortcut (Join-Path $Desktop "金瞳 GoldWatcher.lnk")

Write-Host "已安装：$InstalledExe"
Write-Host "已创建开始菜单和桌面快捷方式。"
Write-Host "可在应用设置中开启开机自动启动。"
