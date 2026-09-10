//! Import the real Procreate brushset, export it to `.abr` with our writer,
//! re-import that `.abr` with our own reader, and check the two agree.
//! Also spot-checks a couple of values against
//! `RakeBrushpackPhotoshopFinal.abr` itself, since that file is this
//! brushset's known origin (every brush in it has `importedFromABR: true`).

use brushes_converter::formats::{abr, krita, procreate};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn procreate_to_abr_round_trips_through_our_own_reader() {
    let brushset_bytes = std::fs::read(fixture("Rake_Brushpack.brushset")).unwrap();
    let imported = procreate::import(&brushset_bytes).unwrap();
    assert!(!imported.brushes.is_empty());

    let tmp = tempfile::NamedTempFile::new().unwrap();
    abr::export(&imported, tmp.path()).unwrap();

    let abr_bytes = std::fs::read(tmp.path()).unwrap();
    let reimported = abr::import(&abr_bytes).unwrap();

    assert_eq!(reimported.brushes.len(), imported.brushes.len());
    for (original, round_tripped) in imported.brushes.iter().zip(&reimported.brushes) {
        assert_eq!(round_tripped.name, original.name);
        assert_eq!(round_tripped.tip.width, original.tip.width);
        assert_eq!(round_tripped.tip.height, original.tip.height);
        assert!((round_tripped.diameter_px - original.diameter_px).abs() < 0.01);
        assert!((round_tripped.angle_deg - original.angle_deg).abs() < 0.01);
        assert!((round_tripped.roundness_pct - original.roundness_pct).abs() < 0.01);
        assert!((round_tripped.spacing_pct - original.spacing_pct).abs() < 0.01);
        // ABR tips are single-channel: only the alpha/luminance mask survives
        // the round trip, not the original RGBA (reconstructed as white+alpha).
        assert_eq!(round_tripped.tip.to_alpha_mask(), original.tip.to_alpha_mask());
    }
}

#[test]
fn procreate_round_trips_through_our_own_writer_and_reader() {
    let brushset_bytes = std::fs::read(fixture("Rake_Brushpack.brushset")).unwrap();
    let imported = procreate::import(&brushset_bytes).unwrap();
    assert!(!imported.brushes.is_empty());

    let exported_bytes = procreate::export_bytes(&imported).unwrap();
    let reimported = procreate::import(&exported_bytes).unwrap();

    assert_eq!(reimported.brushes.len(), imported.brushes.len());
    for (original, round_tripped) in imported.brushes.iter().zip(&reimported.brushes) {
        assert_eq!(round_tripped.name, original.name);
        assert_eq!(round_tripped.tip.width, original.tip.width);
        assert_eq!(round_tripped.tip.height, original.tip.height);
        assert_eq!(round_tripped.tip.rgba, original.tip.rgba);
        assert!((round_tripped.angle_deg - original.angle_deg).abs() < 0.01);
        assert!((round_tripped.roundness_pct - original.roundness_pct).abs() < 0.01);
        assert!((round_tripped.spacing_pct - original.spacing_pct).abs() < 0.01);
        assert_eq!(round_tripped.pressure_sensitive_size, original.pressure_sensitive_size);
        assert_eq!(round_tripped.pressure_sensitive_opacity, original.pressure_sensitive_opacity);
    }
}

#[test]
fn brushset_names_appear_in_the_original_photoshop_file() {
    // Not a byte-for-byte comparison (the ABR's own samp compression isn't
    // decoded by our reader), just a sanity check that the brush names in
    // the Procreate set line up with names present in the ABR's `desc`
    // section, confirming these two fixtures are indeed the same brush pack.
    let brushset_bytes = std::fs::read(fixture("Rake_Brushpack.brushset")).unwrap();
    let imported = procreate::import(&brushset_bytes).unwrap();

    let abr_bytes = std::fs::read(fixture("RakeBrushpackPhotoshopFinal.abr")).unwrap();

    // Names inside the .abr's `desc` section are UTF-16BE, so search for the
    // UTF-16BE-encoded bytes rather than treating the file as UTF-8 text.
    let contains_utf16be = |haystack: &[u8], needle: &str| {
        let needle_bytes: Vec<u8> = needle.encode_utf16().flat_map(u16::to_be_bytes).collect();
        haystack.windows(needle_bytes.len()).any(|w| w == needle_bytes.as_slice())
    };

    let mut matched = 0;
    for brush in &imported.brushes {
        let base_name = brush.name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ');
        if contains_utf16be(&abr_bytes, base_name.trim()) {
            matched += 1;
        }
    }
    assert!(
        matched > 0,
        "expected at least one Procreate brush name to appear in the source .abr"
    );
}

