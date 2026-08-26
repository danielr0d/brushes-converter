pub mod abr;
pub mod csp;
pub mod krita;
pub mod procreate;

use crate::error::{ConverterError, Result};
use crate::schema::BrushSet;
use std::path::Path;

/// Recognized source formats, detected by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Procreate,
    Abr,
    Krita,
    Csp,
}

pub fn detect(path: &Path) -> Option<Format> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() {
        "brush" | "brushset" => Some(Format::Procreate),
        "abr" => Some(Format::Abr),
        "kbr" | "bundle" => Some(Format::Krita),
        "sut" => Some(Format::Csp),
        _ => None,
    }
}

/// Imports whatever brush file lives at `path`, dispatching by extension.
/// `bytes` must be the full file contents already read from `path` (the SQLite-based
/// CSP importer needs a real file path rather than an in-memory buffer, so both
/// are threaded through).
pub fn import_auto(path: &Path, bytes: &[u8]) -> Result<BrushSet> {
    match detect(path) {
        Some(Format::Procreate) => procreate::import(bytes),
        Some(Format::Abr) => abr::import(bytes),
        Some(Format::Krita) => krita::import(bytes),
        Some(Format::Csp) => csp::import(path),
        None => Err(ConverterError::UnknownFormat(
            path.display().to_string(),
        )),
    }
}
