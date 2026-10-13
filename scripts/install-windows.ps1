# Renames VB-Audio Virtual Cable endpoints to Frisp Mic and Frisp Output.
# Run from an administrator PowerShell after the cable is installed. Secure Boot stays on.
$ErrorActionPreference = "Stop"

$friendly = "{a45c254e-df1c-4efd-8020-67d146a850e0},2"

function Get-EndpointName([string]$PropertiesPath) {
    $item = Get-ItemProperty -Path $PropertiesPath -ErrorAction SilentlyContinue
    if (-not $item) { return $null }
    return $item.$friendly
}

function Set-EndpointName([string]$Flow, [string]$Match, [string]$NewName) {
    $root = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\$Flow"
    $renamed = $false
    foreach ($endpoint in Get-ChildItem $root -ErrorAction SilentlyContinue) {
        $props = Join-Path $endpoint.PSPath "Properties"
        $name = Get-EndpointName $props
        if ($name -and $name -like $Match) {
            Set-ItemProperty -Path $props -Name $friendly -Value $NewName
            Write-Host "Renamed $name to $NewName"
            $renamed = $true
        }
    }
    return $renamed
}

Write-Host "Frisp virtual microphone" -ForegroundColor Cyan

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host "Run this script from an administrator PowerShell." -ForegroundColor Yellow
    exit 1
}

$capture = Set-EndpointName "Capture" "CABLE Output*" "Frisp Mic"
$render = Set-EndpointName "Render" "CABLE Input*" "Frisp Output"

if (-not $capture -and -not $render) {
    Write-Host "VB-Audio Virtual Cable is not installed."
    Write-Host "Download it from https://vb-audio.com/Cable/ , install it, reboot, then run this script again."
    exit 1
}

Restart-Service Audiosrv -Force
Write-Host "Frisp Mic is ready. Reopen Frisp, then choose Frisp Mic in OBS or your browser."
