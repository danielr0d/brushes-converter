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
//! Import is not implemented — this module is export-only.

use crate::error::{ConverterError, Result};
use crate::schema::{BrushSet, IntermediateBrush, RasterImage};
use std::collections::HashSet;
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const MIMETYPE: &str = "application/x-krita-resourcebundle";

pub fn import(_bytes: &[u8]) -> Result<BrushSet> {
    Err(ConverterError::NotImplemented("Krita"))
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
}
