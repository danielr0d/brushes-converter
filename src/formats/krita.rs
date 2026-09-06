//! Krita `.bundle` resource-bundle export. A bundle is a zip archive that
//! Krita's own resource manager reads directly: one raster tip PNG per brush
//! under `brushes/`, one paintop preset (`.kpp`) per brush under
//! `paintoppresets/` referencing its tip by filename, plus the OASIS-manifest
//! scaffolding real bundles carry (`mimetype`, `META-INF/manifest.xml`,
//! `meta.xml`, `preview.png`).
//!
//! A `.kpp` preset is itself a PNG: the raster is the preset's UI thumbnail,
//! and the actual settings live in two PNG text chunks — a plain `tEXt`
//! chunk keyed `version` and a zlib-compressed `zTXt` chunk keyed `preset`
//! holding the settings XML (`<Preset paintopid="..." name="...">` wrapping
//! `<param type="..." name="...">value</param>` children, one of which,
//! `brush_definition`, is itself an embedded `<Brush type="png_brush" .../>`
//! fragment referencing the tip by filename+md5sum).
//!
//! Krita has never published a schema for any of this. Everything above was
//! reverse-engineered against real Krita-authored fixtures fetched from the
//! KDE/krita repo itself — `krita/data/bundles/RGBA_brushes.bundle` (a real
//! populated bundle with raster `png_brush` presets) and several `.kpp`
//! files under its test suites — and cross-checked against the matching
//! C++ source (`libs/image/kis_properties_configuration.cc`'s `toXML` for
//! the `<param>` wrapper, `plugins/paintops/libpaintop/kis_brush_option.cpp`
//! for the `brush_definition` wrapping, `libs/brush/kis_predefined_brush_factory.cpp`
//! for the `<Brush>` attribute set, and `libs/ui/KisResourceBundle.cpp` /
//! `libs/resources/tests/data/bundles/test1.bundle` for the manifest/meta
//! shape). Not verified: this writer has never been round-tripped through a
//! real Krita install, only the exact param set a "Pixel brush"
//! (`paintopid="paintbrush"`) needs for a predefined raster tip is emitted —
//! real presets also carry dozens of cosmetic dynamics-curve params (per-
//! sensor curves for mirror/scatter/rotation/etc.) that are left out here,
//! relying on Krita defaulting anything a `<param>` doesn't mention (this
//! fallback behavior is real and confirmed in `fromXML`, not assumed). The
//! brush angle is assumed to be stored in radians (unconfirmed in source;
//! kept consistent with this codebase's Procreate writer, which faced the
//! same ambiguity for its own `shapeAngle` field).
//!
//! Import reads both shapes back: a `.bundle` zip (scanning every
//! `paintoppresets/*.kpp` entry, resolving each preset's raster tip against
//! its sibling `brushes/<filename>` entry by the `brush_definition` param's
//! `filename` attribute) and a bare `.kpp` PNG passed on its own (falling
//! back to the `.kpp`'s own thumbnail raster as the tip, since without a
//! bundle there's nowhere else to resolve the referenced brush file from —
//! this is a real loss of fidelity for a standalone `.kpp`, not just a
//! simplification). Only presets whose `brush_definition` is a
//! `type="png_brush"` (a predefined raster tip stored as a plain PNG) map
//! onto this schema; parametric (`auto_brush`) presets are skipped, and so
//! is `gbr_brush` — confirmed against `RGBA_brushes.bundle`'s own two
//! `gbr_brush` presets that it's used for a `.gih` (GIMP's animated
//! multi-frame "image pipe") filename, not a `.gbr` single-frame brush, and
//! this schema has no multi-frame tip to import it into regardless.
//!
//! The preset XML is parsed with a small hand-rolled scanner rather than a
//! real XML library: real presets (checked against `RGBA_brushes.bundle`'s
//! six shipped presets, see module-level export docs for provenance) nest
//! nothing but opaque CDATA text one level deep — even a param's own nested
//! "sensor" dynamics fragment is stored as a literal string, not as child
//! elements the outer parser ever sees — so a flat scan for `<param
//! type="..." name="...">value</param>` (value either `<![CDATA[...]]>` or,
//! for the one observed non-string case, `bytearray`, plain base64 text) is
//! enough. Confirmed against real files: attribute order is arbitrary (not
//! alphabetical, unlike this module's own writer), and a real `<Brush>`
//! element doesn't always carry `md5sum` — so attribute parsing is a
//! generic name="value" scan, not a fixed-order match.

