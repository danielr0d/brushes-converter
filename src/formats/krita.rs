//! Krita `.kbr`/`.bundle` support is not implemented yet — both formats are
//! ZIP archives (PNG tip + XML settings for `.kbr`; a bundle manifest around
//! several resources for `.bundle`), scaffolded here so the format-dispatch
//! table in `formats::mod` is complete.

use crate::error::{ConverterError, Result};
use crate::schema::BrushSet;

pub fn import(_bytes: &[u8]) -> Result<BrushSet> {
    Err(ConverterError::NotImplemented("Krita"))
}
