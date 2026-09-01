//! Plain folder export: one subdirectory per brush, each holding `tip.png`
//! (and `grain.png` when present) plus a `settings.json` of every
//! non-raster field. No painting app reads this back — it's a lossless-ish
//! escape hatch for hand inspection or feeding into other tooling.

use crate::error::{ConverterError, Result};
use crate::schema::{BrushSet, RasterImage};
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

#[derive(Serialize)]
struct BrushSettings<'a> {
    name: &'a str,
    diameter_px: f32,
    angle_deg: f32,
    roundness_pct: f32,
    spacing_pct: f32,
    interpolate: bool,
    flip_x: bool,
    flip_y: bool,
    pressure_sensitive_size: bool,
    pressure_sensitive_opacity: bool,
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    extra: &'a serde_json::Map<String, serde_json::Value>,
}

pub fn export(brush_set: &BrushSet, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;

    let mut used_names = HashSet::new();
    for (i, brush) in brush_set.brushes.iter().enumerate() {
        let brush_dir = dir.join(unique_folder_name(&brush.name, i, &mut used_names));
        std::fs::create_dir_all(&brush_dir)?;

        write_png(&brush.tip, &brush_dir.join("tip.png"))?;
        if let Some(grain) = &brush.grain {
            write_png(grain, &brush_dir.join("grain.png"))?;
        }

        let settings = BrushSettings {
            name: &brush.name,
            diameter_px: brush.diameter_px,
            angle_deg: brush.angle_deg,
            roundness_pct: brush.roundness_pct,
            spacing_pct: brush.spacing_pct,
            interpolate: brush.interpolate,
            flip_x: brush.flip_x,
            flip_y: brush.flip_y,
            pressure_sensitive_size: brush.pressure_sensitive_size,
            pressure_sensitive_opacity: brush.pressure_sensitive_opacity,
            extra: &brush.extra,
        };
        let json = serde_json::to_string_pretty(&settings)?;
        std::fs::write(brush_dir.join("settings.json"), json)?;
    }
    Ok(())
}

fn write_png(image: &RasterImage, path: &Path) -> Result<()> {
    let buf = image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone())
        .ok_or_else(|| ConverterError::Malformed("raster buffer length doesn't match its width/height".into()))?;
    buf.save(path)?;
    Ok(())
}

/// Filesystem-safe folder name for a brush: its name with anything but
/// alphanumerics/spaces/-/_ replaced, falling back to `brush` if that
/// leaves nothing, disambiguated with `_<index>` on collision (brush names
/// aren't guaranteed unique within a set).
fn unique_folder_name(name: &str, index: usize, used: &mut HashSet<String>) -> String {
    let base = sanitize(name);
    let candidate = if used.contains(&base) {
        format!("{base}_{index}")
    } else {
        base
    };
    used.insert(candidate.clone());
    candidate
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() { "brush".to_string() } else { trimmed.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::IntermediateBrush;

    fn brush(name: &str, with_grain: bool) -> IntermediateBrush {
        IntermediateBrush {
            name: name.to_string(),
            tip: RasterImage { width: 2, height: 2, rgba: vec![255; 2 * 2 * 4] },
            grain: with_grain.then(|| RasterImage { width: 2, height: 2, rgba: vec![128; 2 * 2 * 4] }),
            diameter_px: 64.0,
            angle_deg: 0.0,
            roundness_pct: 100.0,
            spacing_pct: 25.0,
            interpolate: true,
            flip_x: false,
            flip_y: false,
            pressure_sensitive_size: true,
            pressure_sensitive_opacity: false,
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn writes_tip_grain_and_settings_per_brush() {
        let brush_set = BrushSet { brushes: vec![brush("Round Marker", true), brush("Flat/Weird*Name", false)] };
        let dir = tempfile::tempdir().unwrap();

        export(&brush_set, dir.path()).unwrap();

        let marker_dir = dir.path().join("Round Marker");
        assert!(marker_dir.join("tip.png").is_file());
        assert!(marker_dir.join("grain.png").is_file());
        let settings: serde_json::Value =
            serde_json::from_slice(&std::fs::read(marker_dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["name"], "Round Marker");
        assert_eq!(settings["diameter_px"], 64.0);
        assert_eq!(settings["pressure_sensitive_size"], true);

        let flat_dir = dir.path().join("Flat_Weird_Name");
        assert!(flat_dir.join("tip.png").is_file());
        assert!(!flat_dir.join("grain.png").exists());

        let decoded = image::open(marker_dir.join("tip.png")).unwrap();
        assert_eq!(decoded.width(), 2);
        assert_eq!(decoded.height(), 2);
    }

    #[test]
    fn disambiguates_duplicate_names() {
        let brush_set = BrushSet { brushes: vec![brush("Dup", false), brush("Dup", false)] };
        let dir = tempfile::tempdir().unwrap();

        export(&brush_set, dir.path()).unwrap();

        assert!(dir.path().join("Dup").is_dir());
        assert!(dir.path().join("Dup_1").is_dir());
    }
}
