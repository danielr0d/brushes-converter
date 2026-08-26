use brushes_converter::app::App;

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Brush Converter",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Box::new(App::default())),
    )
}
