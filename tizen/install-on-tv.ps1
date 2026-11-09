<#
.SYNOPSIS
  Installs the IPTV app on a Samsung TV over Wi-Fi.

.DESCRIPTION
  Samsung TVs only accept apps from the Samsung store, or - in Developer
  Mode - apps signed with a Samsung certificate that lists that TV. This
  script signs the IPTV app with your certificate and installs it.

  One-time preparation (see README, "Samsung TVs"):
    1. Install Tizen Studio (command-line tools) with the "Samsung Certificate
       Extension" and "TV Extensions Tools".
    2. Put the TV in Developer Mode with this PC's IP address.
    3. Create a Samsung TV certificate in Tizen's Certificate Manager while
       this script has the TV connected (it shows the TV's ID / DUID).

.PARAMETER TvIp
  The TV's IP address (TV: Settings > General > Network > Network Status).

.PARAMETER Profile
  Name of your certificate profile in Certificate Manager (default: IPTV).

.PARAMETER AppDir
  The app folder to install (default: the "app" folder next to this script,
  as in the release download; when building from source: target\tizen\app).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File install-on-tv.ps1 -TvIp 192.168.1.4
#>
param(
    [Parameter(Mandatory = $true)][string]$TvIp,
    [string]$Profile = 'IPTV',
    [string]$AppDir,
    [string]$TizenStudio = 'C:\tizen-studio'
)
$ErrorActionPreference = 'Stop'
$appId = 'vkIPTVtv01.IPTV'
# Default: the "app" folder next to this script. (Set here, not in param():
# Windows PowerShell 5.1 leaves $PSScriptRoot empty in parameter defaults.)
if (-not $AppDir) { $AppDir = Join-Path $PSScriptRoot 'app' }

function Step($text) { Write-Host "`n== $text" -ForegroundColor Cyan }
function Fail($text) { Write-Host "`n$text" -ForegroundColor Red; exit 1 }

$tizen = Join-Path $TizenStudio 'tools\ide\bin\tizen.bat'
$sdb = Join-Path $TizenStudio 'tools\sdb.exe'
if (-not (Test-Path $tizen) -or -not (Test-Path $sdb)) {
    Fail "Tizen Studio not found in $TizenStudio. Install the Tizen Studio command-line tools first (see README), or pass -TizenStudio <folder>."
}
if (-not (Test-Path (Join-Path $AppDir 'config.xml'))) { Fail "No app found in $AppDir (expected config.xml)." }

Step "Connecting to the TV at $TvIp"
& $sdb connect "${TvIp}:26101" | Out-Host
$serial = (& $sdb devices) | Select-String -Pattern "^${TvIp}:\d+\s+device\s+(\S+)" | ForEach-Object { $_.Matches[0].Groups[1].Value } | Select-Object -First 1
if (-not $serial) {
    Fail ("Could not connect. Check that the TV is on, in Developer Mode with this PC's IP address " +
          "($((Get-NetIPConfiguration | Where-Object IPv4DefaultGateway | Select-Object -First 1).IPv4Address.IPAddress)), " +
          "and was restarted after enabling it.")
}
$duid = (& $sdb -s "${TvIp}:26101" shell 0 getduid 2>$null) -join ''
Write-Host "Connected: $serial  (DUID: $($duid.Trim()))"

Step "Checking certificate profile '$Profile'"
$profiles = (& $tizen security-profiles list 2>&1) -join "`n"
if ($profiles -notmatch "(?m)^\s*$([regex]::Escape($Profile))\b") {
    Write-Host @"

No certificate profile named '$Profile' yet. Create one (only needed once per PC):
  1. Start: $TizenStudio\tools\certificate-manager\certificate-manager.exe
  2. Click '+', choose 'Samsung', device type 'TV', profile name '$Profile'.
  3. Create a new author certificate (any name and password; keep the password).
  4. Sign in with your Samsung account when asked.
  5. For the distributor certificate keep 'Public' privilege; the TV's DUID
     ($($duid.Trim())) is filled in automatically while it's connected.
  6. Finish, then run this script again.
"@ -ForegroundColor Yellow
    exit 2
}

Step "Signing the app"
$work = Join-Path $env:TEMP "iptv-tizen-$PID"
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
Copy-Item $AppDir $work -Recurse
& $tizen package -t wgt -s $Profile -- $work | Out-Host
$wgt = Get-ChildItem "$work\*.wgt" | Select-Object -First 1
if (-not $wgt) { Fail "Packaging failed (see messages above)." }

Step "Installing on the TV"
$out = (& $tizen install -n $wgt.FullName -t $serial 2>&1) -join "`n"
Write-Host $out
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
if ($out -notmatch 'successfully|completed') {
    if ($out -match 'certificate|signature|author') {
        Fail ("The TV rejected the signature. Make sure profile '$Profile' is a Samsung certificate that " +
              "includes this TV's DUID ($($duid.Trim())). Recreate it with the TV connected.")
    }
    Fail "Installation failed (see messages above)."
}

Step "Starting IPTV"
& $tizen run -p $appId -t $serial | Out-Host
Write-Host "`nDone. IPTV is now in the TV's Apps list." -ForegroundColor Green
