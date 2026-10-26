#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win_installer {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    // Embedded payloads
    const FRISP_EXE: &[u8] = include_bytes!("payload/frisp.exe");
    const UNINSTALLER_EXE: &[u8] = include_bytes!("payload/uninstall.exe");
    const DRIVER_ZIP: &[u8] = include_bytes!("payload/VBCABLE_Driver_Pack45.zip");

    const COLOR_BG: u32 = 0x00181111; // Dark navy #111827 (0x00BGR)
    const COLOR_CARD: u32 = 0x00241E1F; // #1F2937
    const COLOR_TEXT: u32 = 0x00F3F4F6;
    const COLOR_MUTED: u32 = 0x009CA3AF;
    const COLOR_EMERALD: u32 = 0x0078D534; // #34D578
    const COLOR_BORDER: u32 = 0x00374151;

    pub struct State {
        pub progress: AtomicU32,
        pub status_text: std::sync::Mutex<String>,
        pub completed: AtomicBool,
        pub launch_on_finish: AtomicBool,
        pub installing: AtomicBool,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn run_install(state: Option<&Arc<State>>) -> Result<(), String> {
        let set_status = |pct: u32, text: &str| {
            if let Some(s) = state {
                s.progress.store(pct, Ordering::Relaxed);
                if let Ok(mut lock) = s.status_text.lock() {
                    *lock = text.to_string();
                }
            }
        };

        set_status(10, "Closing existing instances of Frisp...");
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "frisp.exe"])
            .output();

        let install_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("C:\\"))
            .join("Programs")
            .join("Frisp");

        set_status(25, "Creating Frisp program directory...");
        std::fs::create_dir_all(&install_dir).map_err(|e| format!("Failed to create install dir: {e}"))?;

        let frisp_path = install_dir.join("frisp.exe");
        let uninstaller_path = install_dir.join("uninstall.exe");

        set_status(40, "Extracting Frisp binary...");
        std::fs::write(&frisp_path, FRISP_EXE).map_err(|e| format!("Failed to write frisp.exe: {e}"))?;

        set_status(50, "Writing uninstaller...");
        std::fs::write(&uninstaller_path, UNINSTALLER_EXE)
            .map_err(|e| format!("Failed to write uninstall.exe: {e}"))?;

        set_status(65, "Checking Virtual Microphone Driver...");
        let driver_present = is_virtual_cable_present();
        if !driver_present {
            set_status(75, "Setting up VB-Audio Virtual Cable...");
            let temp_dir = std::env::temp_dir().join("frisp_vbcable_install");
            let _ = std::fs::create_dir_all(&temp_dir);
            let zip_path = temp_dir.join("cable.zip");
            let _ = std::fs::write(&zip_path, DRIVER_ZIP);

            let script = format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                zip_path.to_string_lossy(),
                temp_dir.to_string_lossy()
            );
            let _ = Command::new("powershell").args(["-NoProfile", "-Command", &script]).output();

            let setup_x64 = temp_dir.join("VBCABLE_Setup_x64.exe");
            let inf_path = temp_dir.join("vbMmeCable64_win10.inf");

            if inf_path.exists() {
                let _ = Command::new("pnputil")
                    .args(["/add-driver", &inf_path.to_string_lossy(), "/install"])
                    .output();
            } else if setup_x64.exists() {
                let _ = Command::new(&setup_x64).args(["-i", "-h"]).output();
            }
        }

        set_status(85, "Creating Start Menu & Desktop shortcuts...");
        create_shortcuts(&frisp_path, &install_dir);

        set_status(95, "Registering in Windows Settings...");
        register_uninstall(&install_dir, &frisp_path, &uninstaller_path);

        set_status(100, "Installation Complete!");
        if let Some(s) = state {
            s.completed.store(true, Ordering::Relaxed);
        }

        Ok(())
    }

    fn is_virtual_cable_present() -> bool {
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-PnpDevice -FriendlyName '*CABLE*' -Status OK -ErrorAction SilentlyContinue | Measure-Object | Select-Object -ExpandProperty Count",
            ])
            .output();
        if let Ok(out) = output {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if let Ok(count) = s.parse::<u32>() {
                return count > 0;
            }
        }
        false
    }

    fn create_shortcuts(frisp_path: &Path, install_dir: &Path) {
        let script = format!(
            r#"
$ws = New-Object -ComObject WScript.Shell
$target = "{}"
$workDir = "{}"

$startDir = [Environment]::GetFolderPath("Programs")
$startLnk = Join-Path $startDir "Frisp.lnk"
$s = $ws.CreateShortcut($startLnk)
$s.TargetPath = $target
$s.WorkingDirectory = $workDir
$s.IconLocation = "$target,0"
$s.Save()

$desktopDir = [Environment]::GetFolderPath("Desktop")
$desktopLnk = Join-Path $desktopDir "Frisp.lnk"
$d = $ws.CreateShortcut($desktopLnk)
$d.TargetPath = $target
$d.WorkingDirectory = $workDir
$d.IconLocation = "$target,0"
$d.Save()
"#,
            frisp_path.to_string_lossy().replace('\\', "\\\\"),
            install_dir.to_string_lossy().replace('\\', "\\\\")
        );

        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output();
    }

    fn register_uninstall(install_dir: &Path, frisp_path: &Path, uninstaller_path: &Path) {
        let root = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Frisp";
        let dir_str = install_dir.to_string_lossy();
        let frisp_str = frisp_path.to_string_lossy();
        let uninst_str = uninstaller_path.to_string_lossy();

        let _ = Command::new("reg").args(["add", root, "/v", "DisplayName", "/d", "Frisp", "/f"]).output();
        let _ = Command::new("reg").args(["add", root, "/v", "DisplayVersion", "/d", "1.0.0", "/f"]).output();
        let _ = Command::new("reg").args(["add", root, "/v", "Publisher", "/d", "Frisp Audio", "/f"]).output();
        let _ = Command::new("reg").args(["add", root, "/v", "InstallLocation", "/d", &dir_str, "/f"]).output();
        let _ = Command::new("reg").args(["add", root, "/v", "UninstallString", "/d", &format!("\"{uninst_str}\""), "/f"]).output();
        let _ = Command::new("reg").args(["add", root, "/v", "DisplayIcon", "/d", &format!("\"{frisp_str}\",0"), "/f"]).output();
        let _ = Command::new("reg").args(["add", root, "/v", "NoModify", "/t", "REG_DWORD", "/d", "1", "/f"]).output();
        let _ = Command::new("reg").args(["add", root, "/v", "NoRepair", "/t", "REG_DWORD", "/d", "1", "/f"]).output();
    }

    static APP_STATE: std::sync::OnceLock<Arc<State>> = std::sync::OnceLock::new();

    pub fn main_gui() {
        let state = Arc::new(State {
            progress: AtomicU32::new(0),
            status_text: std::sync::Mutex::new("Ready to install Frisp".to_string()),
            completed: AtomicBool::new(false),
            launch_on_finish: AtomicBool::new(true),
            installing: AtomicBool::new(false),
        });

        let _ = APP_STATE.set(Arc::clone(&state));

        unsafe {
            let class_name = wide("FrispSetupWindow");
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: LoadIconW(std::ptr::null_mut(), 32512 as *const u16),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&class);

            let title = wide("Frisp Setup");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                520,
                460,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );

            ShowWindow(hwnd, SW_SHOW);
            SetTimer(hwnd, 1, 40, None);

            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_TIMER => {
                InvalidateRect(hwnd, std::ptr::null(), 0);
                0
            }
            WM_LBUTTONDOWN => {
                let x = (lparam & 0xFFFF) as i32;
                let y = ((lparam >> 16) & 0xFFFF) as i32;

                if let Some(state) = APP_STATE.get() {
                    let btn_rect = RECT { left: 40, top: 340, right: 460, bottom: 390 };
                    let chk_rect = RECT { left: 40, top: 295, right: 300, bottom: 325 };

                    // Toggle checkbox
                    if x >= chk_rect.left && x <= chk_rect.right && y >= chk_rect.top && y <= chk_rect.bottom {
                        let cur = state.launch_on_finish.load(Ordering::Relaxed);
                        state.launch_on_finish.store(!cur, Ordering::Relaxed);
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                    }

                    // Click Install / Finish button
                    if x >= btn_rect.left && x <= btn_rect.right && y >= btn_rect.top && y <= btn_rect.bottom {
                        if state.completed.load(Ordering::Relaxed) {
                            if state.launch_on_finish.load(Ordering::Relaxed) {
                                let install_dir = dirs::data_local_dir()
                                    .unwrap_or_else(|| PathBuf::from("C:\\"))
                                    .join("Programs")
                                    .join("Frisp");
                                let frisp_exe = install_dir.join("frisp.exe");
                                let _ = Command::new("explorer").arg(frisp_exe).spawn();
                            }
                            PostQuitMessage(0);
                        } else if !state.installing.load(Ordering::Relaxed) {
                            state.installing.store(true, Ordering::Relaxed);
                            let state_clone = Arc::clone(state);
                            std::thread::spawn(move || {
                                let _ = run_install(Some(&state_clone));
                            });
                        }
                    }
                }
                0
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                paint_installer(hwnd, hdc);
                EndPaint(hwnd, &ps);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn paint_installer(hwnd: HWND, hdc: HDC) {
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let width = client.right - client.left;
        let height = client.bottom - client.top;

        let mem_dc = CreateCompatibleDC(hdc);
        let mem_bmp = CreateCompatibleBitmap(hdc, width, height);
        let old_bmp = SelectObject(mem_dc, mem_bmp);

        // Background
        let bg_brush = CreateSolidBrush(COLOR_BG);
        FillRect(mem_dc, &client, bg_brush);
        DeleteObject(bg_brush);

        SetBkMode(mem_dc, TRANSPARENT as i32);

        // Header
        SetTextColor(mem_dc, COLOR_TEXT);
        let font_title = CreateFontW(28, 0, 0, 0, FW_BOLD as i32, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, wide("Segoe UI").as_ptr());
        let old_font = SelectObject(mem_dc, font_title);
        let title = wide("Frisp Setup");
        TextOutW(mem_dc, 40, 30, title.as_ptr(), (title.len() - 1) as i32);

        let font_sub = CreateFontW(16, 0, 0, 0, FW_NORMAL as i32, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, wide("Segoe UI").as_ptr());
        SelectObject(mem_dc, font_sub);
        SetTextColor(mem_dc, COLOR_MUTED);
        let subtitle = wide("Krisp-grade AI noise suppression & virtual microphone");
        TextOutW(mem_dc, 40, 68, subtitle.as_ptr(), (subtitle.len() - 1) as i32);

        // Card Features
        let card_rect = RECT { left: 40, top: 105, right: 460, bottom: 235 };
        let card_brush = CreateSolidBrush(COLOR_CARD);
        FillRect(mem_dc, &card_rect, card_brush);
        DeleteObject(card_brush);

        let border_pen = CreatePen(PS_SOLID as i32, 1, COLOR_BORDER);
        let old_pen = SelectObject(mem_dc, border_pen);
        let null_brush = GetStockObject(NULL_BRUSH);
        let old_brush = SelectObject(mem_dc, null_brush);
        Rectangle(mem_dc, card_rect.left, card_rect.top, card_rect.right, card_rect.bottom);
        SelectObject(mem_dc, old_brush);
        SelectObject(mem_dc, old_pen);
        DeleteObject(border_pen);

        let font_item = CreateFontW(15, 0, 0, 0, FW_SEMIBOLD as i32, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, wide("Segoe UI").as_ptr());
        SelectObject(mem_dc, font_item);

        let items = [
            ("✓  Krisp DeepFilterNet + Butterworth HPF + Voice Leveler", 125),
            ("✓  VB-Audio Virtual Cable Microphone Driver (Zero latency)", 158),
            ("✓  Desktop & Start Menu Shortcuts, Clean Uninstaller", 191),
        ];
        for (item_text, y) in items {
            SetTextColor(mem_dc, COLOR_EMERALD);
            let w_check = wide("✓ ");
            TextOutW(mem_dc, 60, y, w_check.as_ptr(), (w_check.len() - 1) as i32);

            SetTextColor(mem_dc, COLOR_TEXT);
            let w_body = wide(&item_text[3..]);
            TextOutW(mem_dc, 82, y, w_body.as_ptr(), (w_body.len() - 1) as i32);
        }

        // Progress bar and Status
        if let Some(state) = APP_STATE.get() {
            let pct = state.progress.load(Ordering::Relaxed);
            let completed = state.completed.load(Ordering::Relaxed);
            let installing = state.installing.load(Ordering::Relaxed);
            let status = state.status_text.lock().unwrap().clone();

            let font_status = CreateFontW(14, 0, 0, 0, FW_NORMAL as i32, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, wide("Segoe UI").as_ptr());
            SelectObject(mem_dc, font_status);
            SetTextColor(mem_dc, COLOR_MUTED);
            let w_status = wide(&status);
            TextOutW(mem_dc, 40, 252, w_status.as_ptr(), (w_status.len() - 1) as i32);

            // Progress bar track
            let track_rect = RECT { left: 40, top: 275, right: 460, bottom: 285 };
            let track_brush = CreateSolidBrush(COLOR_CARD);
            FillRect(mem_dc, &track_rect, track_brush);
            DeleteObject(track_brush);

            // Progress bar fill
            let fill_width = ((pct as f32 / 100.0) * (track_rect.right - track_rect.left) as f32) as i32;
            if fill_width > 0 {
                let fill_rect = RECT { left: track_rect.left, top: track_rect.top, right: track_rect.left + fill_width, bottom: track_rect.bottom };
                let fill_brush = CreateSolidBrush(COLOR_EMERALD);
                FillRect(mem_dc, &fill_rect, fill_brush);
                DeleteObject(fill_brush);
            }

            // Launch checkbox
            let checked = state.launch_on_finish.load(Ordering::Relaxed);
            SetTextColor(mem_dc, COLOR_TEXT);
            let chk_label = if checked { "☑  Launch Frisp when setup finishes" } else { "☐  Launch Frisp when setup finishes" };
            let w_chk = wide(chk_label);
            TextOutW(mem_dc, 40, 305, w_chk.as_ptr(), (w_chk.len() - 1) as i32);

            // Main Action Button
            let btn_rect = RECT { left: 40, top: 340, right: 460, bottom: 390 };
            let btn_brush = CreateSolidBrush(COLOR_EMERALD);
            FillRect(mem_dc, &btn_rect, btn_brush);
            DeleteObject(btn_brush);

            let font_btn = CreateFontW(18, 0, 0, 0, FW_BOLD as i32, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, wide("Segoe UI").as_ptr());
            SelectObject(mem_dc, font_btn);
            SetTextColor(mem_dc, 0x00000000); // Black text on emerald button
            let btn_text = if completed {
                "Finish & Launch Frisp"
            } else if installing {
                "Installing..."
            } else {
                "Install Frisp Now"
            };
            let w_btn = wide(btn_text);
            let mut text_rect = btn_rect;
            DrawTextW(mem_dc, w_btn.as_ptr(), (w_btn.len() - 1) as i32, &mut text_rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

            DeleteObject(font_status);
            DeleteObject(font_btn);
        }

        BitBlt(hdc, 0, 0, width, height, mem_dc, 0, 0, SRCCOPY);

        SelectObject(mem_dc, old_font);
        SelectObject(mem_dc, old_bmp);
        DeleteObject(mem_bmp);
        DeleteDC(mem_dc);
        DeleteObject(font_title);
        DeleteObject(font_sub);
        DeleteObject(font_item);
    }
}

#[cfg(windows)]
fn main() {
    let silent = std::env::args().any(|a| a == "/S" || a == "--silent" || a == "-s");
    if silent {
        let _ = win_installer::run_install(None);
    } else {
        win_installer::main_gui();
    }
}

#[cfg(not(windows))]
fn main() {}
