use crate::cable_setup;
use crate::config::Config;
use crate::engine::{Engine, Shared};
use crate::win_paint::{self, Fonts, Frame, Layout, Target};
use anyhow::anyhow;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, DeleteDC,
    DeleteObject, EndPaint, GetStockObject, InvalidateRect, SelectObject, DEFAULT_GUI_FONT,
    PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyMenu, DispatchMessageW, GetClientRect, GetCursorPos, GetMessageW, GetWindowLongPtrW,
    LoadCursorW, LoadIconW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, TrackPopupMenu,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, IDC_ARROW,
    MF_STRING, MSG, SC_MINIMIZE, SW_HIDE, SW_RESTORE, SW_SHOW, SWP_NOMOVE, SWP_NOZORDER,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY,
    WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_RBUTTONUP, WM_SYSCOMMAND, WM_TIMER, WNDCLASSW, WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED,
    WS_SYSMENU,
};

const TRAY_MSG: u32 = 0x8000 + 11;
const ID_TRAY_OPEN: usize = 1;
const ID_TRAY_EXIT: usize = 2;
const DPI_AWARE: isize = -4;

struct App {
    engine: Engine,
    shared: Arc<Shared>,
    config: Config,
    devices: Vec<String>,
    selected: Option<String>,
    chose: bool,
    list_open: bool,
    hover: Target,
    drag_slider: bool,
    input_level: f32,
    output_level: f32,
    history: VecDeque<f32>,
    raw_bars: Vec<f32>,
    clean_bars: Vec<f32>,
    bars_ready: bool,
    layout: Layout,
    fonts: Fonts,
    dpi: u32,
    tray_on: bool,
}

