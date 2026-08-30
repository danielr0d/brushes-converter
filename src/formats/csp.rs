//! Clip Studio Paint `.sut` ("sub tool", i.e. brush) importer.
//!
//! A `.sut` file is a plain SQLite3 database; Celsys has never published its
//! schema, so this only implements the pieces independently verified by
//! reverse-engineering:
//!
//! - Brush tip (and, if present, a second grain/texture) rasters live in the
//!   `MaterialFile` table's `FileData` BLOB column, one row per resource.
//!   Each `FileData` blob is a CSP-specific container with a PNG embedded
//!   somewhere inside it — confirmed against
//!   github.com/MorrowShore/CSPBrushExtract, a working community brush-tip
//!   extractor, which locates it by scanning for the *last* PNG signature in
//!   the blob (CSP prepends its own metadata first, which can itself contain
//!   the bytes "PNG"). We do the same, then hand everything from that offset
//!   onward to the `image` crate, which stops at the PNG's `IEND` chunk on
//!   its own — no need to hunt down the chunk's end ourselves.
//! - The subtool/brush name lives in the `Node` table's `NodeName` column
//!   (per the CSPBrushInfo project's documentation of CSP's shared
//!   tool-preset format). We take the first non-empty one.
//!
//! Everything else a brush can carry — size, spacing, angle, pressure
//! curves — lives in the `Variant` table's parameter blob, whose binary
//! layout is undocumented and reported to change across CSP versions. We
//! don't guess at it: those fields are left at schema defaults rather than
//! risk silently-wrong values, same spirit as the `.abr` reader's own
//! "best-effort, not verified" scope note.

use crate::error::{ConverterError, Result};
use crate::schema::{BrushSet, IntermediateBrush, RasterImage};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

pub fn import(path: &Path) -> Result<BrushSet> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| ConverterError::Malformed(format!("failed to open .sut as SQLite: {e}")))?;

    let name = read_node_name(&conn);
    let mut rasters = read_material_rasters(&conn)?.into_iter();

    let tip = rasters.next().ok_or_else(|| {
        ConverterError::Malformed("no brush tip image found in MaterialFile".into())
    })?;
    let grain = rasters.next();

    let brush = IntermediateBrush {
        name: name.unwrap_or_else(|| "Untitled Brush".to_string()),
        diameter_px: tip.width as f32,
        tip,
        grain,
        angle_deg: 0.0,
        roundness_pct: 100.0,
        spacing_pct: 25.0,
        interpolate: true,
        flip_x: false,
        flip_y: false,
        pressure_sensitive_size: false,
        pressure_sensitive_opacity: false,
        extra: serde_json::Map::new(),
    };

    Ok(BrushSet { brushes: vec![brush] })
}

/// Best-effort: absent on any error (missing `Node` table, no non-empty
/// name, etc.) rather than failing the whole import over a display label.
fn read_node_name(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT NodeName FROM Node WHERE NodeName IS NOT NULL AND NodeName <> '' LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

fn read_material_rasters(conn: &Connection) -> Result<Vec<RasterImage>> {
    let mut stmt = match conn.prepare("SELECT FileData FROM MaterialFile") {
        Ok(stmt) => stmt,
        Err(_) => return Ok(Vec::new()), // no MaterialFile table in this .sut
    };

    let blobs = stmt
        .query_map([], |row| row.get::<_, Option<Vec<u8>>>(0))
        .map_err(|e| ConverterError::Malformed(format!("failed to read MaterialFile: {e}")))?;

    let mut rasters = Vec::new();
    for blob in blobs {
        let blob = blob
            .map_err(|e| ConverterError::Malformed(format!("failed to read MaterialFile row: {e}")))?;
        let Some(blob) = blob else { continue };
        if let Some(raster) = extract_png(&blob)? {
            rasters.push(raster);
        }
    }
    Ok(rasters)
}

fn extract_png(blob: &[u8]) -> Result<Option<RasterImage>> {
    let Some(start) = last_subslice(blob, &PNG_SIGNATURE) else {
        return Ok(None);
    };
    let img = image::load_from_memory(&blob[start..])?.into_rgba8();
    let (width, height) = img.dimensions();
    Ok(Some(RasterImage {
        width,
        height,
        rgba: img.into_raw(),
    }))
}

fn last_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Builds a minimal synthetic `.sut`-shaped SQLite database (a `Node`
    /// table with a name, a `MaterialFile` table with one PNG-in-a-blob row
    /// prefixed by junk bytes that also contain the ASCII text "PNG", to
    /// exercise the last-signature search) and checks it round-trips
    /// through `import`. This validates our own wiring; it says nothing
    /// about compatibility with real CSP-authored `.sut` files, which
    /// requires a real fixture we don't have.
    #[test]
    fn imports_name_and_tip_from_synthetic_database() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sut");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE Node (NodeName TEXT);
             CREATE TABLE MaterialFile (_PW_ID INTEGER, FileData BLOB);",
        )
        .unwrap();
        conn.execute("INSERT INTO Node (NodeName) VALUES ('Test Brush')", [])
            .unwrap();

        let mut png_bytes = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .unwrap();

        let mut blob = b"junk metadata mentioning PNG before the real image".to_vec();
        blob.extend_from_slice(&png_bytes);

        conn.execute(
            "INSERT INTO MaterialFile (_PW_ID, FileData) VALUES (1, ?1)",
            [blob],
        )
        .unwrap();
        drop(conn);

        let brush_set = import(&db_path).unwrap();
        assert_eq!(brush_set.brushes.len(), 1);
        let brush = &brush_set.brushes[0];
        assert_eq!(brush.name, "Test Brush");
        assert_eq!(brush.tip.width, 4);
        assert_eq!(brush.tip.height, 4);
        assert!(brush.grain.is_none());
    }
}
