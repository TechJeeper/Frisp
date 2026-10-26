#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
fn main() {
    let silent = std::env::args().any(|a| a == "/S" || a == "--silent" || a == "-s");

    if !silent {
        let title = "Frisp Uninstaller\0".encode_utf16().collect::<Vec<_>>();
        let msg = "Are you sure you want to completely uninstall Frisp from your computer?\0"
            .encode_utf16()
            .collect::<Vec<_>>();
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONQUESTION, MB_YESNO};
            let ret = MessageBoxW(0 as _, msg.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION);
            if ret != IDYES {
                return;
            }
        }
    }

    // Kill running Frisp instances
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "frisp.exe"])
        .output();

    // Remove Start Menu shortcut
    if let Some(roaming) = dirs::config_dir() {
        let shortcut = roaming
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Frisp.lnk");
        let _ = std::fs::remove_file(shortcut);
    }

    // Remove Desktop shortcut
    if let Some(desktop) = dirs::desktop_dir() {
        let shortcut = desktop.join("Frisp.lnk");
        let _ = std::fs::remove_file(shortcut);
    }

    // Remove registry entry in HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Frisp
    let _ = Command::new("reg")
        .args([
            "delete",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Frisp",
            "/f",
        ])
        .output();

    let install_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Programs")
        .join("Frisp");

    // Spawn a delayed cmd process to clean up install directory
    let dir_str = install_dir.to_string_lossy().to_string();
    let cmd_str = format!("timeout /t 1 /nobreak >nul & rd /s /q \"{}\"", dir_str);
    let _ = Command::new("cmd")
        .args(["/c", &cmd_str])
        .spawn();

    if !silent {
        let title = "Frisp Uninstaller\0".encode_utf16().collect::<Vec<_>>();
        let msg = "Frisp has been successfully removed from your computer.\0"
            .encode_utf16()
            .collect::<Vec<_>>();
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
            MessageBoxW(0 as _, msg.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION);
        }
    }
}

#[cfg(not(windows))]
fn main() {}
