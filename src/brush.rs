use eframe::egui;

/// Raw RGBA pixel buffer for a brush tip or texture stamp, in a form every
/// format-specific parser produces and `egui` can render directly.
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl RasterImage {
    pub fn to_color_image(&self) -> egui::ColorImage {
        egui::ColorImage::from_rgba_unmultiplied(
            [self.width as usize, self.height as usize],
            &self.rgba,
        )
    }
}

/// Universal brush representation. Every format reader (Procreate, ABR,
/// Clip Studio, ...) converts into this, and every writer converts out of it.
pub struct IntermediateBrush {
    pub name: String,
    pub tip: RasterImage,
    pub texture: Option<RasterImage>,
    pub base_size: f32,
    pub spacing: f32,
    pub pressure_sensitive_size: bool,
    pub pressure_sensitive_opacity: bool,
}
