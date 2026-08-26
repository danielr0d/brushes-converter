use eframe::egui;
use serde::{Deserialize, Serialize};

/// A raster image kept as straight (non-premultiplied) RGBA8, used both for
/// GUI previews and as the source data for tip/grain rasters.
#[derive(Clone, Serialize, Deserialize)]
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

    /// Single-channel 8-bit buffer, one byte per pixel, row-major, suitable
    /// for a Photoshop-style sampled-brush mask. Uses the alpha channel when
    /// it carries information (i.e. the source is a stamp cut out on
    /// transparency); otherwise falls back to luminance.
    pub fn to_alpha_mask(&self) -> Vec<u8> {
        let has_alpha_variation = self.rgba.chunks_exact(4).any(|p| p[3] != 255);
        let mut out = Vec::with_capacity((self.width * self.height) as usize);
        for px in self.rgba.chunks_exact(4) {
            let value = if has_alpha_variation {
                px[3]
            } else {
                let (r, g, b) = (px[0] as u32, px[1] as u32, px[2] as u32);
                ((r * 299 + g * 587 + b * 114) / 1000) as u8
            };
            out.push(value);
        }
        out
    }

    pub fn from_alpha_mask(width: u32, height: u32, mask: &[u8]) -> Self {
        let mut rgba = Vec::with_capacity(mask.len() * 4);
        for &v in mask {
            rgba.extend_from_slice(&[255, 255, 255, v]);
        }
        RasterImage { width, height, rgba }
    }
}

/// Format-neutral representation of a single brush preset. Every importer
/// maps into this; every exporter maps out of a subset of it (each target
/// format only supports whatever dynamics fields it understands).
#[derive(Clone, Serialize, Deserialize)]
pub struct IntermediateBrush {
    pub name: String,

    /// Grayscale/alpha stamp raster (the shape that gets stamped along a stroke).
    pub tip: RasterImage,
    /// Optional secondary texture/grain raster.
    pub grain: Option<RasterImage>,

    /// Nominal tip diameter in pixels (derived from the tip raster's own
    /// dimensions, not a format's UI-normalized "size" slider).
    pub diameter_px: f32,
    /// Static rotation, in degrees.
    pub angle_deg: f32,
    /// Static roundness, 0-100 (100 = circular/uncompressed).
    pub roundness_pct: f32,
    /// Stamp spacing as a percentage of diameter.
    pub spacing_pct: f32,

    pub interpolate: bool,
    pub flip_x: bool,
    pub flip_y: bool,

    pub pressure_sensitive_size: bool,
    pub pressure_sensitive_opacity: bool,

    /// Every other format-specific dynamics field the importer could pull
    /// out, kept around for lossless-ish round-tripping and future format
    /// support, keyed by the source format's own field names.
    #[serde(default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BrushSet {
    pub brushes: Vec<IntermediateBrush>,
}
