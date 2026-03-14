$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$target = if ($env:TARGET) { $env:TARGET } else { 'x86_64-pc-windows-msvc' }
$dist = Join-Path $root "dist/windows/$target"
$bin = Join-Path $root "target/$target/release/dux-ai-node-tray.exe"

if ($env:OS -ne 'Windows_NT') {
  Write-Error "Please run this script on a Windows machine with MSVC build tools installed."
}

cargo build --release -p dux-ai-node-tray --target $target
if (Test-Path $dist) {
  Remove-Item -Recurse -Force $dist
}
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item $bin (Join-Path $dist 'dux-ai-node-tray.exe') -Force
New-Item -ItemType Directory -Force -Path (Join-Path $dist 'web') | Out-Null
Copy-Item (Join-Path $root 'web/settings') (Join-Path $dist 'web/settings') -Recurse -Force
$zip = Join-Path $root "dist/windows/dux-ai-node-$target.zip"
if (Test-Path $zip) {
  Remove-Item -Force $zip
}
Compress-Archive -Path (Join-Path $dist '*') -DestinationPath $zip
Write-Host "Built Windows tray binary: $(Join-Path $dist 'dux-ai-node-tray.exe')"
