$ErrorActionPreference = "Stop"

$ROOT_DIR = Split-Path -Parent $PSScriptRoot
$BIN_DIR = Join-Path $ROOT_DIR "bin"
New-Item -ItemType Directory -Force -Path $BIN_DIR | Out-Null

Write-Host "=== [1/5] Generating Lime + AMOLED Black PC & Phone Icons ==="
python (Join-Path $ROOT_DIR "assets\generate_icons.py")
if ($LASTEXITCODE -ne 0) { throw "Icon generation failed" }

Push-Location $ROOT_DIR
try {
    Write-Host "=== [2/5] Running Rust Protocol Unit Tests ==="
    cargo test -p i_connect_proto --offline
    if ($LASTEXITCODE -ne 0) { throw "Protocol tests failed" }

    Write-Host "=== [3/5] Building Windows PC Server & Uninstaller (Release) ==="
    cargo build --release --offline -p i_connect_server -p i_connect_uninstaller
    if ($LASTEXITCODE -ne 0) { throw "Server/Uninstaller build failed" }

    Copy-Item (Join-Path $ROOT_DIR "target\release\i_connect_server.exe") (Join-Path $BIN_DIR "I_Connect_Server.exe") -Force
    Copy-Item (Join-Path $ROOT_DIR "target\release\i_connect_uninstaller.exe") (Join-Path $BIN_DIR "uninstall.exe") -Force

    Write-Host "=== [4/5] Building Native Self-Extracting Windows Installer ==="
    cargo build --release --offline -p i_connect_installer
    if ($LASTEXITCODE -ne 0) { throw "Installer build failed" }

    Copy-Item (Join-Path $ROOT_DIR "target\release\i_connect_installer.exe") (Join-Path $BIN_DIR "I_Connect_Installer.exe") -Force
} finally {
    Pop-Location
}

Write-Host "=== [5/5] Building Rust Android Core & Signed APK ==="
& (Join-Path $PSScriptRoot "build_apk.ps1")
if ($LASTEXITCODE -ne 0) { throw "APK build failed" }

Write-Host ""
Write-Host "=========================================================="
Write-Host " I CONNECT - ALL RELEASE ARTIFACTS BUILT SUCCESSFULLY!"
Write-Host "=========================================================="
Get-ChildItem $BIN_DIR | Select-Object Name, Length, LastWriteTime