use crate::error::{ConverterError, Result};
use crate::schema::{BrushSet, IntermediateBrush, RasterImage};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const MIMETYPE: &str = "application/x-krita-resourcebundle";

pub fn import(bytes: &[u8]) -> Result<BrushSet> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let brush = import_standalone_kpp(bytes)?;
        return Ok(BrushSet { brushes: vec![brush] });
    }
    import_bundle(bytes)
}

/// Reads a `.bundle` zip: every `paintoppresets/*.kpp` entry becomes one
/// brush, its tip resolved against `brushes/<filename>` by name.
fn import_bundle(bytes: &[u8]) -> Result<BrushSet> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;

    let mut raster_index: HashMap<String, usize> = HashMap::new();
    let mut preset_indices: Vec<usize> = Vec::new();
    for i in 0..zip.len() {
        let name = zip.by_index(i)?.name().to_string();
        if let Some(base) = name.strip_prefix("brushes/") {
            raster_index.insert(base.to_string(), i);
        } else if name.starts_with("paintoppresets/") && name.ends_with(".kpp") {
            preset_indices.push(i);
        }
    }

    let mut brushes = Vec::new();
    for idx in preset_indices {
        let mut kpp_bytes = Vec::new();
        zip.by_index(idx)?.read_to_end(&mut kpp_bytes)?;

        let preset_xml = read_kpp_preset_xml(&kpp_bytes)?;
        let preset = parse_preset_xml(&preset_xml)?;
        let Some(brush_def) = preset.params.get("brush_definition") else {
            continue; // no raster brush attached to this preset at all
        };
        let brush_attrs = parse_attrs(brush_def);
        if brush_attrs.get("type").map(String::as_str) != Some("png_brush") {
            continue; // parametric/pipe brush — no single raster tip to import
        }

        let tip = match brush_attrs.get("filename").and_then(|f| raster_index.get(f)) {
            Some(&ridx) => {
                let mut raster_bytes = Vec::new();
                zip.by_index(ridx)?.read_to_end(&mut raster_bytes)?;
                decode_png(&raster_bytes)?
            }
            None => decode_png(&kpp_bytes)?, // fall back to the preset's own thumbnail
        };

        brushes.push(brush_from_preset(&preset, &brush_attrs, tip));
    }

    if brushes.is_empty() {
        return Err(ConverterError::Malformed(
            "no importable (raster) presets found in Krita bundle".into(),
        ));
    }
    Ok(BrushSet { brushes })
}

/// Reads a bare `.kpp` PNG with no surrounding bundle. Its own raster is the
/// only tip available, so it's used as-is (see module docs on fidelity).
fn import_standalone_kpp(bytes: &[u8]) -> Result<IntermediateBrush> {
    let preset_xml = read_kpp_preset_xml(bytes)?;
    let preset = parse_preset_xml(&preset_xml)?;
    let brush_attrs = preset
        .params
        .get("brush_definition")
        .map(|s| parse_attrs(s))
        .unwrap_or_default();
    let tip = decode_png(bytes)?;
    Ok(brush_from_preset(&preset, &brush_attrs, tip))
}

/// The settings XML held in a `.kpp`'s zlib-compressed `preset` `zTXt` chunk.
fn read_kpp_preset_xml(bytes: &[u8]) -> Result<String> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let reader = decoder
        .read_info()
        .map_err(|e| ConverterError::Malformed(format!("failed reading .kpp PNG header: {e}")))?;
    let chunk = reader
        .info()
        .compressed_latin1_text
        .iter()
        .find(|t| t.keyword == "preset")
        .ok_or_else(|| ConverterError::Malformed(".kpp file has no 'preset' zTXt chunk".into()))?;
    chunk
        .get_text()
        .map_err(|e| ConverterError::Malformed(format!("failed decompressing .kpp preset chunk: {e}")))
}

