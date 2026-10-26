use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, LineTo, MoveToEx, RoundRect, SelectObject,
    SetBkMode, SetTextColor, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK, PS_SOLID, TRANSPARENT,
};
use windows_sys::Win32::Graphics::Gdi::{HDC, HFONT};

pub struct Fonts {
    pub title: HFONT,
    pub body: HFONT,
    pub small: HFONT,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Target {
    None,
    Mic,
    Toggle,
    Slider,
    Clarity,
    Leveler,
    LowCut,
    Record,
    Original,
    Cancelled,
    Tray,
    Status,
    InstallCable,
    Row(usize),
}

#[derive(Clone)]
pub struct Frame {
    pub devices: Vec<String>,
    pub selected: String,
    pub list_open: bool,
    pub hover: Target,
    pub input_level: f32,
    pub output_level: f32,
    pub history: Vec<f32>,
    pub cancel: bool,
    pub strength: f32,
    pub clarity_boost: bool,
    pub voice_leveler: bool,
    pub low_cut: bool,
    pub voice_active: bool,
    pub recording: bool,
    pub progress: f32,
    pub has_clip: bool,
    pub raw_bars: Vec<f32>,
    pub clean_bars: Vec<f32>,
    pub playing: u32,
    pub status: String,
    pub tray: bool,
    pub live: bool,
}

#[derive(Clone, Copy)]
pub struct Layout {
    pub mic: RECT,
    pub toggle: RECT,
    pub slider: RECT,
    pub clarity: RECT,
    pub leveler: RECT,
    pub low_cut: RECT,
    pub record: RECT,
    pub original: RECT,
    pub cancelled: RECT,
    pub tray: RECT,
    pub status: RECT,
    pub install_cable: RECT,
    pub rows: [RECT; 8],
    pub row_count: usize,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            mic: empty(),
            toggle: empty(),
            slider: empty(),
            clarity: empty(),
            leveler: empty(),
            low_cut: empty(),
            record: empty(),
            original: empty(),
            cancelled: empty(),
            tray: empty(),
            status: empty(),
            install_cable: empty(),
            rows: [empty(); 8],
            row_count: 0,
        }
    }
}

