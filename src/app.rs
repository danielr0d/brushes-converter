use crate::formats::{self, abr, folder, krita, procreate};
use crate::schema::BrushSet;
use eframe::egui;
use std::path::PathBuf;

const COLOR_BG: egui::Color32 = egui::Color32::from_rgb(0x17, 0x18, 0x1C);
const COLOR_SURFACE: egui::Color32 = egui::Color32::from_rgb(0x21, 0x23, 0x28);
const COLOR_SURFACE_BORDER: egui::Color32 = egui::Color32::from_rgb(0x33, 0x36, 0x3D);
const COLOR_ACCENT: egui::Color32 = egui::Color32::from_rgb(0x5B, 0x8D, 0xEF);
const COLOR_MUTED_TEXT: egui::Color32 = egui::Color32::from_rgb(0x9A, 0x9E, 0xA6);
const COLOR_ERROR_BG: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x1E, 0x1E);

/// Applies the app's dark theme, spacing, and typography. Call once at startup.
pub fn apply_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = COLOR_BG;
    visuals.window_fill = COLOR_BG;
    visuals.extreme_bg_color = COLOR_SURFACE;
    visuals.faint_bg_color = COLOR_SURFACE;
    visuals.selection.bg_fill = COLOR_ACCENT;
    visuals.hyperlink_color = COLOR_ACCENT;
    visuals.widgets.inactive.rounding = egui::Rounding::same(6.0);
    visuals.widgets.hovered.rounding = egui::Rounding::same(6.0);
    visuals.widgets.active.rounding = egui::Rounding::same(6.0);
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(6.0);
    visuals.window_rounding = egui::Rounding::same(8.0);
    style.visuals = visuals;

    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);

    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(22.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(14.5, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(14.5, egui::FontFamily::Proportional),
    );

    ctx.set_style(style);
}

/// A card-style frame for brush thumbnails: surface fill, subtle border and shadow.
fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(COLOR_SURFACE)
        .stroke(egui::Stroke::new(1.0_f32, COLOR_SURFACE_BORDER))
        .rounding(egui::Rounding::same(10.0))
        .inner_margin(egui::Margin::same(10.0))
        .shadow(egui::epaint::Shadow {
            offset: egui::vec2(0.0, 1.0),
            blur: 6.0,
            spread: 0.0,
            color: egui::Color32::from_black_alpha(60),
        })
}

/// A flat surface frame for the header and export bar strips.
fn bar_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(COLOR_SURFACE)
        .inner_margin(egui::Margin::symmetric(16.0, 10.0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Abr,
    Brushset,
    KritaBundle,
    Folder,
}

impl ExportFormat {
    const ALL: [ExportFormat; 4] = [
        ExportFormat::Abr,
        ExportFormat::Brushset,
        ExportFormat::KritaBundle,
        ExportFormat::Folder,
    ];

    fn label(self) -> &'static str {
        match self {
            ExportFormat::Abr => "Photoshop (.abr)",
            ExportFormat::Brushset => "Procreate (.brushset)",
            ExportFormat::KritaBundle => "Krita bundle (.bundle)",
            ExportFormat::Folder => "Folder (PNG + settings.json)",
        }
    }
}

pub struct App {
    brush_set: Option<BrushSet>,
    textures: Vec<egui::TextureHandle>,
    status: String,
    error: Option<String>,
    export_format: ExportFormat,
}