struct PresetXml {
    paintopid: String,
    name: String,
    params: HashMap<String, String>,
}

/// Parses `<Preset paintopid="..." name="...">` and its flat `<param
/// type="..." name="...">value</param>` children (see module docs for why a
/// flat scan is enough for this shape).
fn parse_preset_xml(xml: &str) -> Result<PresetXml> {
    let malformed = || ConverterError::Malformed("malformed Krita preset XML".into());

    let preset_tag_end = xml.find('>').ok_or_else(malformed)?;
    let preset_attrs = parse_attrs(&xml[..preset_tag_end]);
    let paintopid = preset_attrs.get("paintopid").cloned().unwrap_or_default();
    let name = preset_attrs.get("name").cloned().unwrap_or_default();

    let mut params = HashMap::new();
    let mut rest = &xml[preset_tag_end + 1..];
    while let Some(tag_start) = rest.find("<param ") {
        let after_tag = &rest[tag_start..];
        let tag_end = after_tag.find('>').ok_or_else(malformed)?;
        let param_attrs = parse_attrs(&after_tag[..tag_end]);
        let param_name = param_attrs.get("name").cloned().unwrap_or_default();

        let content = &after_tag[tag_end + 1..];
        let (value, content_len) = if let Some(cdata) = content.strip_prefix("<![CDATA[") {
            let end = cdata.find("]]>").ok_or_else(malformed)?;
            (cdata[..end].to_string(), "<![CDATA[".len() + end + "]]>".len())
        } else {
            let end = content.find("</param>").ok_or_else(malformed)?;
            (xml_unescape(&content[..end]), end + "</param>".len())
        };
        params.insert(param_name, value);
        rest = &content[content_len..];
    }

    Ok(PresetXml { paintopid, name, params })
}

fn brush_from_preset(
    preset: &PresetXml,
    brush_attrs: &HashMap<String, String>,
    tip: RasterImage,
) -> IntermediateBrush {
    let spacing_frac: f32 = brush_attrs
        .get("spacing")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.1);
    let angle_rad: f64 = brush_attrs
        .get("angle")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let bool_param = |name: &str| preset.params.get(name).map(|v| v == "true").unwrap_or(false);

    let mut extra = serde_json::Map::new();
    for (k, v) in &preset.params {
        extra.insert(k.clone(), v.clone().into());
    }
    extra.insert("paintopid".to_string(), preset.paintopid.clone().into());

    IntermediateBrush {
        name: if preset.name.is_empty() { "Untitled Brush".to_string() } else { preset.name.clone() },
        diameter_px: tip.width as f32,
        tip,
        grain: None,
        angle_deg: angle_rad.to_degrees() as f32,
        roundness_pct: 100.0,
        spacing_pct: spacing_frac * 100.0,
        interpolate: true,
        flip_x: false,
        flip_y: false,
        pressure_sensitive_size: bool_param("PressureSize"),
        pressure_sensitive_opacity: bool_param("PressureOpacity"),
        extra,
    }
}

/// Generic `name="value"` attribute scan over a tag's inner text (the part
/// between `<Tag` and its closing `>`/`/>`), unescaping each value. Real
/// Krita XML doesn't keep attributes in a fixed order (unlike this module's
/// own writer), so this doesn't assume one.
fn parse_attrs(tag: &str) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'-' | b'/' | b':')) {
            i += 1;
        }
        if i == name_start {
            break;
        }
        let name = &tag[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'"' {
            continue;
        }
        i += 1;
        let val_start = i;
        while i < bytes.len() && bytes[i] != b'"' {
            i += 1;
        }
        let value = xml_unescape(&tag[val_start..i]);
        i += 1;
        attrs.insert(name.to_string(), value);
    }
    attrs
}