pub fn paint(hdc: HDC, width: i32, height: i32, dpi: u32, fonts: &Fonts, frame: &Frame) -> Layout {
    let mut layout = Layout::default();
    let s = |value: i32| value * dpi as i32 / 96;
    fill_rect(hdc, 0, 0, width, height, rgb(16, 18, 22));

    let pad = s(22);
    let mut y = s(20);
    text(hdc, fonts.title, pad, y, width - pad * 2, s(32), rgb(244, 247, 250), DT_LEFT, "Frisp");
    let dot = if frame.live { rgb(45, 196, 160) } else { rgb(232, 168, 72) };
    fill_round(hdc, width - pad - s(10), y + s(10), s(10), s(10), s(10), dot, dot);
    y += s(34);
    text(
        hdc,
        fonts.body,
        pad,
        y,
        width - pad * 2,
        s(22),
        rgb(154, 164, 176),
        DT_LEFT,
        "Noise-cancelling virtual microphone",
    );
    y += s(36);

    text(hdc, fonts.small, pad, y, width - pad * 2, s(16), rgb(154, 164, 176), DT_LEFT, "MICROPHONE");
    y += s(20);
    layout.mic = rect(pad, y, width - pad, y + s(44));
    let mic_hover = frame.hover == Target::Mic;
    fill_round(
        hdc,
        layout.mic.left,
        layout.mic.top,
        layout.mic.right - layout.mic.left,
        layout.mic.bottom - layout.mic.top,
        s(12),
        if mic_hover { rgb(40, 48, 58) } else { rgb(28, 33, 40) },
        rgb(58, 68, 80),
    );
    text(
        hdc,
        fonts.body,
        layout.mic.left + s(14),
        layout.mic.top,
        layout.mic.right - layout.mic.left - s(40),
        layout.mic.bottom - layout.mic.top,
        rgb(236, 239, 244),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        &frame.selected,
    );
    text(
        hdc,
        fonts.body,
        layout.mic.right - s(28),
        layout.mic.top,
        s(18),
        layout.mic.bottom - layout.mic.top,
        rgb(154, 164, 176),
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        if frame.list_open { "–" } else { "+" },
    );
    y += s(56);

    text(hdc, fonts.small, pad, y, s(160), s(16), rgb(154, 164, 176), DT_LEFT, "INBOUND");
    text(
        hdc,
        fonts.small,
        pad,
        y,
        width - pad * 2,
        s(16),
        rgb(45, 196, 160),
        DT_RIGHT,
        &format!("{:.0} dB", db(frame.input_level)),
    );
    y += s(20);
    meter(hdc, pad, y, width - pad * 2, s(16), frame.input_level, rgb(45, 196, 160), s(8));
    y += s(24);
    spark(hdc, pad, y, width - pad * 2, s(36), &frame.history, rgb(45, 196, 160));
    y += s(48);

    text(hdc, fonts.small, pad, y, s(160), s(16), rgb(154, 164, 176), DT_LEFT, "FRISP MIC");
    if frame.voice_active {
        let badge_x = pad + s(76);
        let badge_w = s(72);
        let badge_h = s(16);
        fill_round(hdc, badge_x, y, badge_w, badge_h, s(8), rgb(20, 60, 45), rgb(45, 196, 160));
        text(
            hdc,
            fonts.small,
            badge_x,
            y,
            badge_w,
            badge_h,
            rgb(60, 230, 180),
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            "SPEECH",
        );
    }
    text(
        hdc,
        fonts.small,
        pad,
        y,
        width - pad * 2,
        s(16),
        rgb(110, 176, 255),
        DT_RIGHT,
        &format!("{:.0} dB", db(frame.output_level)),
    );
    y += s(20);
    meter(hdc, pad, y, width - pad * 2, s(16), frame.output_level, rgb(110, 176, 255), s(8));
    y += s(32);

    layout.toggle = rect(pad, y, width - pad, y + s(36));
    let knob = s(22);
    let track_w = s(46);
    let track_h = s(26);
    let track_x = layout.toggle.left;
    let track_y = layout.toggle.top + (s(36) - track_h) / 2;
    fill_round(
        hdc,
        track_x,
        track_y,
        track_w,
        track_h,
        track_h,
        if frame.cancel { rgb(45, 196, 160) } else { rgb(52, 58, 68) },
        if frame.cancel { rgb(45, 196, 160) } else { rgb(52, 58, 68) },
    );
    let knob_x = if frame.cancel {
        track_x + track_w - knob - s(2)
    } else {
        track_x + s(2)
    };
    fill_round(hdc, knob_x, track_y + s(2), knob, knob, knob, rgb(244, 247, 250), rgb(244, 247, 250));
    text(
        hdc,
        fonts.body,
        track_x + track_w + s(12),
        layout.toggle.top,
        width - pad * 2 - track_w - s(12),
        s(36),
        rgb(236, 239, 244),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        "Noise cancellation",
    );
    y += s(48);

    text(hdc, fonts.small, pad, y, s(180), s(16), rgb(154, 164, 176), DT_LEFT, "CANCELLATION");
    text(
        hdc,
        fonts.small,
        pad,
        y,
        width - pad * 2,
        s(16),
        rgb(236, 239, 244),
        DT_RIGHT,
        &format!("{:.0}%", frame.strength.clamp(0.0, 1.0) * 100.0),
    );
    y += s(22);
    layout.slider = rect(pad, y, width - pad, y + s(28));
    let track_top = layout.slider.top + s(11);
    fill_round(hdc, layout.slider.left, track_top, width - pad * 2, s(6), s(6), rgb(40, 46, 54), rgb(40, 46, 54));
    let filled = ((width - pad * 2) as f32 * frame.strength.clamp(0.0, 1.0)) as i32;
    if filled > s(6) {
        fill_round(hdc, layout.slider.left, track_top, filled, s(6), s(6), rgb(45, 196, 160), rgb(45, 196, 160));
    }
    let knob_size = s(18);
    let knob_left = (layout.slider.left + filled - knob_size / 2).clamp(layout.slider.left, layout.slider.right - knob_size);
    fill_round(
        hdc,
        knob_left,
        layout.slider.top + s(5),
        knob_size,
        knob_size,
        knob_size,
        rgb(244, 247, 250),
        rgb(45, 196, 160),
    );
    y += s(38);

    text(hdc, fonts.small, pad, y, width - pad * 2, s(16), rgb(154, 164, 176), DT_LEFT, "KRISP AUDIO ENHANCEMENTS");
    y += s(20);
    let chip_gap = s(8);
    let total_w = width - pad * 2;
    let chip_w = (total_w - chip_gap * 2) / 3;
    let chip_h = s(28);

    // Chip 1: Clarity (HVC)
    layout.clarity = rect(pad, y, pad + chip_w, y + chip_h);
    let c_hover = frame.hover == Target::Clarity;
    let (c_bg, c_border, c_fg) = if frame.clarity_boost {
        (if c_hover { rgb(32, 72, 56) } else { rgb(22, 56, 44) }, rgb(45, 196, 160), rgb(236, 255, 248))
    } else {
        (if c_hover { rgb(38, 44, 54) } else { rgb(28, 33, 40) }, rgb(52, 60, 72), rgb(130, 140, 155))
    };
    fill_round(hdc, layout.clarity.left, layout.clarity.top, chip_w, chip_h, s(8), c_bg, c_border);
    let c_label = if frame.clarity_boost { "✓ Voice Clarity" } else { "Voice Clarity" };
    text(hdc, fonts.small, layout.clarity.left, layout.clarity.top, chip_w, chip_h, c_fg, DT_CENTER | DT_VCENTER | DT_SINGLELINE, c_label);

    // Chip 2: Voice Leveler (volumeUp)
    let lev_left = pad + chip_w + chip_gap;
    layout.leveler = rect(lev_left, y, lev_left + chip_w, y + chip_h);
    let l_hover = frame.hover == Target::Leveler;
    let (l_bg, l_border, l_fg) = if frame.voice_leveler {
        (if l_hover { rgb(32, 72, 56) } else { rgb(22, 56, 44) }, rgb(45, 196, 160), rgb(236, 255, 248))
    } else {
        (if l_hover { rgb(38, 44, 54) } else { rgb(28, 33, 40) }, rgb(52, 60, 72), rgb(130, 140, 155))
    };
    fill_round(hdc, layout.leveler.left, layout.leveler.top, chip_w, chip_h, s(8), l_bg, l_border);
    let l_label = if frame.voice_leveler { "✓ Auto-Level" } else { "Auto-Level" };
    text(hdc, fonts.small, layout.leveler.left, layout.leveler.top, chip_w, chip_h, l_fg, DT_CENTER | DT_VCENTER | DT_SINGLELINE, l_label);

    // Chip 3: Low-Cut Filter (80Hz rumble)
    let lc_left = pad + (chip_w + chip_gap) * 2;
    layout.low_cut = rect(lc_left, y, lc_left + chip_w, y + chip_h);
    let lc_hover = frame.hover == Target::LowCut;
    let (lc_bg, lc_border, lc_fg) = if frame.low_cut {
        (if lc_hover { rgb(32, 72, 56) } else { rgb(22, 56, 44) }, rgb(45, 196, 160), rgb(236, 255, 248))
    } else {
        (if lc_hover { rgb(38, 44, 54) } else { rgb(28, 33, 40) }, rgb(52, 60, 72), rgb(130, 140, 155))
    };
    fill_round(hdc, layout.low_cut.left, layout.low_cut.top, chip_w, chip_h, s(8), lc_bg, lc_border);
    let lc_label = if frame.low_cut { "✓ Rumble Cut" } else { "Rumble Cut" };
    text(hdc, fonts.small, layout.low_cut.left, layout.low_cut.top, chip_w, chip_h, lc_fg, DT_CENTER | DT_VCENTER | DT_SINGLELINE, lc_label);

    y += s(38);

    let card = rect(pad, y, width - pad, y + s(210));
    fill_round(hdc, card.left, card.top, card.right - card.left, card.bottom - card.top, s(16), rgb(24, 28, 34), rgb(48, 56, 66));
    text(hdc, fonts.small, card.left + s(16), card.top + s(14), card.right - card.left - s(32), s(16), rgb(154, 164, 176), DT_LEFT, "HEAR THE DIFFERENCE");
    layout.record = rect(card.left + s(16), card.top + s(38), card.right - s(16), card.top + s(76));
    let record_fill = if frame.recording { rgb(176, 64, 64) } else { rgb(45, 196, 160) };
    fill_round(
        hdc,
        layout.record.left,
        layout.record.top,
        layout.record.right - layout.record.left,
        layout.record.bottom - layout.record.top,
        s(10),
        record_fill,
        record_fill,
    );
    let record_label = if frame.recording {
        format!("Recording  {:.0}%", frame.progress.clamp(0.0, 1.0) * 100.0)
    } else {
        "Record 4 seconds".to_string()
    };
    text(
        hdc,
        fonts.body,
        layout.record.left,
        layout.record.top,
        layout.record.right - layout.record.left,
        layout.record.bottom - layout.record.top,
        rgb(12, 18, 16),
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        &record_label,
    );
    let gap = s(10);
    let half = (card.right - card.left - s(32) - gap) / 2;
    layout.original = rect(card.left + s(16), card.top + s(86), card.left + s(16) + half, card.bottom - s(14));
    layout.cancelled = rect(layout.original.right + gap, layout.original.top, layout.original.right + gap + half, layout.original.bottom);
    play_card(hdc, fonts, layout.original, "Original", &frame.raw_bars, frame.playing == 1, frame.has_clip, frame.hover == Target::Original, dpi);
    play_card(hdc, fonts, layout.cancelled, "Cancelled", &frame.clean_bars, frame.playing == 2, frame.has_clip, frame.hover == Target::Cancelled, dpi);
    y = card.bottom + s(16);

    let status_color = if frame.live { rgb(45, 196, 160) } else { rgb(232, 168, 72) };
    layout.status = rect(pad, y, width - pad, y + s(30));
    fill_round(hdc, pad, y + s(4), s(8), s(8), s(8), status_color, status_color);
    text(hdc, fonts.small, pad + s(16), y, width - pad * 2 - s(16), s(30), rgb(210, 216, 224), DT_LEFT | DT_WORDBREAK, &frame.status);
    y += s(34);

    if !frame.live {
        let btn_h = s(34);
        layout.install_cable = rect(pad, y, width - pad, y + btn_h);
        let btn_hover = frame.hover == Target::InstallCable;
        let (btn_bg, btn_border, btn_fg) = if btn_hover {
            (rgb(36, 80, 62), rgb(52, 220, 180), rgb(255, 255, 255))
        } else {
            (rgb(26, 56, 44), rgb(45, 196, 160), rgb(236, 255, 248))
        };
        fill_round(hdc, layout.install_cable.left, layout.install_cable.top, width - pad * 2, btn_h, s(8), btn_bg, btn_border);
        text(
            hdc,
            fonts.body,
            layout.install_cable.left,
            layout.install_cable.top,
            width - pad * 2,
            btn_h,
            btn_fg,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            "⚡ Install VB-CABLE Driver",
        );
        y += btn_h + s(12);
    } else {
        layout.install_cable = empty();
    }

    layout.tray = rect(pad, y, width - pad, y + s(24));
    let box_sz = s(16);
    let box_y = layout.tray.top + s(4);
    fill_round(
        hdc,
        layout.tray.left,
        box_y,
        box_sz,
        box_sz,
        s(4),
        if frame.tray { rgb(45, 196, 160) } else { rgb(28, 33, 40) },
        if frame.tray { rgb(45, 196, 160) } else { rgb(90, 100, 112) },
    );
    if frame.tray {
        text(hdc, fonts.small, layout.tray.left, box_y - s(1), box_sz, box_sz, rgb(12, 18, 16), DT_CENTER | DT_VCENTER | DT_SINGLELINE, "✓");
    }
    text(
        hdc,
        fonts.body,
        layout.tray.left + s(24),
        layout.tray.top,
        width - pad * 2 - s(24),
        s(24),
        rgb(210, 216, 224),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        "Minimize to the system tray",
    );
    y = layout.tray.bottom + s(12);

    text(
        hdc,
        fonts.small,
        pad,
        y,
        width - pad * 2,
        s(22),
        rgb(120, 128, 138),
        DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
        "In OBS, Streamyard, or any browser, choose Frisp Mic.",
    );

    if frame.list_open {
        let shown = frame.devices.len().min(layout.rows.len());
        let row_h = s(36);
        let panel_h = s(8) + row_h * shown as i32;
        let panel = rect(layout.mic.left, layout.mic.bottom + s(6), layout.mic.right, layout.mic.bottom + s(6) + panel_h.max(s(36)));
        fill_round(hdc, panel.left, panel.top, panel.right - panel.left, panel.bottom - panel.top, s(12), rgb(32, 38, 46), rgb(70, 82, 96));
        if shown == 0 {
            text(hdc, fonts.body, panel.left + s(12), panel.top, panel.right - panel.left - s(24), panel.bottom - panel.top, rgb(154, 164, 176), DT_LEFT | DT_VCENTER | DT_SINGLELINE, "No microphones found");
        }
        layout.row_count = shown;
        for index in 0..shown {
            let top = panel.top + s(4) + row_h * index as i32;
            let row = rect(panel.left + s(4), top, panel.right - s(4), top + row_h);
            layout.rows[index] = row;
            if frame.hover == Target::Row(index) {
                fill_round(hdc, row.left, row.top, row.right - row.left, row.bottom - row.top, s(8), rgb(45, 62, 58), rgb(45, 62, 58));
            }
            text(
                hdc,
                fonts.body,
                row.left + s(10),
                row.top,
                row.right - row.left - s(20),
                row.bottom - row.top,
                rgb(236, 239, 244),
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                &frame.devices[index],
            );
        }
    }
    let _ = height;
    layout
}

