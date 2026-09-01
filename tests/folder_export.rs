//! Import the real Procreate brushset and export it as a folder tree,
//! checking every brush gets a `tip.png` that decodes back to the same
//! dimensions as the source raster, plus a readable `settings.json`.

use brushes_converter::formats::{folder, procreate};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn procreate_brushset_exports_to_one_folder_per_brush() {
    let brushset_bytes = std::fs::read(fixture("Rake_Brushpack.brushset")).unwrap();
    let imported = procreate::import(&brushset_bytes).unwrap();
    assert!(!imported.brushes.is_empty());

    let out_dir = tempfile::tempdir().unwrap();
    folder::export(&imported, out_dir.path()).unwrap();

    let mut candidate_dirs: Vec<_> = std::fs::read_dir(out_dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(candidate_dirs.len(), imported.brushes.len());

    for brush in &imported.brushes {
        let idx = candidate_dirs
            .iter()
            .position(|dir| {
                serde_json::from_str::<serde_json::Value>(
                    &std::fs::read_to_string(dir.join("settings.json")).unwrap(),
                )
                .unwrap()["name"]
                    == serde_json::Value::String(brush.name.clone())
            })
            .expect("exported folder for brush not found");
        let brush_dir = candidate_dirs.remove(idx);

        let tip_png = image::open(brush_dir.join("tip.png")).unwrap();
        assert_eq!(tip_png.width(), brush.tip.width);
        assert_eq!(tip_png.height(), brush.tip.height);

        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(brush_dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["name"], brush.name);
        assert!((settings["diameter_px"].as_f64().unwrap() - brush.diameter_px as f64).abs() < 0.01);
    }
}
