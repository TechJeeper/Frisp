$Identity = Join-Path $env:USERPROFILE ".ssh\id_ed25519_printventory"
$SshHost = "cody@192.168.68.92"
$sshArgs = @(
    "-i", $Identity,
    "-o", "BatchMode=yes"
)

$remote = @"
set -euo pipefail
cd "`$HOME/frisp-build/src-tree"
rm -rf dist/dmg-stage
mkdir -p dist/dmg-stage
cp -R dist/Frisp-macOS/Frisp.app dist/dmg-stage/
cp dist/Frisp-macOS/install-macos.sh dist/dmg-stage/
cp dist/Frisp-macOS/frisp-cable dist/dmg-stage/
rm -f dist/dmg-stage/Applications
ln -s /Applications dist/dmg-stage/Applications
rm -f dist/Frisp-Installer.dmg
hdiutil create -volname "Frisp" -srcfolder dist/dmg-stage -ov -format UDZO dist/Frisp-Installer.dmg
"@
$remote = $remote.Replace("`r`n", "`n")

& ssh @sshArgs $SshHost $remote
if ($LASTEXITCODE -eq 0) {
    & scp @sshArgs "${SshHost}:~/frisp-build/src-tree/dist/Frisp-Installer.dmg" .\dist\Frisp-Installer.dmg
    Write-Host "Downloaded dist\Frisp-Installer.dmg successfully!"
} else {
    throw "Remote DMG build failed"
}
