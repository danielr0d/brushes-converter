use brushes_converter::app::{apply_style, App};
use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Brush Converter")
            .with_inner_size(egui::vec2(880.0, 640.0))
            .with_min_inner_size(egui::vec2(560.0, 420.0)),
        ..Default::default()
    };
    eframe::run_native(
        "Brush Converter",
        options,
        Box::new(|cc| {
            apply_style(&cc.egui_ctx);
            Box::new(App::default())
        }),
    )
}
