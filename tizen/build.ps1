<#
.SYNOPSIS
  Builds the Samsung TV app: compiles the Rust core to WebAssembly and
  assembles the web app in target\tizen\app. With -Profile, also packages
  (and signs) it as target\tizen\IPTV.wgt.

.PARAMETER Profile
  Tizen security profile (certificate) to sign with, created in Tizen's
  Certificate Manager. Without it, only the app folder is assembled.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tizen\build.ps1 -Profile mytv
#>
param([string]$Profile)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot\..").Path
$out = "$root\target\tizen"
$app = "$out\app"

# TV browsers are old Chromium versions (2019 models: Chromium 63). Stable
# Rust's prebuilt standard library uses newer WebAssembly features
# (reference types, Chromium 96+), so the standard library is rebuilt for the
# original "MVP" feature set with nightly Rust. Requires:
#   rustup toolchain install nightly --component rust-src --target wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.130
#   binaryen (wasm-opt) on PATH or in %USERPROFILE%\.local\binaryen-*\bin
Write-Host "Compiling the Rust core to WebAssembly (MVP features)..." -ForegroundColor Cyan
$wasmTarget = "$root\target\wasm-mvp"
$env:RUSTFLAGS = "-C target-cpu=mvp"
$env:CARGO_PROFILE_RELEASE_STRIP = "false"   # keep the feature section for wasm-bindgen
cargo +nightly build -Z build-std=std,panic_abort --release --target wasm32-unknown-unknown `
    -p iptv-web --manifest-path "$root\Cargo.toml" --target-dir $wasmTarget
$code = $LASTEXITCODE
Remove-Item Env:RUSTFLAGS, Env:CARGO_PROFILE_RELEASE_STRIP
if ($code) { throw "cargo build failed" }

Write-Host "Generating JavaScript bindings..." -ForegroundColor Cyan
$gen = "$out\wasm-bindgen"
wasm-bindgen --target no-modules --no-typescript --out-dir $gen "$wasmTarget\wasm32-unknown-unknown\release\iptv_web.wasm"
if ($LASTEXITCODE) { throw "wasm-bindgen failed (install it with: cargo install wasm-bindgen-cli --version 0.2.130)" }

# Validate that only MVP features are used (fails otherwise) and shrink it.
$wasmOpt = (Get-Command wasm-opt -ErrorAction SilentlyContinue).Source
if (-not $wasmOpt) {
    $wasmOpt = Get-ChildItem "$env:USERPROFILE\.local\binaryen-*\bin\wasm-opt.exe" -ErrorAction SilentlyContinue |
        Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
}
if (-not $wasmOpt) { throw "wasm-opt not found: install binaryen (https://github.com/WebAssembly/binaryen/releases)" }
& $wasmOpt "$gen\iptv_web_bg.wasm" --mvp-features -Oz -o "$gen\iptv_web_bg.wasm"
if ($LASTEXITCODE) { throw "wasm-opt failed: the module uses WebAssembly features older TVs lack" }

Write-Host "Assembling the app..." -ForegroundColor Cyan
if (Test-Path $app) { Remove-Item $app -Recurse -Force }
Copy-Item "$PSScriptRoot\app" $app -Recurse
# Keep config.xml's version in step with Cargo.toml.
$m = Select-String -Path "$root\Cargo.toml" -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
$version = $m.Matches[0].Groups[1].Value
$config = Get-Content "$app\config.xml" -Raw
$config = $config -replace 'version="[0-9.]+" viewmodes', "version=""$version"" viewmodes"
Set-Content "$app\config.xml" $config -Encoding utf8 -NoNewline
Copy-Item "$gen\iptv_web.js" "$app\js\core.js"
# The TV loads the app from local files, where fetching a .wasm file isn't
# reliable, so the module is embedded in a script as base64.
$b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes("$gen\iptv_web_bg.wasm"))
Set-Content "$app\js\core_wasm.js" "window.IPTV_WASM_B64 = '$b64';" -Encoding ascii -NoNewline
Copy-Item "$root\LICENSE" "$app\LICENSE.txt"
Write-Host "App folder: $app (version $version)"

if ($Profile) {
    $tizen = "C:\tizen-studio\tools\ide\bin\tizen.bat"
    if (-not (Test-Path $tizen)) { throw "Tizen Studio CLI not found at $tizen" }
    Write-Host "Packaging and signing with profile '$Profile'..." -ForegroundColor Cyan
    Remove-Item "$app\*.wgt" -ErrorAction SilentlyContinue
    & $tizen package -t wgt -s $Profile -- $app
    if ($LASTEXITCODE) { throw "tizen package failed" }
    $wgt = Get-ChildItem "$app\*.wgt" | Select-Object -First 1
    Move-Item $wgt.FullName "$out\IPTV.wgt" -Force
    Write-Host "Package: $out\IPTV.wgt" -ForegroundColor Green
}
