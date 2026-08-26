//! Clip Studio Paint `.sut` support is not implemented yet — `.sut` is a
//! SQLite3 database with brush tip BLOBs, scaffolded here so the
//! format-dispatch table in `formats::mod` is complete.

use crate::error::{ConverterError, Result};
use crate::schema::BrushSet;
use std::path::Path;

pub fn import(_path: &Path) -> Result<BrushSet> {
    Err(ConverterError::NotImplemented("Clip Studio Paint"))
}
