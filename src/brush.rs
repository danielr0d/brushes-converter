use eframe::egui;

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

pub struct IntermediateBrush {
    pub name: String,
    pub tip: RasterImage,
    pub texture: Option<RasterImage>,
    pub base_size: f32,
    pub spacing: f32,
    pub pressure_sensitive_size: bool,
    pub pressure_sensitive_opacity: bool,
}
