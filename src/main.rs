mod config;
mod engine;

use config::Config;
use engine::{default_input_name, list_inputs, Engine};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Frisp")
            .with_inner_size([460.0, 560.0])
            .with_min_inner_size([400.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Frisp",
        options,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(FrispApp::new()))
        }),
    )
}

struct FrispApp {
    config: Config,
    devices: Vec<String>,
    selected: Option<String>,
    noise_cancel: bool,
    status: String,
    input_level: f32,
    output_level: f32,
    engine: Engine,
    shared: Arc<engine::Shared>,
    last_refresh: Instant,
}

impl FrispApp {
    fn new() -> Self {
        let config = Config::load();
        let (engine, shared) = Engine::spawn(config.noise_cancel);
        let devices = list_inputs();
        let selected = config
            .input_device
            .clone()
            .filter(|name| devices.iter().any(|device| device == name))
            .or_else(default_input_name)
            .filter(|name| devices.iter().any(|device| device == name));
        engine.set_input(selected.clone());
        Self {
            noise_cancel: config.noise_cancel,
            config,
            devices,
            selected,
            status: "Starting.".into(),
            input_level: 0.0,
            output_level: 0.0,
            engine,
            shared,
            last_refresh: Instant::now(),
        }
    }

    fn refresh_devices(&mut self) {
        self.devices = list_inputs();
        if let Some(selected) = &self.selected {
            if !self.devices.iter().any(|device| device == selected) {
                self.selected = None;
                self.engine.set_input(None);
                self.config.input_device = None;
                self.config.save();
            }
        }
        self.last_refresh = Instant::now();
    }
}

impl eframe::App for FrispApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_theme(ctx);
        ctx.request_repaint_after(Duration::from_millis(33));
        if self.last_refresh.elapsed() > Duration::from_secs(1) {
            self.refresh_devices();
        }
        if let Ok(status) = self.shared.status.lock() {
            self.status = status.clone();
        }
        let measured_in = f32::from_bits(self.shared.input_peak.load(Ordering::Relaxed));
        let measured_out = f32::from_bits(self.shared.output_peak.load(Ordering::Relaxed));
        self.input_level = (self.input_level * 0.82).max(measured_in.min(1.0));
        self.output_level = (self.output_level * 0.82).max(measured_out.min(1.0));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            ui.heading("Frisp");
            ui.label(
                egui::RichText::new("Noise-cancelling virtual microphone")
                    .color(egui::Color32::from_rgb(168, 176, 184)),
            );
            ui.add_space(18.0);

            ui.label("Microphone");
            let selected_text = self
                .selected
                .clone()
                .unwrap_or_else(|| "Select a microphone".to_string());
            let mut picked = self.selected.clone();
            egui::ComboBox::from_id_salt("microphone")
                .width(ui.available_width())
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    if self.devices.is_empty() {
                        ui.label("No microphones found");
                    }
                    for name in &self.devices {
                        ui.selectable_value(&mut picked, Some(name.clone()), name);
                    }
                });
            if picked != self.selected {
                self.selected = picked;
                self.engine.set_input(self.selected.clone());
                self.config.input_device = self.selected.clone();
                self.config.save();
            }

            ui.add_space(12.0);
            if ui
                .checkbox(&mut self.noise_cancel, "Noise cancellation")
                .changed()
            {
                self.shared
                    .noise_cancel
                    .store(self.noise_cancel, Ordering::Relaxed);
                self.config.noise_cancel = self.noise_cancel;
                self.config.save();
            }

            ui.add_space(18.0);
            meter(ui, "Source", self.input_level);
            ui.add_space(8.0);
            meter(ui, "Frisp Mic", self.output_level);

            ui.add_space(18.0);
            let live = self.shared.virtual_ready.load(Ordering::Relaxed);
            let color = if live {
                egui::Color32::from_rgb(47, 158, 132)
            } else {
                egui::Color32::from_rgb(196, 122, 74)
            };
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 5.0, color);
                ui.label(&self.status);
            });

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.label(
                    egui::RichText::new("0.1.0")
                        .size(12.0)
                        .color(egui::Color32::from_rgb(120, 128, 136)),
                );
                ui.add_space(6.0);
                ui.label("In OBS, Streamyard, or any browser, choose the microphone named Frisp Mic. Leave Frisp open while you use it.");
            });
        });
    }
}

fn meter(ui: &mut egui::Ui, label: &str, level: f32) {
    ui.label(label);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 16.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(32, 38, 44));
    let filled = rect.width() * level.clamp(0.0, 1.0);
    if filled > 0.0 {
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(filled, rect.height()));
        painter.rect_filled(bar, 4.0, egui::Color32::from_rgb(47, 158, 132));
    }
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = egui::Color32::from_rgb(18, 22, 26);
    visuals.panel_fill = egui::Color32::from_rgb(18, 22, 26);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(28, 34, 40);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(36, 44, 51);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(48, 58, 66);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(47, 158, 132);
    visuals.selection.bg_fill = egui::Color32::from_rgb(47, 158, 132);
    visuals.override_text_color = Some(egui::Color32::from_rgb(232, 236, 239));
    ctx.set_visuals(visuals);
}