pub fn run() -> anyhow::Result<()> {
    unsafe {
        let _ = std::fs::write("frisp_debug.log", "win_ui::run started\n");
        SetProcessDpiAwarenessContext(DPI_AWARE as *mut _);
        let class_name = wide("FrispWindow");
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
        if RegisterClassW(&class) == 0 {
            return Err(anyhow!("Windows did not register the Frisp window"));
        }
        let title = wide("Frisp");
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            480,
            900,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err(anyhow!("Windows did not open Frisp"));
        }
        dark_caption(hwnd);
        ShowWindow(hwnd, SW_SHOW);
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    {
        if message == WM_CREATE {
            let app = Box::new(App::new(hwnd));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(app) as isize);
            fit_window(hwnd);
            windows_sys::Win32::UI::WindowsAndMessaging::SetTimer(hwnd, 1, 33, None);
            cable_setup::kickoff();
            return 0;
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        if message == WM_DESTROY {
            (*pointer).hide_tray(hwnd);
            drop(Box::from_raw(pointer));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            PostQuitMessage(0);
            return 0;
        }
        let app = &mut *pointer;
        match message {
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                paint(hwnd, app);
                0
            }
            WM_TIMER => {
                app.tick();
                InvalidateRect(hwnd, std::ptr::null(), 0);
                0
            }
            WM_MOUSEMOVE => {
                let (x, y) = point(lparam);
                let next = hit(app, x, y);
                if next != app.hover {
                    app.hover = next;
                }
                if app.drag_slider {
                    app.set_strength(strength_at(app, x));
                }
                0
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point(lparam);
                app.press(hwnd, x, y);
                0
            }
            WM_LBUTTONUP => {
                if app.drag_slider {
                    app.drag_slider = false;
                    ReleaseCapture();
                    app.config.save();
                }
                0
            }
            WM_SYSCOMMAND => {
                if (wparam as u32 & 0xfff0) == SC_MINIMIZE && app.config.minimize_to_tray {
                    ShowWindow(hwnd, SW_HIDE);
                    app.show_tray(hwnd);
                    return 0;
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            TRAY_MSG => {
                let mouse = lparam as u32;
                if mouse == WM_LBUTTONUP || mouse == 0x0203 {
                    app.restore(hwnd);
                } else if mouse == WM_RBUTTONUP {
                    app.tray_menu(hwnd);
                }
                0
            }
            WM_COMMAND => {
                match (wparam as usize) & 0xffff {
                    ID_TRAY_OPEN => app.restore(hwnd),
                    ID_TRAY_EXIT => {
                        app.hide_tray(hwnd);
                        PostQuitMessage(0);
                    }
                    _ => {}
                }
                0
            }
            WM_DPICHANGED => {
                fit_window(hwnd);
                app.rebuild_fonts(hwnd);
                0
            }
            WM_CLOSE => {
                app.hide_tray(hwnd);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

impl App {
    fn new(hwnd: HWND) -> Self {
        let config = Config::load();
        let (engine, shared) = Engine::spawn(
            config.noise_cancel,
            config.strength,
            config.clarity_boost,
            config.voice_leveler,
            config.low_cut,
        );
        shared.noise_cancel.store(config.noise_cancel, Ordering::Relaxed);
        let dpi = unsafe { GetDpiForWindow(hwnd).max(96) };
        Self {
            engine,
            shared,
            config,
            devices: Vec::new(),
            selected: None,
            chose: false,
            list_open: false,
            hover: Target::None,
            drag_slider: false,
            input_level: 0.0,
            output_level: 0.0,
            history: VecDeque::from(vec![0.0; 48]),
            raw_bars: Vec::new(),
            clean_bars: Vec::new(),
            bars_ready: false,
            layout: Layout::default(),
            fonts: make_fonts(dpi),
            dpi,
            tray_on: false,
        }
    }

    fn tick(&mut self) {
        self.sync_devices();
        let measured_in = f32::from_bits(self.shared.input_peak.load(Ordering::Relaxed)).min(1.0);
        let measured_out = f32::from_bits(self.shared.output_peak.load(Ordering::Relaxed)).min(1.0);
        self.input_level = (self.input_level * 0.84).max(measured_in);
        self.output_level = (self.output_level * 0.84).max(measured_out);
        self.input_level *= 0.98;
        self.output_level *= 0.98;
        self.history.pop_front();
        self.history.push_back(self.input_level);
        if self.shared.has_clip() && !self.bars_ready {
            if let Ok(slot) = self.shared.compare.lock() {
                if let Some(clip) = slot.as_ref() {
                    self.raw_bars = win_paint::bars(&clip.original, 42);
                    self.clean_bars = win_paint::bars(&clip.cleaned, 42);
                    self.bars_ready = true;
                }
            }
        }
        if self.shared.recording() {
            self.bars_ready = false;
        }
    }

    fn sync_devices(&mut self) {
        let Ok(devices) = self.shared.devices.lock() else {
            return;
        };
        if *devices != self.devices {
            self.devices = devices.clone();
        }
        drop(devices);
        if let Some(selected) = &self.selected {
            if !self.devices.iter().any(|device| device == selected) {
                self.selected = None;
                self.engine.set_input(None);
                self.config.input_device = None;
                self.config.save();
                self.chose = false;
            }
        }
        if self.chose || self.selected.is_some() {
            if self.selected.is_some() {
                self.chose = true;
            }
            return;
        }
        let default = self.shared.default_input.lock().ok().and_then(|slot| slot.clone());
        let selected = self
            .config
            .input_device
            .clone()
            .filter(|name| self.devices.iter().any(|device| device == name))
            .or(default)
            .filter(|name| self.devices.iter().any(|device| device == name));
        if selected.is_some() {
            self.selected = selected.clone();
            self.engine.set_input(selected);
            self.config.input_device = self.selected.clone();
            self.config.save();
            self.chose = true;
        }
    }

    fn press(&mut self, hwnd: HWND, x: i32, y: i32) {
        if self.list_open {
            for index in 0..self.layout.row_count {
                if contains(self.layout.rows[index], x, y) {
                    if let Some(name) = self.devices.get(index).cloned() {
                        self.selected = Some(name.clone());
                        self.engine.set_input(Some(name.clone()));
                        self.config.input_device = Some(name);
                        self.config.save();
                        self.chose = true;
                    }
                    self.list_open = false;
                    return;
                }
            }
            self.list_open = false;
            if !contains(self.layout.mic, x, y) {
                return;
            }
        }
        match hit(self, x, y) {
            Target::Mic => self.list_open = !self.list_open,
            Target::Toggle => {
                let enabled = !self.shared.noise_cancel.load(Ordering::Relaxed);
                self.shared.noise_cancel.store(enabled, Ordering::Relaxed);
                self.config.noise_cancel = enabled;
                self.config.save();
            }
            Target::Slider => {
                self.drag_slider = true;
                unsafe { SetCapture(hwnd); }
                self.set_strength(strength_at(self, x));
            }
            Target::Clarity => {
                let enabled = !self.shared.clarity_boost.load(Ordering::Relaxed);
                self.shared.clarity_boost.store(enabled, Ordering::Relaxed);
                self.config.clarity_boost = enabled;
                self.config.save();
            }
            Target::Leveler => {
                let enabled = !self.shared.voice_leveler.load(Ordering::Relaxed);
                self.shared.voice_leveler.store(enabled, Ordering::Relaxed);
                self.config.voice_leveler = enabled;
                self.config.save();
            }
            Target::LowCut => {
                let enabled = !self.shared.low_cut.load(Ordering::Relaxed);
                self.shared.low_cut.store(enabled, Ordering::Relaxed);
                self.config.low_cut = enabled;
                self.config.save();
            }
            Target::Record => {
                if !self.shared.recording() {
                    self.bars_ready = false;
                    self.raw_bars.clear();
                    self.clean_bars.clear();
                    self.engine.begin_sample(self.selected.clone());
                }
            }
            Target::Original if self.shared.has_clip() => self.engine.preview(false),
            Target::Cancelled if self.shared.has_clip() => self.engine.preview(true),
            Target::Tray => {
                self.config.minimize_to_tray = !self.config.minimize_to_tray;
                self.config.save();
            }
            Target::Status => {
                if !self.shared.virtual_ready.load(Ordering::Relaxed) {
                    std::thread::spawn(|| {
                        let _ = cable_setup::launch_installer_gui();
                    });
                }
            }
            Target::InstallCable => {
                std::thread::spawn(|| {
                    let _ = cable_setup::launch_installer_gui();
                });
            }
            _ => {}
        }
    }

    fn set_strength(&mut self, value: f32) {
        let value = value.clamp(0.0, 1.0);
        self.shared.strength.store(value.to_bits(), Ordering::Relaxed);
        self.config.strength = value;
    }

    fn scene(&self) -> Frame {
        let recording = self.shared.recording();
        let playing = self.shared.playing.load(Ordering::Relaxed);
        let status = if recording {
            format!("Listening for a sample… {:.0}%", self.shared.record_progress() * 100.0)
        } else if playing == 1 {
            "Playing the original.".into()
        } else if playing == 2 {
            "Playing with cancellation.".into()
        } else if let Some(notice) = cable_setup::notice() {
            notice
        } else {
            self.shared.status.lock().map(|text| text.clone()).unwrap_or_else(|_| "Looking for microphones.".into())
        };
        Frame {
            devices: self.devices.clone(),
            selected: self.selected.clone().unwrap_or_else(|| "Select a microphone".into()),
            list_open: self.list_open,
            hover: self.hover,
            input_level: self.input_level,
            output_level: self.output_level,
            history: self.history.iter().copied().collect(),
            cancel: self.shared.noise_cancel.load(Ordering::Relaxed),
            strength: f32::from_bits(self.shared.strength.load(Ordering::Relaxed)),
            clarity_boost: self.shared.clarity_boost.load(Ordering::Relaxed),
            voice_leveler: self.shared.voice_leveler.load(Ordering::Relaxed),
            low_cut: self.shared.low_cut.load(Ordering::Relaxed),
            voice_active: self.shared.voice_active.load(Ordering::Relaxed),
            recording,
            progress: self.shared.record_progress(),
            has_clip: self.shared.has_clip(),
            raw_bars: self.raw_bars.clone(),
            clean_bars: self.clean_bars.clone(),
            playing,
            status,
            tray: self.config.minimize_to_tray,
            live: self.shared.virtual_ready.load(Ordering::Relaxed),
        }
    }

    fn show_tray(&mut self, hwnd: HWND) {
        if self.tray_on {
            return;
        }
        let mut tip = [0u16; 128];
        for (index, unit) in "Frisp".encode_utf16().take(127).enumerate() {
            tip[index] = unit;
        }
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = TRAY_MSG;
        data.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), 32512 as *const u16) };
        data.szTip = tip;
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &data);
        }
        self.tray_on = true;
    }

    fn hide_tray(&mut self, hwnd: HWND) {
        if !self.tray_on {
            return;
        }
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
        }
        self.tray_on = false;
    }

    fn restore(&mut self, hwnd: HWND) {
        self.hide_tray(hwnd);
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }

    fn tray_menu(&mut self, hwnd: HWND) {
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            let open = wide("Open Frisp");
            let exit = wide("Exit");
            AppendMenuW(menu, MF_STRING, ID_TRAY_OPEN, open.as_ptr());
            AppendMenuW(menu, MF_STRING, ID_TRAY_EXIT, exit.as_ptr());
            let mut cursor = POINT { x: 0, y: 0 };
            GetCursorPos(&mut cursor);
            SetForegroundWindow(hwnd);
            TrackPopupMenu(menu, tpm_rightalign_safe(), cursor.x, cursor.y, 0, hwnd, std::ptr::null());
            DestroyMenu(menu);
        }
    }

    fn rebuild_fonts(&mut self, hwnd: HWND) {
        let dpi = unsafe { GetDpiForWindow(hwnd).max(96) };
        if dpi == self.dpi {
            return;
        }
        drop_fonts(&self.fonts);
        self.dpi = dpi;
        self.fonts = make_fonts(dpi);
    }
}

