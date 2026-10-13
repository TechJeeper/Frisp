# Installs the Frisp virtual microphone. Run from an administrator PowerShell.
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = $here
if (Test-Path (Join-Path $here "..\driver")) {
    $root = (Resolve-Path (Join-Path $here "..")).Path
}
$inf = Get-ChildItem -Path (Join-Path $root "driver\windows") -Recurse -Filter *.inf -ErrorAction SilentlyContinue |
    Select-Object -First 1

Write-Host "Frisp virtual microphone" -ForegroundColor Cyan

try {
    $secureBoot = (Get-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\SecureBoot\State" -Name UEFISecureBootEnabled -ErrorAction Stop).UEFISecureBootEnabled
} catch {
    $secureBoot = $null
}

if ($secureBoot -eq 1) {
    Write-Host "Secure Boot is on. Windows will not load a test-signed virtual microphone driver until Secure Boot is turned off and test signing is enabled, or until the driver has a Microsoft attestation signature." -ForegroundColor Yellow
}

if (-not $inf) {
    Write-Host "No driver package was found under driver\windows."
    Write-Host "Build the Windows virtual microphone driver, then run this script again."
    exit 1
}

Write-Host "Installing $($inf.FullName)"
pnputil /add-driver $inf.FullName /install
Write-Host "If the device does not appear, reboot and check Sound settings for Frisp Mic."