impl Default for App {
    fn default() -> Self {
        App {
            brush_set: None,
            textures: Vec::new(),
            status: "Drop a brush file here, or click \"Open file…\"".to_string(),
            error: None,
            export_format: ExportFormat::Abr,
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
                &["brush", "brushset", "abr", "sut", "kpp", "bundle"],
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

    fn export_as_brushset(&mut self) {
        let Some(brush_set) = &self.brush_set else { return };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("export.brushset")
            .add_filter("Procreate Brushset", &["brushset"])
            .save_file()
        else {
            return;
        };
        match procreate::export(brush_set, &path) {
            Ok(()) => {
                self.status = format!("Exported to {}", path.display());
                self.error = None;
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }

    fn export_as_krita_bundle(&mut self) {
        let Some(brush_set) = &self.brush_set else { return };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("export.bundle")
            .add_filter("Krita Resource Bundle", &["bundle"])
            .save_file()
        else {
            return;
        };
        match krita::export(brush_set, &path) {
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

    fn export_selected(&mut self) {
        match self.export_format {
            ExportFormat::Abr => self.export_as_abr(),
            ExportFormat::Brushset => self.export_as_brushset(),
            ExportFormat::KritaBundle => self.export_as_krita_bundle(),
            ExportFormat::Folder => self.export_as_folder(),
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

        egui::TopBottomPanel::top("header")
            .exact_height(56.0)
            .frame(bar_frame())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Brush Converter");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Open file…").clicked() {
                            self.open_file_dialog(ctx);
                        }
                    });
                });
            });

        if self.brush_set.is_some() {
            egui::TopBottomPanel::bottom("export_bar")
                .frame(bar_frame())
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Convert into:");
                        egui::ComboBox::new("export_format", "")
                            .selected_text(self.export_format.label())
                            .show_ui(ui, |ui| {
                                for format in ExportFormat::ALL {
                                    ui.selectable_value(
                                        &mut self.export_format,
                                        format,
                                        format.label(),
                                    );
                                }
                            });
                        if ui.button("Export…").clicked() {
                            self.export_selected();
                        }
                    });
                });
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(COLOR_BG)
                    .inner_margin(egui::Margin::same(16.0)),
            )
            .show(ctx, |ui| {
                if let Some(err) = &self.error {
                    egui::Frame::none()
                        .fill(COLOR_ERROR_BG)
                        .rounding(egui::Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                        .show(ui, |ui| {
                            ui.colored_label(ui.visuals().error_fg_color, err);
                        });
                } else {
                    egui::Frame::none()
                        .fill(COLOR_SURFACE)
                        .rounding(egui::Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                        .show(ui, |ui| {
                            ui.colored_label(COLOR_MUTED_TEXT, &self.status);
                        });
                }

                ui.add_space(12.0);

                if let Some(brush_set) = &self.brush_set {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for (brush, texture) in
                                    brush_set.brushes.iter().zip(&self.textures)
                                {
                                    card_frame().show(ui, |ui| {
                                        ui.set_width(100.0);
                                        ui.vertical(|ui| {
                                            ui.add(
                                                egui::Image::new(texture)
                                                    .fit_to_exact_size(egui::vec2(72.0, 72.0)),
                                            );
                                            ui.add_space(4.0);
                                            ui.label(&brush.name);
                                        });
                                    });
                                }
                            });
                        });
                } else {
                    let dragging = ctx.input(|i| !i.raw.hovered_files.is_empty());
                    let border_color = if dragging {
                        COLOR_ACCENT
                    } else {
                        COLOR_SURFACE_BORDER
                    };
                    egui::Frame::none()
                        .fill(COLOR_SURFACE)
                        .stroke(egui::Stroke::new(1.5_f32, border_color))
                        .rounding(egui::Rounding::same(12.0))
                        .inner_margin(egui::Margin::same(32.0))
                        .show(ui, |ui| {
                            ui.set_min_height(220.0);
                            ui.vertical_centered(|ui| {
                                ui.add_space(40.0);
                                ui.colored_label(COLOR_MUTED_TEXT, "Drop a brush file here");
                                ui.add_space(4.0);
                                ui.colored_label(COLOR_MUTED_TEXT, "or");
                                ui.add_space(8.0);
                                if ui.button("Open file…").clicked() {
                                    self.open_file_dialog(ctx);
                                }
                            });
                        });
                }
            });
    }
}
