<#
.SYNOPSIS
  Builds IPTV-Setup-<version>.exe: a single installer containing iptv.exe and mpv.

.PARAMETER MpvDir
  Folder containing the mpv.exe to bundle (default: the winget shinchiro.mpv
  install location).

.PARAMETER Iscc
  Path to Inno Setup's ISCC.exe (default: searched in PATH and usual install dirs).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
#>
param(
    [string]$MpvDir = "C:\Program Files\MPV Player",
    [string]$Iscc
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$here = $PSScriptRoot

# --- version from the workspace Cargo.toml
$m = Select-String -Path "$root\Cargo.toml" -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $m) { throw "could not read version from Cargo.toml" }
$version = $m.Matches[0].Groups[1].Value

# --- tools
if (-not $Iscc) {
    $Iscc = @(
        (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $Iscc) { throw "Inno Setup not found. Install it with: winget install JRSoftware.InnoSetup" }
$mpvExe = Join-Path $MpvDir 'mpv.exe'
if (-not (Test-Path $mpvExe)) { throw "mpv.exe not found in $MpvDir (install with: winget install shinchiro.mpv, or pass -MpvDir)" }

# --- build
Write-Host "Building iptv $version (release)..." -ForegroundColor Cyan
cargo build --release --manifest-path "$root\Cargo.toml" -p iptv-desktop
if ($LASTEXITCODE) { throw "cargo build failed" }

# --- stage files exactly as they will be installed
$out = "$root\target\installer"
$stage = "$out\stage"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force "$stage\mpv", "$stage\licenses" | Out-Null

Copy-Item "$root\target\release\iptv.exe" $stage
Copy-Item "$root\LICENSE" "$stage\LICENSE.txt"
Copy-Item $mpvExe "$stage\mpv"
Get-ChildItem $MpvDir -Filter *.dll | Copy-Item -Destination "$stage\mpv"
Copy-Item "$here\licenses\*" "$stage\licenses"

$mpvVersion = (& $mpvExe --version | Select-Object -First 1)
(Get-Content "$here\THIRD-PARTY-NOTICES.txt" -Raw).Replace('@MPV_VERSION@', $mpvVersion) |
    Set-Content "$stage\THIRD-PARTY-NOTICES.txt" -Encoding utf8

# --- compile the installer
Write-Host "Compiling installer with $Iscc ..." -ForegroundColor Cyan
& $Iscc /Q "/DAppVersion=$version" "/DStageDir=$stage" "/DOutputDir=$out" "$here\iptv.iss"
if ($LASTEXITCODE) { throw "ISCC failed" }

$setup = Get-Item "$out\IPTV-Setup-$version.exe"
Write-Host ("Done: {0} ({1:N1} MB)" -f $setup.FullName, ($setup.Length / 1MB)) -ForegroundColor Green