/// Reverse of [`xml_escape`]: named entities plus numeric `&#x..;`/`&#..;`
/// references. An unterminated `&` (no `;` within a sane lookahead) is left
/// as literal text rather than treated as an error — real text content
/// isn't guaranteed entity-clean.
fn xml_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let mut entity = String::new();
        let mut closed = false;
        for c2 in chars.by_ref() {
            if c2 == ';' {
                closed = true;
                break;
            }
            entity.push(c2);
            if entity.len() > 12 {
                break;
            }
        }
        if !closed {
            out.push('&');
            out.push_str(&entity);
            continue;
        }
        match entity.as_str() {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                if let Some(ch) = u32::from_str_radix(&entity[2..], 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            _ if entity.starts_with('#') => {
                if let Some(ch) = entity[1..].parse::<u32>().ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            _ => {
                out.push('&');
                out.push_str(&entity);
                out.push(';');
            }
        }
    }
    out
}

/// Writes `brush_set` as a `.bundle` archive: one `brushes/<name>.png` +
/// `paintoppresets/<name>.kpp` pair per brush, plus the manifest/meta/preview
/// files real Krita bundles carry — the same shape a real Krita install's
/// resource bundle importer expects (see module docs for what's verified).
pub fn export(brush_set: &BrushSet, path: &Path) -> Result<()> {
    std::fs::write(path, export_bytes(brush_set)?)?;
    Ok(())
}

pub fn export_bytes(brush_set: &BrushSet) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut zip = ZipWriter::new(Cursor::new(&mut buf));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default();

    zip.start_file("mimetype", stored)?;
    zip.write_all(MIMETYPE.as_bytes())?;

    let mut used_names = HashSet::new();
    let mut manifest_entries = Vec::new();

    for (i, brush) in brush_set.brushes.iter().enumerate() {
        let stem = unique_stem(&brush.name, i, &mut used_names);
        let tip_filename = format!("{stem}.png");

        let tip_bytes = encode_png(&brush.tip)?;
        let tip_md5 = md5_hex(&tip_bytes);
        let tip_path = format!("brushes/{tip_filename}");
        zip.start_file(&tip_path, deflated)?;
        zip.write_all(&tip_bytes)?;
        manifest_entries.push((tip_path, "brushes", tip_md5.clone()));

        let preset_bytes = encode_kpp(brush, &tip_filename, &tip_md5)?;
        let preset_path = format!("paintoppresets/{stem}.kpp");
        zip.start_file(&preset_path, deflated)?;
        zip.write_all(&preset_bytes)?;
        manifest_entries.push((preset_path, "paintoppresets", md5_hex(&preset_bytes)));
    }

    let preview = brush_set
        .brushes
        .first()
        .map(|b| b.tip.clone())
        .unwrap_or(RasterImage { width: 1, height: 1, rgba: vec![0; 4] });
    zip.start_file("preview.png", deflated)?;
    zip.write_all(&encode_png(&preview)?)?;

    zip.start_file("META-INF/manifest.xml", deflated)?;
    zip.write_all(manifest_xml(&manifest_entries).as_bytes())?;

    zip.start_file("meta.xml", deflated)?;
    zip.write_all(META_XML.as_bytes())?;

    zip.finish()?;
    Ok(buf)
}

const META_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<meta:meta>
 <meta:generator>brushes-converter</meta:generator>
 <meta:bundle-version>1</meta:bundle-version>
 <dc:description>Exported by brushes-converter</dc:description>
</meta:meta>
"#;

fn manifest_xml(entries: &[(String, &str, String)]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.2\">\n\
         \x20<manifest:file-entry manifest:media-type=\"application/x-krita-resourcebundle\" manifest:full-path=\"/\"/>\n",
    );
    for (path, media_type, md5) in entries {
        out.push_str(&format!(
            " <manifest:file-entry manifest:media-type=\"{media_type}\" manifest:full-path=\"{}\" manifest:md5sum=\"{md5}\"/>\n",
            xml_escape(path),
        ));
    }
    out.push_str("</manifest:manifest>\n");
    out
}

