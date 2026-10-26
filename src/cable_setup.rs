use crate::engine::virtual_cable_present;
use anyhow::anyhow;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};
use windows_sys::Win32::UI::Shell::{
    IsUserAnAdmin, ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const ZIP_URL: &str = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip";

static STARTED: AtomicBool = AtomicBool::new(false);

pub fn kickoff() {
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        if virtual_cable_present() {
            let _ = write_status("ready", "");
            return;
        }
        if let Some((phase, _)) = read_status() {
            if phase == "reboot" {
                return;
            }
            if phase == "working" && status_age() < Duration::from_secs(180) {
                return;
            }
        }
        if let Err(err) = begin() {
            let _ = write_status("failed", &err.to_string());
        }
    });
}

pub fn install_blocking() -> anyhow::Result<()> {
    write_status("working", "Downloading the virtual cable.")?;
    let dir = cache_dir()?;
    std::fs::create_dir_all(&dir)?;
    let zip_path = dir.join("VBCABLE_Driver_Pack45.zip");
    download(&zip_path)?;
    write_status("working", "Installing the virtual cable. Accept the Windows driver prompt if it appears.")?;
    let unpacked = dir.join("unpacked");
    if unpacked.exists() {
        let _ = std::fs::remove_dir_all(&unpacked);
    }
    std::fs::create_dir_all(&unpacked)?;
    extract(&zip_path, &unpacked)?;
    let inf_path = unpacked.join("vbMmeCable64_win10.inf");
    if inf_path.exists() {
        let _ = Command::new("pnputil.exe")
            .args(["/add-driver", inf_path.to_str().unwrap_or(""), "/install"])
            .status();
    }
    let setup = find_setup(&unpacked).ok_or_else(|| anyhow!("the cable package did not include its installer"))?;
    let _ = Command::new(&setup).current_dir(&unpacked).arg("-i").status();
    let _ = rename_endpoints();
    std::thread::sleep(Duration::from_secs(2));
    if virtual_cable_present() {
        write_status("ready", "Frisp Mic is ready.")?;
    } else {
        write_status(
            "failed",
            "Frisp Mic driver needs admin approval: run VBCABLE_Setup as Admin.",
        )?;
    }
    Ok(())
}

pub fn notice() -> Option<String> {
    if virtual_cable_present() {
        return None;
    }
    let (phase, message) = read_status()?;
    match phase.as_str() {
        "working" | "reboot" | "failed" if !message.is_empty() => Some(message),
        _ => None,
    }
}

pub fn launch_installer_gui() -> anyhow::Result<()> {
    let dir = cache_dir()?;
    let unpacked = dir.join("unpacked");
    let zip_path = dir.join("VBCABLE_Driver_Pack45.zip");
    if !unpacked.exists() {
        std::fs::create_dir_all(&dir)?;
        if !zip_path.exists() {
            let _ = download(&zip_path);
        }
        let _ = extract(&zip_path, &unpacked);
    }
    let setup = find_setup(&unpacked).ok_or_else(|| anyhow!("could not find VBCABLE_Setup"))?;
    let file = wide_path(&setup);
    let verb = wide("runas");
    let directory = wide_path(&unpacked);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpDirectory = directory.as_ptr();
    info.nShow = SW_SHOWNORMAL;
    let ok = unsafe { ShellExecuteExW(&mut info) != 0 };
    if !ok {
        let _ = Command::new("powershell.exe")
            .args([
                "-Command",
                &format!(
                    "Start-Process -FilePath '{}' -WorkingDirectory '{}' -Verb RunAs",
                    setup.display(),
                    unpacked.display()
                ),
            ])
            .status();
    }
    let _ = write_status("working", "Click 'Install Driver' in the VB-CABLE window.");
    Ok(())
}

pub fn begin() -> anyhow::Result<()> {
    if unsafe { IsUserAnAdmin() } != 0 {
        return install_blocking();
    }
    write_status("working", "Windows is asking permission to install the virtual cable.")?;
    let exe = std::env::current_exe()?;
    if !run_elevated(&exe) {
        write_status("failed", "The virtual cable install was cancelled.")?;
    }
    Ok(())
}

fn run_elevated(exe: &Path) -> bool {
    let file = wide_path(exe);
    let params = wide("--install-cable");
    let verb = wide("runas");
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = SW_SHOWNORMAL;
    unsafe { ShellExecuteExW(&mut info) != 0 }
}

fn download(path: &Path) -> anyhow::Result<()> {
    let status = Command::new("curl.exe")
        .args(["-fsSL", "-o", path.to_str().unwrap(), ZIP_URL])
        .status()?;
    if !status.success() {
        return Err(anyhow!("failed to download virtual cable"));
    }
    Ok(())
}

fn extract(zip_path: &Path, dest: &Path) -> anyhow::Result<()> {
    let status = Command::new("tar.exe")
        .args(["-xf", zip_path.to_str().unwrap(), "-C", dest.to_str().unwrap()])
        .status()?;
    if !status.success() {
        return Err(anyhow!("failed to extract virtual cable"));
    }
    Ok(())
}

fn find_setup(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut x64 = None;
    let mut x86 = None;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
            if name == "vbcable_setup_x64.exe" {
                x64 = Some(path);
            } else if name == "vbcable_setup.exe" {
                x86 = Some(path);
            }
        }
    }
    x64.or(x86)
}

fn rename_endpoints() -> anyhow::Result<()> {
    let script = cache_dir()?.join("rename-cable.ps1");
    if let Some(parent) = script.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&script, RENAME_SCRIPT)?;
    let status = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("the cable is installed and still using its original name"))
    }
}

fn cache_dir() -> anyhow::Result<PathBuf> {
    let base = dirs::data_local_dir().ok_or_else(|| anyhow!("no local app data folder"))?;
    Ok(base.join("Frisp").join("cable"))
}

fn status_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Frisp")
        .join("install-status.txt")
}

fn write_status(phase: &str, message: &str) -> anyhow::Result<()> {
    let path = status_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{phase}|{message}"))?;
    Ok(())
}

fn read_status() -> Option<(String, String)> {
    let text = std::fs::read_to_string(status_path()).ok()?;
    let (phase, message) = text.split_once('|')?;
    Some((phase.trim().to_string(), message.trim().to_string()))
}

fn status_age() -> Duration {
    std::fs::metadata(status_path())
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .unwrap_or(Duration::from_secs(10_000))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(path: &Path) -> Vec<u16> {
    wide(&path.display().to_string())
}

const RENAME_SCRIPT: &str = r#"
$ErrorActionPreference = "Stop"
$friendly = "{a45c254e-df1c-4efd-8020-67d146a850e0},2"
function Set-EndpointName([string]$Flow, [string]$Match, [string]$NewName) {
    $root = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\$Flow"
    $renamed = $false
    foreach ($endpoint in Get-ChildItem $root -ErrorAction SilentlyContinue) {
        $props = Join-Path $endpoint.PSPath "Properties"
        $item = Get-ItemProperty -Path $props -ErrorAction SilentlyContinue
        if (-not $item) { continue }
        $name = $item.$friendly
        if ($name -and $name -like $Match) {
            Set-ItemProperty -Path $props -Name $friendly -Value $NewName
            $renamed = $true
        }
    }
    return $renamed
}
$capture = Set-EndpointName "Capture" "CABLE Output*" "Frisp Mic"
$render = Set-EndpointName "Render" "CABLE Input*" "Frisp Output"
if (-not $capture -and -not $render) { exit 1 }
Restart-Service Audiosrv -Force
"#;