pub fn bars(samples: &[f32], count: usize) -> Vec<f32> {
    if count == 0 {
        return Vec::new();
    }
    if samples.is_empty() {
        return vec![0.0; count];
    }
    let step = (samples.len() / count).max(1);
    (0..count)
        .map(|index| {
            let start = index * step;
            let end = (start + step).min(samples.len());
            samples[start..end].iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()))
        })
        .collect()
}

fn play_card(hdc: HDC, fonts: &Fonts, area: RECT, label: &str, bars: &[f32], playing: bool, enabled: bool, hover: bool, dpi: u32) {
    let s = |v: i32| v * dpi as i32 / 96;
    let radius = s(8);
    let fill = if playing {
        rgb(36, 64, 58)
    } else if hover && enabled {
        rgb(40, 48, 58)
    } else {
        rgb(30, 35, 42)
    };
    let border = if playing { rgb(45, 196, 160) } else { rgb(58, 68, 80) };
    fill_round(hdc, area.left, area.top, area.right - area.left, area.bottom - area.top, radius, fill, border);
    let color = if enabled { rgb(236, 239, 244) } else { rgb(130, 140, 155) };
    text(hdc, fonts.small, area.left + s(12), area.top + s(10), area.right - area.left - s(24), s(20), color, DT_LEFT | DT_VCENTER | DT_SINGLELINE, label);
    let wave_top = area.top + s(34);
    let wave_bottom = area.bottom - s(10);
    let wave_left = area.left + s(10);
    let wave_right = area.right - s(10);
    if bars.is_empty() || !enabled {
        text(hdc, fonts.small, wave_left, wave_top, wave_right - wave_left, wave_bottom - wave_top, rgb(90, 98, 108), DT_CENTER | DT_VCENTER | DT_SINGLELINE, "Play");
        return;
    }
    let count = bars.len() as i32;
    let width = (wave_right - wave_left).max(1);
    let mid = (wave_top + wave_bottom) / 2;
    let half = (wave_bottom - wave_top) / 2;
    let color = if playing { rgb(45, 196, 160) } else { rgb(110, 196, 176) };
    unsafe {
        let pen = CreatePen(PS_SOLID, s(2).max(1), color);
        let old = SelectObject(hdc, pen);
        for index in 0..count {
            let x = wave_left + width * index / count;
            let magnitude = bars[index as usize].clamp(0.0, 1.0);
            let rise = (half as f32 * magnitude.max(0.08)) as i32;
            MoveToEx(hdc, x, mid - rise, std::ptr::null_mut());
            LineTo(hdc, x, mid + rise);
        }
        SelectObject(hdc, old);
        DeleteObject(pen);
    }
}