fn tpm_rightalign_safe() -> u32 {
    TPM_RETURNCMD | TPM_LEFTALIGN | TPM_BOTTOMALIGN
}

fn paint(hwnd: HWND, app: &mut App) {
    unsafe {
        let mut info: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut info);
        let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut client);
        let width = client.right - client.left;
        let height = client.bottom - client.top;
        let memory = CreateCompatibleDC(hdc);
        let bitmap = CreateCompatibleBitmap(hdc, width, height);
        let previous = SelectObject(memory, bitmap);
        let scene = app.scene();
        app.layout = win_paint::paint(memory, width, height, app.dpi, &app.fonts, &scene);
        BitBlt(hdc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
        SelectObject(memory, previous);
        DeleteObject(bitmap);
        DeleteDC(memory);
        EndPaint(hwnd, &info);
    }
}

fn fit_window(hwnd: HWND) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let mut client = RECT {
            left: 0,
            top: 0,
            right: 460 * dpi as i32 / 96,
            bottom: 870 * dpi as i32 / 96,
        };
        AdjustWindowRectEx(&mut client, WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, 0, 0);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            client.right - client.left,
            client.bottom - client.top,
            SWP_NOMOVE | SWP_NOZORDER,
        );
    }
}

fn dark_caption(hwnd: HWND) {
    unsafe {
        let dark: i32 = 1;
        DwmSetWindowAttribute(hwnd, 20, &dark as *const i32 as *const _, 4);
        let caption = 16u32 | (18u32 << 8) | (22u32 << 16);
        DwmSetWindowAttribute(hwnd, 35, &caption as *const u32 as *const _, 4);
        let text = 244u32 | (247u32 << 8) | (250u32 << 16);
        DwmSetWindowAttribute(hwnd, 36, &text as *const u32 as *const _, 4);
    }
}