/// Builds the `.kpp` PNG: the tip raster as the preview thumbnail, plus the
/// `version`/`preset` text chunks Krita reads its settings from.
fn encode_kpp(brush: &IntermediateBrush, tip_filename: &str, tip_md5_hex: &str) -> Result<Vec<u8>> {
    let xml = preset_xml(brush, tip_filename, tip_md5_hex);

    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, brush.tip.width, brush.tip.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_text_chunk("version".to_string(), "2.2".to_string())
            .map_err(|e| ConverterError::Malformed(format!("failed writing .kpp version chunk: {e}")))?;
        encoder
            .add_ztxt_chunk("preset".to_string(), xml)
            .map_err(|e| ConverterError::Malformed(format!("failed writing .kpp preset chunk: {e}")))?;
        let mut writer = encoder
            .write_header()
            .map_err(|e| ConverterError::Malformed(format!("failed writing .kpp PNG header: {e}")))?;
        writer
            .write_image_data(&brush.tip.rgba)
            .map_err(|e| ConverterError::Malformed(format!("failed writing .kpp PNG data: {e}")))?;
    }
    Ok(buf)
}

fn preset_xml(brush: &IntermediateBrush, tip_filename: &str, tip_md5_hex: &str) -> String {
    let spacing_frac = (brush.spacing_pct / 100.0).max(0.01);
    let angle_rad = (brush.angle_deg as f64).to_radians();
    let bool_str = |b: bool| if b { "true" } else { "false" };

    let brush_definition = format!(
        "<Brush type=\"png_brush\" filename=\"{filename}\" md5sum=\"{md5}\" BrushVersion=\"2\" \
         spacing=\"{spacing}\" angle=\"{angle}\" scale=\"1\" useAutoSpacing=\"0\" autoSpacingCoeff=\"1\" \
         brushApplication=\"0\" ColorAsMask=\"1\" preserveLightness=\"0\" AdjustmentMidPoint=\"127\" \
         BrightnessAdjustment=\"0\" ContrastAdjustment=\"0\"/>",
        filename = tip_filename,
        md5 = tip_md5_hex,
        spacing = spacing_frac,
        angle = angle_rad,
    );

    let mut params = String::new();
    let mut param = |name: &str, value: &str| {
        params.push_str(&format!(
            "<param type=\"string\" name=\"{name}\"><![CDATA[{value}]]></param>"
        ));
    };
    param("brush_definition", &brush_definition);
    param("requiredBrushFile", tip_filename);
    param("paintop", "paintbrush");
    param("CompositeOp", "normal");
    param("PaintOpAction", "2");
    param("SizeValue", "1");
    param("SizeUseCurve", "false");
    param("PressureSize", bool_str(brush.pressure_sensitive_size));
    param("OpacityValue", "1");
    param("OpacityUseCurve", "false");
    param("PressureOpacity", bool_str(brush.pressure_sensitive_opacity));
    param("FlowValue", "1");
    param("SpacingValue", "1");
    param("SpacingUseCurve", "false");
    param("EraserMode", "false");
    param("HorizontalMirrorEnabled", "false");
    param("VerticalMirrorEnabled", "false");

    format!(
        "<Preset paintopid=\"paintbrush\" name=\"{name}\">{params}</Preset>",
        name = xml_escape(&brush.name),
    )
}

fn encode_png(image: &RasterImage) -> Result<Vec<u8>> {
    let buf = image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone())
        .ok_or_else(|| ConverterError::Malformed("raster buffer length doesn't match its width/height".into()))?;
    let mut out = Vec::new();
    buf.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

fn decode_png(bytes: &[u8]) -> Result<RasterImage> {
    let img = image::load_from_memory(bytes)?.into_rgba8();
    let (width, height) = img.dimensions();
    Ok(RasterImage { width, height, rgba: img.into_raw() })
}

fn md5_hex(bytes: &[u8]) -> String {
    format!("{:x}", md5::compute(bytes))
}

/// XML-escapes `text` for use both as element content and inside a
/// double-quoted attribute, and additionally numeric-escapes anything
/// outside ASCII so the result stays safe to Latin1-encode into a PNG
/// `zTXt` chunk (brush names are arbitrary Unicode; `zTXt`/`tEXt` chunks
/// are not) — Krita's own XML parser resolves `&#x...;` references back to
/// the original character on load.
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_ascii() => out.push(c),
            c => out.push_str(&format!("&#x{:X};", c as u32)),
        }
    }
    out
}