#[test]
fn imports_the_real_krita_bundle_end_to_end() {
    // `RGBA_brushes.bundle` is a real Krita-authored bundle fetched from
    // krita/data/bundles/RGBA_brushes.bundle in the KDE/krita repo (see
    // src/formats/krita.rs's module docs and the krita-bundle-schema-reference
    // memory for provenance). It ships 6 presets: 4 are `png_brush` (a plain
    // PNG pixel tip, referencing brushes/*.png) and import cleanly; the other
    // 2 (Impasto/Impasto-details) are `gbr_brush` referencing a `.gih` file
    // (GIMP's animated multi-frame "image pipe" format, not a plain PNG) and
    // are correctly skipped — this schema has no way to represent an
    // animated multi-frame tip, and `.gih` isn't PNG-decodable regardless.
    let bundle_bytes = std::fs::read(fixture("RGBA_brushes.bundle")).unwrap();
    let imported = krita::import(&bundle_bytes).unwrap();

    assert_eq!(imported.brushes.len(), 4);

    let names: Vec<&str> = imported.brushes.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"m)_RGBA_01_Thick-dry"));
    assert!(names.contains(&"m)_RGBA_03_Rake"));

    for brush in &imported.brushes {
        assert!(brush.tip.width > 0 && brush.tip.height > 0);
        // The tip must be the actual referenced brush raster, not the .kpp's
        // own UI-thumbnail raster (a different, much larger preview image).
        assert!(brush.tip.rgba.iter().any(|&b| b != 0), "tip for '{}' is blank", brush.name);
        assert!(brush.spacing_pct > 0.0);
    }

    let rake = imported.brushes.iter().find(|b| b.name == "m)_RGBA_03_Rake").unwrap();
    assert!((rake.spacing_pct - 3.0).abs() < 0.5, "spacing_pct was {}", rake.spacing_pct);
}

#[test]
fn imports_the_real_photoshop_abr_end_to_end() {
    // Unlike the round-trip tests above (which only validate our writer
    // against our own reader), this decodes RakeBrushpackPhotoshopFinal.abr
    // itself: real Adobe-authored `desc` descriptors cross-referencing real
    // Adobe-authored `samp` rasters, including live PackBits decompression.
    let abr_bytes = std::fs::read(fixture("RakeBrushpackPhotoshopFinal.abr")).unwrap();
    let imported = abr::import(&abr_bytes).unwrap();

    // The desc section lists 28 brush presets, several of which reuse the
    // same underlying sampled tip (fewer than 28 samp entries exist).
    assert_eq!(imported.brushes.len(), 28);

    let first = &imported.brushes[0];
    assert_eq!(first.name, "Very Basic Rake");
    assert!((first.diameter_px - 700.0).abs() < 0.01);
    assert!(first.tip.width > 0 && first.tip.height > 0);

    // Every decoded tip should be non-degenerate: real pixel data, not an
    // empty or uniform mask.
    for brush in &imported.brushes {
        assert!(brush.tip.width > 0 && brush.tip.height > 0);
        let mask = brush.tip.to_alpha_mask();
        assert_eq!(mask.len(), (brush.tip.width * brush.tip.height) as usize);
        assert!(mask.iter().any(|&b| b != 0), "brush {} decoded to an all-zero tip", brush.name);
    }
}
