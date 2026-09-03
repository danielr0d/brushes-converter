//! Photoshop `.abr` support.
//!
//! Forensic inspection of a real CC-era (`version=10, subversion=2`) `.abr`
//! file confirmed the top-level shape: a 4-byte header, then three
//! `8BIM`-tagged, length-prefixed sections in this order:
//!   - `samp` — sampled brush tip rasters, one per GUID (see [`samp`]).
//!   - `patt` — pattern data (empty in every observed real file; we always
//!     write it empty too).
//!   - `desc` — a Photoshop "Descriptor" structure (see [`descriptor`])
//!     holding a `Brsh` key whose value is a `VlLs` list of `brushPreset`
//!     objects, each with a `Nm  ` name and a nested `sampledBrush`
//!     descriptor (`Dmtr`/`Angl`/`Rndn`/`Spcn`/`Intr`/`flipX`/`flipY`/
//!     `sampledData`) — `sampledData` is the GUID that cross-references a
//!     `samp` entry.
//!
//! Real files may have further `8BIM`-tagged sections after `desc` (e.g. a
//! `phry` section observed in the fixture, contents unknown) — the reader
//! skips any section it doesn't recognize. Each section's content is padded
//! to a 4-byte boundary; failing to skip that padding misaligns every
//! section read after an odd-length one (a real file's `desc` section was
//! 47243 bytes, exposing this).
//!
//! The `desc` grammar was verified byte-exact (zero leftover bytes) against
//! a real 47KB `desc` section. The `samp` per-entry layout is our own
//! self-consistent construction inspired by, but not byte-identical to,
//! Adobe's (see `samp` module docs) — real Photoshop compatibility for
//! *written* files is therefore best-effort, not verified.

pub mod descriptor;
pub mod samp;

use crate::error::{ConverterError, Result};
use crate::schema::{BrushSet, IntermediateBrush, RasterImage};
use descriptor::{DescObject, DescValue};
use std::path::Path;

const VERSION: u16 = 10;
const SUBVERSION: u16 = 2;

pub fn export(brush_set: &BrushSet, path: &Path) -> Result<()> {
    std::fs::write(path, export_bytes(brush_set))?;
    Ok(())
}

pub fn export_bytes(brush_set: &BrushSet) -> Vec<u8> {
    let mut samp_entries = Vec::with_capacity(brush_set.brushes.len());
    let mut presets = Vec::with_capacity(brush_set.brushes.len());

    for brush in &brush_set.brushes {
        let guid = uuid::Uuid::new_v4().to_string();

        samp_entries.push(samp::SampEntry {
            guid: guid.clone(),
            width: brush.tip.width,
            height: brush.tip.height,
            mask: brush.tip.to_alpha_mask(),
        });

        presets.push(DescValue::Object(brush_preset_descriptor(brush, &guid)));
    }

    let samp_bytes = samp::write_samp_section(&samp_entries);

    let mut root = DescObject::new("null");
    root.push("Brsh", DescValue::List(presets));
    let desc_bytes = descriptor::write_descriptor(&root);

    let mut out = Vec::new();
    out.extend_from_slice(&VERSION.to_be_bytes());
    out.extend_from_slice(&SUBVERSION.to_be_bytes());
    write_section(&mut out, "samp", &samp_bytes);
    write_section(&mut out, "patt", &[]);
    write_section(&mut out, "desc", &desc_bytes);
    out
}

fn brush_preset_descriptor(brush: &IntermediateBrush, guid: &str) -> DescObject {
    let mut tip = DescObject::new("sampledBrush");
    tip.push("Dmtr", unit_float("#Pxl", brush.diameter_px as f64))
        .push("Angl", unit_float("#Ang", brush.angle_deg as f64))
        .push("Rndn", unit_float("#Prc", brush.roundness_pct as f64))
        .push("Nm  ", DescValue::Text(brush.name.clone()))
        .push("Spcn", unit_float("#Prc", brush.spacing_pct as f64))
        .push("Intr", DescValue::Bool(brush.interpolate))
        .push("flipX", DescValue::Bool(brush.flip_x))
        .push("flipY", DescValue::Bool(brush.flip_y))
        .push("sampledData", DescValue::Text(guid.to_string()));

    let mut preset = DescObject::new("brushPreset");
    preset.push("Nm  ", DescValue::Text(brush.name.clone()));
    preset.push("Brsh", DescValue::Object(tip));
    preset
}

fn unit_float(unit: &str, value: f64) -> DescValue {
    DescValue::UnitFloat { unit: unit.into(), value }
}

fn write_section(out: &mut Vec<u8>, key: &str, content: &[u8]) {
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(&(content.len() as u32).to_be_bytes());
    out.extend_from_slice(content);
    let pad = (4 - content.len() % 4) % 4; // sections are padded to a 4-byte boundary
    out.extend(std::iter::repeat_n(0u8, pad));
}