/// Filesystem/zip-safe stem for a brush's `brushes/<stem>.png` and
/// `paintoppresets/<stem>.kpp` entries: its name with anything but
/// alphanumerics/spaces/-/_ replaced, falling back to `brush` if that
/// leaves nothing, disambiguated with `_<index>` on collision (brush names
/// aren't guaranteed unique within a set).
fn unique_stem(name: &str, index: usize, used: &mut HashSet<String>) -> String {
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
    use std::io::Read;
    use zip::ZipArchive;

    fn brush(name: &str) -> IntermediateBrush {
        IntermediateBrush {
            name: name.to_string(),
            tip: RasterImage { width: 4, height: 4, rgba: vec![255; 4 * 4 * 4] },
            grain: None,
            diameter_px: 4.0,
            angle_deg: 45.0,
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

    fn preset_xml_of(zip: &mut ZipArchive<Cursor<&[u8]>>, path: &str) -> String {
        let mut kpp_bytes = Vec::new();
        zip.by_name(path).unwrap().read_to_end(&mut kpp_bytes).unwrap();

        let decoder = png::Decoder::new(Cursor::new(kpp_bytes));
        let reader = decoder.read_info().unwrap();
        let info = reader.info();
        assert!(info.uncompressed_latin1_text.iter().any(|t| t.keyword == "version"));
        let chunk = info
            .compressed_latin1_text
            .iter()
            .find(|t| t.keyword == "preset")
            .expect("preset zTXt chunk");
        chunk.get_text().unwrap()
    }

    #[test]
    fn writes_bundle_scaffolding_and_a_preset_per_brush() {
        let brush_set = BrushSet { brushes: vec![brush("Round Marker"), brush("Round Marker")] };
        let bytes = export_bytes(&brush_set).unwrap();

        let mut zip = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut mimetype = String::new();
        zip.by_name("mimetype").unwrap().read_to_string(&mut mimetype).unwrap();
        assert_eq!(mimetype, MIMETYPE);

        assert!(zip.by_name("preview.png").is_ok());
        assert!(zip.by_name("meta.xml").is_ok());

        let mut manifest = String::new();
        zip.by_name("META-INF/manifest.xml").unwrap().read_to_string(&mut manifest).unwrap();
        assert!(manifest.contains("brushes/Round Marker.png"));
        assert!(manifest.contains("brushes/Round Marker_1.png"));
        assert!(manifest.contains("paintoppresets/Round Marker.kpp"));
        assert!(manifest.contains("paintoppresets/Round Marker_1.kpp"));

        let xml = preset_xml_of(&mut zip, "paintoppresets/Round Marker.kpp");
        assert!(xml.starts_with("<Preset paintopid=\"paintbrush\" name=\"Round Marker\">"));
        assert!(xml.contains("type=\"png_brush\" filename=\"Round Marker.png\""));
        assert!(xml.contains("<param type=\"string\" name=\"requiredBrushFile\"><![CDATA[Round Marker.png]]></param>"));
        assert!(xml.contains("<param type=\"string\" name=\"PressureSize\"><![CDATA[true]]></param>"));
        assert!(xml.contains("<param type=\"string\" name=\"PressureOpacity\"><![CDATA[false]]></param>"));
    }

    #[test]
    fn xml_escape_handles_metacharacters_and_non_ascii() {
        assert_eq!(xml_escape("caf\u{00e9} \"quoted\" & <tag>"), "caf&#xE9; &quot;quoted&quot; &amp; &lt;tag&gt;");
    }

    #[test]
    fn preset_xml_stays_ascii_for_a_non_ascii_brush_name() {
        let mut b = brush("stem");
        b.name = "caf\u{00e9}".to_string();
        let xml = preset_xml(&b, "stem.png", "deadbeef");
        assert!(xml.is_ascii());
        assert!(xml.contains("name=\"caf&#xE9;\""));
    }

    #[test]
    fn round_trips_through_our_own_writer_and_reader() {
        let mut b = brush("caf\u{00e9} Marker");
        b.angle_deg = 30.0;
        let brush_set = BrushSet { brushes: vec![b, brush("Round Marker")] };

        let bytes = export_bytes(&brush_set).unwrap();
        let reimported = import(&bytes).unwrap();

        assert_eq!(reimported.brushes.len(), 2);
        for (original, round_tripped) in brush_set.brushes.iter().zip(&reimported.brushes) {
            assert_eq!(round_tripped.name, original.name);
            assert_eq!(round_tripped.tip.width, original.tip.width);
            assert_eq!(round_tripped.tip.height, original.tip.height);
            assert_eq!(round_tripped.tip.rgba, original.tip.rgba);
            assert!((round_tripped.angle_deg - original.angle_deg).abs() < 0.01);
            assert!((round_tripped.spacing_pct - original.spacing_pct).abs() < 0.1);
            assert_eq!(round_tripped.pressure_sensitive_size, original.pressure_sensitive_size);
            assert_eq!(round_tripped.pressure_sensitive_opacity, original.pressure_sensitive_opacity);
        }
    }

    #[test]
    fn import_falls_back_to_the_kpp_thumbnail_when_standalone() {
        let brush_set = BrushSet { brushes: vec![brush("Solo")] };
        let bytes = export_bytes(&brush_set).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut kpp_bytes = Vec::new();
        zip.by_name("paintoppresets/Solo.kpp").unwrap().read_to_end(&mut kpp_bytes).unwrap();

        let reimported = import(&kpp_bytes).unwrap();
        assert_eq!(reimported.brushes.len(), 1);
        assert_eq!(reimported.brushes[0].name, "Solo");
        assert_eq!(reimported.brushes[0].tip.width, 4);
    }

    #[test]
    fn skips_non_raster_presets_but_keeps_raster_ones() {
        let brush_set = BrushSet { brushes: vec![brush("Raster One")] };
        let bytes = export_bytes(&brush_set).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();

        // Splice in a second, parametric-brush preset entry with no matching
        // brushes/ raster, alongside the real raster one.
        let mut buf = Vec::new();
        {
            let mut new_zip = ZipWriter::new(Cursor::new(&mut buf));
            let options = SimpleFileOptions::default();
            for i in 0..zip.len() {
                let mut file = zip.by_index(i).unwrap();
                let name = file.name().to_string();
                let mut contents = Vec::new();
                file.read_to_end(&mut contents).unwrap();
                new_zip.start_file(&name, options).unwrap();
                new_zip.write_all(&contents).unwrap();
            }
            let parametric_xml = "<Preset paintopid=\"paintbrush\" name=\"Parametric\">\
                <param type=\"string\" name=\"brush_definition\">\
                <![CDATA[<Brush type=\"auto_brush\" diameter=\"20\"/>]]></param></Preset>";
            let parametric_kpp = encode_kpp_raw(parametric_xml);
            new_zip.start_file("paintoppresets/Parametric.kpp", options).unwrap();
            new_zip.write_all(&parametric_kpp).unwrap();
            new_zip.finish().unwrap();
        }

        let reimported = import(&buf).unwrap();
        assert_eq!(reimported.brushes.len(), 1);
        assert_eq!(reimported.brushes[0].name, "Raster One");
    }

    fn encode_kpp_raw(preset_xml: &str) -> Vec<u8> {
        let tip = RasterImage { width: 2, height: 2, rgba: vec![255; 2 * 2 * 4] };
        let mut buf = Vec::new();
        let mut encoder = png::Encoder::new(&mut buf, tip.width, tip.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.add_ztxt_chunk("preset".to_string(), preset_xml.to_string()).unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&tip.rgba).unwrap();
        drop(writer);
        buf
    }

    #[test]
    fn xml_unescape_reverses_xml_escape() {
        let escaped = xml_escape("caf\u{00e9} \"quoted\" & <tag>");
        assert_eq!(xml_unescape(&escaped), "caf\u{00e9} \"quoted\" & <tag>");
    }

    #[test]
    fn parse_attrs_reads_out_of_order_attributes() {
        let attrs = parse_attrs(
            "<Brush scale=\"1.03\" filename=\"DA_RGBA bluegreen_small1.png\" type=\"png_brush\" angle=\"0\"",
        );
        assert_eq!(attrs.get("filename").unwrap(), "DA_RGBA bluegreen_small1.png");
        assert_eq!(attrs.get("type").unwrap(), "png_brush");
        assert_eq!(attrs.get("scale").unwrap(), "1.03");
    }
}