fn meter(hdc: HDC, x: i32, y: i32, width: i32, height: i32, level: f32, color: u32, radius: i32) {
    fill_round(hdc, x, y, width, height, radius, rgb(32, 38, 46), rgb(32, 38, 46));
    let filled = (width as f32 * level.clamp(0.0, 1.0)) as i32;
    if filled > radius {
        fill_round(hdc, x, y, filled, height, radius, color, color);
    }
}

fn spark(hdc: HDC, x: i32, y: i32, width: i32, height: i32, history: &[f32], color: u32) {
    fill_round(hdc, x, y, width, height, 8, rgb(24, 28, 34), rgb(40, 48, 56));
    if history.len() < 2 {
        return;
    }
    unsafe {
        let pen = CreatePen(PS_SOLID, 2, color);
        let old = SelectObject(hdc, pen);
        let last = history.len() - 1;
        for (index, sample) in history.iter().enumerate() {
            let px = x + 6 + (width - 12) * index as i32 / last.max(1) as i32;
            let py = y + height - 4 - ((height - 8) as f32 * sample.clamp(0.0, 1.0)) as i32;
            if index == 0 {
                MoveToEx(hdc, px, py, std::ptr::null_mut());
            } else {
                LineTo(hdc, px, py);
            }
        }
        SelectObject(hdc, old);
        DeleteObject(pen);
    }
}

