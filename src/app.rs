use crate::formats::{self, abr, folder};
use crate::schema::BrushSet;
use eframe::egui;
use std::path::PathBuf;

pub struct App {
    brush_set: Option<BrushSet>,
    textures: Vec<egui::TextureHandle>,
    status: String,
    error: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        App {
            brush_set: None,
            textures: Vec::new(),
            status: "Drop a brush file here, or click \"Open file…\"".to_string(),
            error: None,
        }
    }
}

impl App {
    fn load(&mut self, ctx: &egui::Context, path: PathBuf, bytes: Vec<u8>) {
        match formats::import_auto(&path, &bytes) {
            Ok(brush_set) => {
                self.textures = brush_set
                    .brushes
                    .iter()
                    .enumerate()
                    .map(|(i, brush)| {
                        ctx.load_texture(
                            format!("brush-tip-{i}"),
                            brush.tip.to_color_image(),
                            egui::TextureOptions::LINEAR,
                        )
                    })
                    .collect();
                self.status = format!(
                    "Loaded {} brush(es) from {}",
                    brush_set.brushes.len(),
                    path.display()
                );
                self.error = None;
                self.brush_set = Some(brush_set);
            }
            Err(err) => {
                self.error = Some(err.to_string());
            }
        }
    }

    fn open_file_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open brush file")
            .add_filter(
                "Brush files",
                &["brush", "brushset", "abr", "sut", "kbr", "bundle"],
            )
            .add_filter("All files", &["*"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => self.load(ctx, path, bytes),
            Err(err) => self.error = Some(err.to_string()),
        }
    }

    fn export_as_abr(&mut self) {
        let Some(brush_set) = &self.brush_set else { return };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("export.abr")
            .add_filter("Photoshop Brush", &["abr"])
            .save_file()
        else {
            return;
        };
        match abr::export(brush_set, &path) {
            Ok(()) => {
                self.status = format!("Exported to {}", path.display());
                self.error = None;
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }

    fn export_as_folder(&mut self) {
        let Some(brush_set) = &self.brush_set else { return };
        let Some(dir) = rfd::FileDialog::new()
            .set_title("Choose export folder")
            .pick_folder()
        else {
            return;
        };
        match folder::export(brush_set, &dir) {
            Ok(()) => {
                self.status = format!("Exported to {}", dir.display());
                self.error = None;
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(file) = dropped.first() {
            if let Some(path) = file.path.clone() {
                match std::fs::read(&path) {
                    Ok(bytes) => self.load(ctx, path, bytes),
                    Err(err) => self.error = Some(err.to_string()),
                }
            } else if let Some(bytes) = &file.bytes {
                self.load(ctx, PathBuf::from(&file.name), bytes.to_vec());
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open file…").clicked() {
                    self.open_file_dialog(ctx);
                }
                ui.label(&self.status);
            });
            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::RED, err);
            }
            ui.separator();

            if let Some(brush_set) = &self.brush_set {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("brush_grid")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            for (brush, texture) in brush_set.brushes.iter().zip(&self.textures) {
                                let display_size = egui::vec2(64.0, 64.0);
                                ui.add(egui::Image::new(texture).fit_to_exact_size(display_size));
                                ui.label(&brush.name);
                                ui.end_row();
                            }
                        });
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Export as .abr…").clicked() {
                        self.export_as_abr();
                    }
                    if ui.button("Export as folder…").clicked() {
                        self.export_as_folder();
                    }
                });
            } else {
                ui.label("(drag and drop a brush file, or click \"Open file…\")");
            }
        });
    }
}
