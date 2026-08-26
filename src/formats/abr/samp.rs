//! The `8BIM samp` section: raw brush-tip raster storage, one entry per
//! brush tip, cross-referenced by GUID from the `desc` section's
//! `sampledData` field.
//!
//! Forensic inspection of a real Photoshop-written file confirmed the outer
//! per-entry framing (`u32 total_len` + Pascal-style GUID string + a small
//! fixed header + pixel bytes) and that some header fields land consistently
//! at fixed offsets (a `data_len` field, and four `u32`s that read as a
//! plausible pixel bounding box) — see the forensics note in the module docs
//! for `abr::mod`. What remains genuinely unclear from static analysis alone
//! is a couple of constant-looking filler fields and whether real Photoshop
//! ever stores more than one resolution per GUID, so rather than guess at
//! full Adobe byte-fidelity, this module defines an **internally consistent
//! layout of our own** (documented below) for entries this crate writes, and
//! reads that same layout back. It also reads Adobe-written entries insofar
//! as their outer framing matches (GUID + bounding box + raw byte count),
//! which is enough to validate round-tripping through our own writer.
//!
//! Entry layout (after the section's `8BIM samp <len>` wrapper), repeated:
//! ```text
//! u32   total_len          // bytes remaining below, excluding this field
//! u8    guid_len
//! [guid_len bytes]  guid   // ASCII, no braces
//! u16   unknown_a = 1
//! u32   unknown_b = 3
//! u32   data_len
//! u32   top
//! u32   left
//! u32   bottom
//! u32   right
//! u16   depth = 8
//! u8    compression = 0    // always raw/uncompressed for data we write
//! [data_len bytes]  pixel data, row-major, 8-bit alpha, (bottom-top)*(right-left) bytes
//! ```

use crate::error::{ConverterError, Result};
use binrw::{BinRead, BinWrite};
use std::io::Cursor;

#[derive(BinRead, BinWrite)]
#[br(big)]
#[bw(big)]
struct EntryHeader {
    unknown_a: u16,
    unknown_b: u32,
    data_len: u32,
    top: u32,
    left: u32,
    bottom: u32,
    right: u32,
    depth: u16,
    compression: u8,
}

pub struct SampEntry {
    pub guid: String,
    pub width: u32,
    pub height: u32,
    /// Row-major 8-bit alpha/grayscale mask, `width * height` bytes.
    pub mask: Vec<u8>,
}

pub fn write_samp_section(entries: &[SampEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in entries {
        let mut body = Vec::new();
        body.push(entry.guid.len() as u8);
        body.extend_from_slice(entry.guid.as_bytes());

        let header = EntryHeader {
            unknown_a: 1,
            unknown_b: 3,
            data_len: entry.mask.len() as u32,
            top: 0,
            left: 0,
            bottom: entry.height,
            right: entry.width,
            depth: 8,
            compression: 0,
        };
        let mut header_bytes = Cursor::new(Vec::new());
        header.write(&mut header_bytes).expect("in-memory write cannot fail");
        body.extend_from_slice(&header_bytes.into_inner());
        body.extend_from_slice(&entry.mask);

        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
    }
    out
}

pub fn read_samp_section(data: &[u8]) -> Result<Vec<SampEntry>> {
    let malformed = || ConverterError::Malformed("truncated samp entry".into());

    let mut entries = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let total_len = u32::from_be_bytes(
            data.get(pos..pos + 4).ok_or_else(malformed)?.try_into().unwrap(),
        ) as usize;
        pos += 4;
        let body = data.get(pos..pos + total_len).ok_or_else(malformed)?;
        pos += total_len;

        let guid_len = *body.first().ok_or_else(malformed)? as usize;
        let guid = String::from_utf8_lossy(
            body.get(1..1 + guid_len).ok_or_else(malformed)?,
        )
        .into_owned();

        let mut header_reader = Cursor::new(&body[1 + guid_len..]);
        let header = EntryHeader::read(&mut header_reader)
            .map_err(|e| ConverterError::Malformed(format!("samp entry header: {e}")))?;
        let header_size = header_reader.position() as usize;
        let data_start = 1 + guid_len + header_size;
        let mask = body
            .get(data_start..data_start + header.data_len as usize)
            .ok_or_else(malformed)?
            .to_vec();

        entries.push(SampEntry {
            guid,
            width: header.right.saturating_sub(header.left),
            height: header.bottom.saturating_sub(header.top),
            mask,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_entries() {
        let entries = vec![
            SampEntry { guid: "aaaa-bbbb".into(), width: 4, height: 2, mask: vec![10, 20, 30, 40, 50, 60, 70, 80] },
            SampEntry { guid: "cccc-dddd".into(), width: 1, height: 1, mask: vec![255] },
        ];
        let bytes = write_samp_section(&entries);
        let parsed = read_samp_section(&bytes).unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].guid, "aaaa-bbbb");
        assert_eq!(parsed[0].width, 4);
        assert_eq!(parsed[0].height, 2);
        assert_eq!(parsed[0].mask, vec![10, 20, 30, 40, 50, 60, 70, 80]);
        assert_eq!(parsed[1].mask, vec![255]);
    }
}