fn hit(app: &App, x: i32, y: i32) -> Target {
    if app.list_open {
        for index in 0..app.layout.row_count {
            if contains(app.layout.rows[index], x, y) {
                return Target::Row(index);
            }
        }
    }
    if contains(app.layout.mic, x, y) {
        return Target::Mic;
    }
    if contains(app.layout.toggle, x, y) {
        return Target::Toggle;
    }
    if contains(app.layout.slider, x, y) {
        return Target::Slider;
    }
    if contains(app.layout.clarity, x, y) {
        return Target::Clarity;
    }
    if contains(app.layout.leveler, x, y) {
        return Target::Leveler;
    }
    if contains(app.layout.low_cut, x, y) {
        return Target::LowCut;
    }
    if contains(app.layout.record, x, y) {
        return Target::Record;
    }
    if contains(app.layout.original, x, y) {
        return Target::Original;
    }
    if contains(app.layout.cancelled, x, y) {
        return Target::Cancelled;
    }
    if contains(app.layout.tray, x, y) {
        return Target::Tray;
    }
    if contains(app.layout.status, x, y) {
        return Target::Status;
    }
    if contains(app.layout.install_cable, x, y) {
        return Target::InstallCable;
    }
    Target::None
}

fn strength_at(app: &App, x: i32) -> f32 {
    let width = (app.layout.slider.right - app.layout.slider.left).max(1) as f32;
    ((x - app.layout.slider.left) as f32 / width).clamp(0.0, 1.0)
}

fn contains(area: RECT, x: i32, y: i32) -> bool {
    x >= area.left && x < area.right && y >= area.top && y < area.bottom
}

fn point(lparam: LPARAM) -> (i32, i32) {
    let packed = lparam as u32;
    let x = packed as u16 as i16 as i32;
    let y = (packed >> 16) as u16 as i16 as i32;
    (x, y)
}

fn make_fonts(dpi: u32) -> Fonts {
    Fonts {
        title: font(22, 600, dpi),
        body: font(15, 400, dpi),
        small: font(11, 600, dpi),
    }
}

fn font(size: i32, weight: i32, dpi: u32) -> windows_sys::Win32::Graphics::Gdi::HFONT {
    let face = wide("Segoe UI");
    unsafe {
        let created = CreateFontW(
            -(size * dpi as i32 / 96),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            face.as_ptr(),
        );
        if created.is_null() {
            GetStockObject(DEFAULT_GUI_FONT) as _
        } else {
            created
        }
    }
}

fn drop_fonts(fonts: &Fonts) {
    unsafe {
        DeleteObject(fonts.title);
        DeleteObject(fonts.body);
        DeleteObject(fonts.small);
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
