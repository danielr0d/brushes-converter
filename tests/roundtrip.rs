//! Import the real Procreate brushset, export it to `.abr` with our writer,
//! re-import that `.abr` with our own reader, and check the two agree.
//! Also spot-checks a couple of values against
//! `RakeBrushpackPhotoshopFinal.abr` itself, since that file is this
//! brushset's known origin (every brush in it has `importedFromABR: true`).

use brushes_converter::formats::{abr, procreate};

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