/// Reads an `.abr` file written by [`export_bytes`] (or, best-effort, one
/// close enough in shape). Not exposed through the GUI — this exists so our
/// own writer output can be validated by reading it back.
pub fn import(bytes: &[u8]) -> Result<BrushSet> {
    if bytes.len() < 4 {
        return Err(ConverterError::Malformed("abr file too short".into()));
    }
    let mut pos = 4; // skip version + subversion
    let mut samp_bytes: Option<&[u8]> = None;
    let mut desc_bytes: Option<&[u8]> = None;

    while pos + 12 <= bytes.len() {
        let tag = &bytes[pos..pos + 4];
        let key = &bytes[pos + 4..pos + 8];
        let len = u32::from_be_bytes(bytes[pos + 8..pos + 12].try_into().unwrap()) as usize;
        pos += 12;
        if tag != b"8BIM" {
            return Err(ConverterError::Malformed(format!(
                "expected 8BIM section tag, found {tag:?}"
            )));
        }
        let content = bytes
            .get(pos..pos + len)
            .ok_or_else(|| ConverterError::Malformed("truncated abr section".into()))?;
        match key {
            b"samp" => samp_bytes = Some(content),
            b"desc" => desc_bytes = Some(content),
            _ => {}
        }
        pos += len;
        pos += (4 - len % 4) % 4; // sections are padded to a 4-byte boundary
    }

    let samp_bytes = samp_bytes.ok_or_else(|| ConverterError::Malformed("missing samp section".into()))?;
    let desc_bytes = desc_bytes.ok_or_else(|| ConverterError::Malformed("missing desc section".into()))?;

    let samp_entries = samp::read_samp_section(samp_bytes)?;
    let root = descriptor::read_descriptor(desc_bytes)?;
    let preset_values = root
        .get("Brsh")
        .and_then(DescValue::as_list)
        .ok_or_else(|| ConverterError::Malformed("desc section missing Brsh list".into()))?;

    let mut brushes = Vec::with_capacity(preset_values.len());
    for preset_value in preset_values {
        let preset = preset_value
            .as_object()
            .ok_or_else(|| ConverterError::Malformed("brush preset is not an object".into()))?;
        brushes.push(brush_from_preset(preset, &samp_entries)?);
    }

    Ok(BrushSet { brushes })
}

fn brush_from_preset(preset: &DescObject, samp_entries: &[samp::SampEntry]) -> Result<IntermediateBrush> {
    let name = preset
        .get("Nm  ")
        .and_then(DescValue::as_text)
        .unwrap_or("Untitled Brush")
        .to_string();

    let tip_desc = preset
        .get("Brsh")
        .and_then(DescValue::as_object)
        .ok_or_else(|| ConverterError::Malformed("brush preset missing Brsh descriptor".into()))?;

    let diameter = tip_desc.get("Dmtr").and_then(DescValue::as_unit_float).map(|(_, v)| v).unwrap_or(0.0);
    let angle = tip_desc.get("Angl").and_then(DescValue::as_unit_float).map(|(_, v)| v).unwrap_or(0.0);
    let roundness = tip_desc.get("Rndn").and_then(DescValue::as_unit_float).map(|(_, v)| v).unwrap_or(100.0);
    let spacing = tip_desc.get("Spcn").and_then(DescValue::as_unit_float).map(|(_, v)| v).unwrap_or(25.0);
    let interpolate = tip_desc.get("Intr").and_then(DescValue::as_bool).unwrap_or(true);
    let flip_x = tip_desc.get("flipX").and_then(DescValue::as_bool).unwrap_or(false);
    let flip_y = tip_desc.get("flipY").and_then(DescValue::as_bool).unwrap_or(false);
    let guid = tip_desc
        .get("sampledData")
        .and_then(DescValue::as_text)
        .ok_or_else(|| ConverterError::Malformed("sampledBrush missing sampledData".into()))?
        .trim_end_matches('\0');

    let entry = samp_entries
        .iter()
        .find(|e| e.guid == guid)
        .ok_or_else(|| ConverterError::Malformed(format!("no samp entry for GUID {guid}")))?;

    let tip = RasterImage::from_alpha_mask(entry.width, entry.height, &entry.mask);

    Ok(IntermediateBrush {
        name,
        tip,
        grain: None,
        diameter_px: diameter as f32,
        angle_deg: angle as f32,
        roundness_pct: roundness as f32,
        spacing_pct: spacing as f32,
        interpolate,
        flip_x,
        flip_y,
        pressure_sensitive_size: false,
        pressure_sensitive_opacity: false,
        extra: serde_json::Map::new(),
    })
}
