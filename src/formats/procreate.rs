//! Procreate `.brush` (single brush) / `.brushset` (multiple brushes)
//! importer.
//!
//! Both are ZIP archives. A `.brush` has its payload at the archive root
//! (`Brush.archive`, `Shape.png`, optionally `Grain.png`). A `.brushset` has
//! one such group per brush, each under a `<UUID>/` directory. `Brush.archive`
//! is an NSKeyedArchiver-encoded binary plist (see `crate::nskeyed`) holding
//! ~170 flat scalar dynamics keys plus a `name` string.

use crate::error::{ConverterError, Result};
use crate::nskeyed::{self, ValueExt};
use crate::schema::{BrushSet, IntermediateBrush, RasterImage};
use std::collections::BTreeMap;
use std::io::{Cursor, Read};

#[derive(Default)]
struct BrushFiles {
    archive: Option<usize>,
    shape: Option<usize>,
    grain: Option<usize>,
}

pub fn import(bytes: &[u8]) -> Result<BrushSet> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;

    // Pass 1: group entries by their containing directory ("" for a
    // single .brush's root-level files, "<UUID>" per brush in a .brushset).
    // Insertion order is preserved (a plain Vec keyed by first-seen index)
    // so brushes come out in the same order they appear in the archive.
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, BrushFiles> = BTreeMap::new();
    for i in 0..zip.len() {
        let name = zip.by_index(i)?.name().to_string();
        let (dir, base) = match name.rsplit_once('/') {
            Some((d, b)) => (d.to_string(), b.to_string()),
            None => (String::new(), name.clone()),
        };
        if !groups.contains_key(&dir) {
            order.push(dir.clone());
        }
        let entry = groups.entry(dir).or_default();
        match base.as_str() {
            "Brush.archive" => entry.archive = Some(i),
            "Shape.png" => entry.shape = Some(i),
            "Grain.png" => entry.grain = Some(i),
            _ => {}
        }
    }

    let mut brushes = Vec::new();
    for dir in order {
        let files = &groups[&dir];
        let Some(archive_idx) = files.archive else {
            continue; // not a brush group (e.g. a top-level QuickLook/ folder)
        };
        let mut archive_bytes = Vec::new();
        zip.by_index(archive_idx)?.read_to_end(&mut archive_bytes)?;

        let Some(shape_idx) = files.shape else {
            return Err(ConverterError::Malformed(format!(
                "brush group '{dir}' has Brush.archive but no Shape.png"
            )));
        };
        let mut shape_bytes = Vec::new();
        zip.by_index(shape_idx)?.read_to_end(&mut shape_bytes)?;
        let tip = decode_png(&shape_bytes)?;

        let grain = match files.grain {
            Some(idx) => {
                let mut grain_bytes = Vec::new();
                zip.by_index(idx)?.read_to_end(&mut grain_bytes)?;
                Some(decode_png(&grain_bytes)?)
            }
            None => None,
        };

        brushes.push(brush_from_archive(&archive_bytes, tip, grain)?);
    }

    if brushes.is_empty() {
        return Err(ConverterError::Malformed(
            "no Brush.archive found in Procreate file".into(),
        ));
    }

    Ok(BrushSet { brushes })
}

fn decode_png(bytes: &[u8]) -> Result<RasterImage> {
    let img = image::load_from_memory(bytes)?.into_rgba8();
    let (width, height) = img.dimensions();
    Ok(RasterImage {
        width,
        height,
        rgba: img.into_raw(),
    })
}

fn brush_from_archive(
    archive_bytes: &[u8],
    tip: RasterImage,
    grain: Option<RasterImage>,
) -> Result<IntermediateBrush> {
    let root = nskeyed::decode_root(archive_bytes)?;

    let name = root
        .field_str("name")
        .unwrap_or("Untitled Brush")
        .to_string();

    let shape_angle_rad = root.field_f64("shapeAngle").unwrap_or(0.0);
    let shape_roundness = root.field_f64("shapeRoundness").unwrap_or(1.0);
    let plot_spacing = root.field_f64("plotSpacing").unwrap_or(0.0);
    let pressure_size = root.field_f64("dynamicsPressureSize").unwrap_or(0.0);
    let pressure_opacity = root.field_f64("dynamicsPressureOpacity").unwrap_or(0.0);

    let extra = nskeyed::to_json_object(&root);

    Ok(IntermediateBrush {
        name,
        diameter_px: tip.width as f32,
        tip,
        grain,
        angle_deg: shape_angle_rad.to_degrees() as f32,
        roundness_pct: (shape_roundness * 100.0) as f32,
        spacing_pct: (plot_spacing * 100.0) as f32,
        interpolate: true,
        flip_x: false,
        flip_y: false,
        pressure_sensitive_size: pressure_size > 0.0,
        pressure_sensitive_opacity: pressure_opacity > 0.0,
        extra,
    })
}
