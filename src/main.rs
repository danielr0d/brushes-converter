mod brush;

use eframe::egui;

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Brush Converter",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Box::new(App::default())),
    )
}

#[derive(Default)]
struct App {}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Drop a brush file here");
            // TODO: handle ctx.input(|i| i.raw.dropped_files.clone())
        });
    }
}
