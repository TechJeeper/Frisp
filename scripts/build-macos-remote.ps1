# Build the macOS app and virtual microphone on the Printventory Mac.
param(
    [string]$SshHost = $(if ($env:MAC_BUILD_SSH_HOST) { $env:MAC_BUILD_SSH_HOST } else { "cody@192.168.68.92" }),
    [string]$Identity = $(Join-Path $env:USERPROFILE ".ssh\id_ed25519_printventory")
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root
New-Item -ItemType Directory -Force -Path (Join-Path $root "dist") | Out-Null

$archive = Join-Path $root "dist\frisp-src.tgz"
if (Test-Path $archive) { Remove-Item $archive }
tar --exclude target --exclude dist --exclude .git --exclude "*.exe" --exclude "*.dmg" -czf $archive .

$sshArgs = @(
    "-i", $Identity,
    "-o", "IdentitiesOnly=yes",
    "-o", "BatchMode=yes",
    "-o", "StrictHostKeyChecking=accept-new"
)
& ssh @sshArgs $SshHost "mkdir -p ~/frisp-build"
if ($LASTEXITCODE -ne 0) { throw "Could not create the remote build directory." }
& scp @sshArgs $archive "${SshHost}:~/frisp-build/frisp-src.tgz"
if ($LASTEXITCODE -ne 0) { throw "Could not upload the source archive." }

$remote = @'
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$HOME/frisp-build"
rm -rf src-tree
mkdir src-tree
tar -xzf frisp-src.tgz -C src-tree
cd src-tree
python3 - << 'PY'
import pathlib
for path in pathlib.Path(".").rglob("*"):
    if path.is_file() and (path.suffix in {".sh", ".c"} or path.name == "Makefile"):
        data = path.read_bytes().replace(b"\r\n", b"\n").replace(b"\r", b"\n")
        path.write_bytes(data)
PY
chmod +x scripts/package-macos.sh scripts/install-macos.sh
scripts/package-macos.sh
'@
$remote = $remote.Replace("`r`n", "`n").Replace("`r", "`n")
$remotePath = Join-Path $env:TEMP "frisp-remote-build.sh"
$utf8 = New-Object System.Text.UTF8Encoding $false
[System.IO.File]::WriteAllText($remotePath, $remote, $utf8)
& scp @sshArgs $remotePath "${SshHost}:~/frisp-build/build.sh"
if ($LASTEXITCODE -ne 0) { throw "Could not upload the remote build script." }
& ssh @sshArgs $SshHost "bash ~/frisp-build/build.sh"
if ($LASTEXITCODE -ne 0) { throw "The macOS build failed." }
New-Item -ItemType Directory -Force -Path (Join-Path $root "dist") | Out-Null
& scp @sshArgs "${SshHost}:~/frisp-build/src-tree/dist/Frisp-macOS.zip" (Join-Path $root "dist\Frisp-macOS.zip")
if ($LASTEXITCODE -ne 0) { throw "Could not download the macOS package." }
Write-Host "Downloaded dist\Frisp-macOS.zip"
