# Zips the built Windows programs: texcopy.exe (command line) and texcopy-gui.exe (the window, no console).
#   powershell -File packaging/windows/make-zip.ps1 [-Out dist] [-Version 0.1.0]
param(
  [string]$Out = "dist",
  [string]$Version = ""
)
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
if (-not $Version) {
  $line = Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version = "(.*)"' | Select-Object -First 1
  $Version = $line.Matches[0].Groups[1].Value
}
$name = "texcopy-$Version-windows-x64"
$stage = Join-Path $Out $name
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
foreach ($f in @("target/release/texcopy.exe", "target/release/texcopy-gui.exe", "README.md", "LICENSE")) {
  Copy-Item (Join-Path $root $f) $stage
}
$zip = Join-Path $Out "$name.zip"
Remove-Item -Force $zip -ErrorAction SilentlyContinue
Compress-Archive -Path $stage -DestinationPath $zip
Remove-Item -Recurse -Force $stage
Write-Host "built $zip"