fn fill_round(hdc: HDC, x: i32, y: i32, width: i32, height: i32, radius: i32, fill: u32, border: u32) {
    if width <= 0 || height <= 0 {
        return;
    }
    unsafe {
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_brush = SelectObject(hdc, brush);
        let old_pen = SelectObject(hdc, pen);
        RoundRect(hdc, x, y, x + width, y + height, radius, radius);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}

fn fill_rect(hdc: HDC, x: i32, y: i32, width: i32, height: i32, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let old = SelectObject(hdc, brush);
        windows_sys::Win32::Graphics::Gdi::Rectangle(hdc, x, y, x + width, y + height);
        SelectObject(hdc, old);
        DeleteObject(brush);
    }
}

fn text(hdc: HDC, font: HFONT, x: i32, y: i32, width: i32, height: i32, color: u32, format: u32, value: &str) {
    let wide = wide(value);
    let mut area = rect(x, y, x + width, y + height);
    unsafe {
        SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, color);
        DrawTextW(hdc, wide.as_ptr(), -1, &mut area, format);
    }
}

fn db(level: f32) -> f32 {
    if level <= 0.0001 {
        return -60.0;
    }
    (20.0 * level.log10()).clamp(-60.0, 0.0)
}

fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT { left, top, right, bottom }
}

fn empty() -> RECT {
    rect(0, 0, 0, 0)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
