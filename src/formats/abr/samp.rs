//! The `8BIM samp` section: raw brush-tip raster storage, one entry per
//! brush tip, cross-referenced by GUID from the `desc` section's
//! `sampledData` field.
//!
//! This layout was reverse-engineered against a real CC-era
//! (`version=10, subversion=2`) Photoshop-authored file
//! (`tests/fixtures/RakeBrushpackPhotoshopFinal.abr`) — every one of its 22
//! `samp` entries parses exactly to its declared length with this grammar,
//! confirmed against the [ABR.ksy Kaitai spec from jlai/brush-viewer]
//! (https://github.com/jlai/brush-viewer/blob/main/shared/abr/ABR.ksy),
//! itself derived from reading ABRViewer's C++ source. Real Photoshop
//! stores a fixed table of 56 "channel" slots per brush (evidently fixed
//! resolution/LOD buckets), of which only one was ever populated in the
//! fixture; this reader tolerates any `num_channels` and any subset being
//! written, picking the highest-resolution (largest-area) populated one as
//! the brush's tip raster. The `bounds` field below and the 8-byte trailer
//! after the channel table are unused framing observed to always be zero;
//! their real meaning (if any) is undocumented and not needed to read the
//! pixel data.
//!
//! Entry layout (after the section's `8BIM samp <len>` wrapper), repeated:
//! ```text
//! u32   sample_len        // bytes remaining below, excluding this field
//! u8    guid_len
//! [guid_len bytes]  guid  // ASCII, no braces
//! u16   meta_len          // observed constant 1, meaning unknown
//! u16   meta_a            // observed constant 0, meaning unknown
//! u32   version           // observed constant 3
//! u32   length            // bytes remaining below, excluding this field
//! [16 bytes]  bounds      // unused/unparsed in every observed file
//! u32   num_channels
//! repeat num_channels:
//!   u32   is_written
//!   if is_written != 0:
//!     u32   chan_len        // bytes remaining below, excluding this field
//!     u32   unused_depth    // observed to duplicate `depth` below
//!     u32   top
//!     u32   left
//!     u32   bottom
//!     u32   right
//!     u16   depth           // only 8 (bits/pixel) is supported
//!     u8    compression     // 0 = raw, 1 = PackBits RLE (see below)
//!     [chan_len - 23 bytes] pixel data
//! [8 bytes]  trailer       // unused, observed to always be zero
//! [padding to a multiple of 4, per sample_len]
//! ```
//!
//! PackBits (`compression == 1`) pixel data is, per scanline (`height` of
//! them): a table of `u16` compressed byte counts for every row, then each
//! row's compressed bytes back to back, standard PackBits-encoded (matches
//! Photoshop's RLE image-data convention elsewhere, e.g. in `.psd` files).
//!
//! This module writes a single populated channel (`num_channels = 1`,
//! uncompressed) rather than replicating the 56-slot table — nothing in the
//! grammar requires the full table, and the reader above accepts any count.

use crate::error::{ConverterError, Result};

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
        write_entry(&mut out, entry);
    }
    out
}

fn write_entry(out: &mut Vec<u8>, entry: &SampEntry) {
    let mut channel_tail = Vec::new();
    channel_tail.extend_from_slice(&8u32.to_be_bytes()); // unused_depth
    channel_tail.extend_from_slice(&0u32.to_be_bytes()); // top
    channel_tail.extend_from_slice(&0u32.to_be_bytes()); // left
    channel_tail.extend_from_slice(&entry.height.to_be_bytes()); // bottom
    channel_tail.extend_from_slice(&entry.width.to_be_bytes()); // right
    channel_tail.extend_from_slice(&8u16.to_be_bytes()); // depth
    channel_tail.push(0); // compression = raw
    channel_tail.extend_from_slice(&entry.mask);

    let mut channel_entry = Vec::new();
    channel_entry.extend_from_slice(&1u32.to_be_bytes()); // is_written
    channel_entry.extend_from_slice(&(channel_tail.len() as u32).to_be_bytes());
    channel_entry.extend_from_slice(&channel_tail);

    let mut after_length = Vec::new();
    after_length.extend_from_slice(&[0u8; 16]); // bounds (reserved/unused)
    after_length.extend_from_slice(&1u32.to_be_bytes()); // num_channels
    after_length.extend_from_slice(&channel_entry);
    after_length.extend_from_slice(&[0u8; 8]); // trailer (reserved/unused)

    let mut body = Vec::new();
    body.push(entry.guid.len() as u8);
    body.extend_from_slice(entry.guid.as_bytes());
    body.extend_from_slice(&1u16.to_be_bytes()); // meta_len
    body.extend_from_slice(&0u16.to_be_bytes()); // meta_a
    body.extend_from_slice(&3u32.to_be_bytes()); // version
    body.extend_from_slice(&(after_length.len() as u32).to_be_bytes()); // length
    body.extend_from_slice(&after_length);

    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    let pad = (4 - body.len() % 4) % 4;
    out.extend(std::iter::repeat_n(0u8, pad));
}

pub fn read_samp_section(data: &[u8]) -> Result<Vec<SampEntry>> {
    let mut entries = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let (entry, consumed) = read_entry(&data[pos..])?;
        entries.push(entry);
        pos += consumed;
    }
    Ok(entries)
}

fn read_entry(data: &[u8]) -> Result<(SampEntry, usize)> {
    let malformed = |msg: &str| ConverterError::Malformed(format!("samp entry: {msg}"));

    let mut pos = 0usize;
    let sample_len = read_u32(data, &mut pos, &malformed)? as usize;

    let body_start = pos;
    let body_end = body_start
        .checked_add(sample_len)
        .filter(|&e| e <= data.len())
        .ok_or_else(|| malformed("truncated body"))?;
    let body = &data[body_start..body_end];

    let mut bpos = 0usize;
    let guid_len = read_u8(body, &mut bpos, &malformed)? as usize;
    let guid = String::from_utf8_lossy(read_slice(body, &mut bpos, guid_len, &malformed)?).into_owned();

    let _meta_len = read_u16(body, &mut bpos, &malformed)?;
    let _meta_a = read_u16(body, &mut bpos, &malformed)?;
    let _version = read_u32(body, &mut bpos, &malformed)?;
    let _length = read_u32(body, &mut bpos, &malformed)?;
    read_slice(body, &mut bpos, 16, &malformed)?; // bounds, unused
    let num_channels = read_u32(body, &mut bpos, &malformed)?;

    let mut best: Option<(u32, u32, Vec<u8>)> = None;
    for _ in 0..num_channels {
        let is_written = read_u32(body, &mut bpos, &malformed)?;
        if is_written == 0 {
            continue;
        }
        let chan_len = read_u32(body, &mut bpos, &malformed)? as usize;
        let chan_start = bpos;
        let chan_end = chan_start
            .checked_add(chan_len)
            .filter(|&e| e <= body.len())
            .ok_or_else(|| malformed("channel exceeds sample bounds"))?;

        let _unused_depth = read_u32(body, &mut bpos, &malformed)?;
        let top = read_u32(body, &mut bpos, &malformed)?;
        let left = read_u32(body, &mut bpos, &malformed)?;
        let bottom = read_u32(body, &mut bpos, &malformed)?;
        let right = read_u32(body, &mut bpos, &malformed)?;
        let depth = read_u16(body, &mut bpos, &malformed)?;
        let compression = read_u8(body, &mut bpos, &malformed)?;

        let width = right.saturating_sub(left);
        let height = bottom.saturating_sub(top);
        let bitmap = body
            .get(bpos..chan_end)
            .ok_or_else(|| malformed("truncated channel bitmap"))?;
        let mask = decode_channel(bitmap, width as usize, height as usize, depth, compression)?;
        bpos = chan_end;

        let area = u64::from(width) * u64::from(height);
        let is_better = best
            .as_ref()
            .is_none_or(|(w, h, _)| area > u64::from(*w) * u64::from(*h));
        if is_better {
            best = Some((width, height, mask));
        }
    }

    let (width, height, mask) = best.ok_or_else(|| malformed("no populated channel"))?;
    let pad = (4 - sample_len % 4) % 4;
    let consumed = 4 + sample_len + pad;
    if consumed > data.len() {
        return Err(malformed("padding exceeds buffer"));
    }

    Ok((SampEntry { guid, width, height, mask }, consumed))
}

fn decode_channel(bitmap: &[u8], width: usize, height: usize, depth: u16, compression: u8) -> Result<Vec<u8>> {
    if depth != 8 {
        return Err(ConverterError::Malformed(format!("samp entry: unsupported channel depth {depth}")));
    }
    match compression {
        0 => {
            let expected = width
                .checked_mul(height)
                .ok_or_else(|| ConverterError::Malformed("samp entry: bitmap dimensions overflow".into()))?;
            bitmap
                .get(..expected)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| ConverterError::Malformed("samp entry: truncated raw bitmap".into()))
        }
        1 => decode_packbits_rows(bitmap, width, height),
        other => Err(ConverterError::Malformed(format!("samp entry: unsupported compression {other}"))),
    }
}

/// PackBits-decodes `height` scanlines, each prefixed (in a table at the
/// front of `data`) by its own compressed byte count — see module docs.
fn decode_packbits_rows(data: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    let malformed = || ConverterError::Malformed("samp entry: truncated RLE bitmap".into());

    let table_len = height.checked_mul(2).ok_or_else(malformed)?;
    let row_lens_bytes = data.get(..table_len).ok_or_else(malformed)?;

    let mut out = Vec::with_capacity(width * height);
    let mut pos = table_len;
    for row_len_bytes in row_lens_bytes.chunks_exact(2) {
        let row_len = u16::from_be_bytes([row_len_bytes[0], row_len_bytes[1]]) as usize;
        let row_end = pos.checked_add(row_len).filter(|&e| e <= data.len()).ok_or_else(malformed)?;
        out.extend_from_slice(&decode_packbits_row(&data[pos..row_end], width)?);
        pos = row_end;
    }
    Ok(out)
}

fn decode_packbits_row(row: &[u8], width: usize) -> Result<Vec<u8>> {
    let malformed = || ConverterError::Malformed("samp entry: malformed PackBits row".into());

    let mut out = Vec::with_capacity(width);
    let mut i = 0usize;
    while i < row.len() {
        let n = row[i] as i8;
        i += 1;
        if n == -128 {
            continue; // no-op filler byte
        } else if n < 0 {
            let count = (-(n as i32) + 1) as usize;
            let byte = *row.get(i).ok_or_else(malformed)?;
            i += 1;
            out.extend(std::iter::repeat_n(byte, count));
        } else {
            let count = n as usize + 1;
            let chunk = row.get(i..i + count).ok_or_else(malformed)?;
            out.extend_from_slice(chunk);
            i += count;
        }
    }
    if out.len() != width {
        return Err(ConverterError::Malformed(format!(
            "samp entry: decoded PackBits row is {} bytes, expected width {width}",
            out.len()
        )));
    }
    Ok(out)
}

fn read_u8(data: &[u8], pos: &mut usize, malformed: &dyn Fn(&str) -> ConverterError) -> Result<u8> {
    let byte = *data.get(*pos).ok_or_else(|| malformed("truncated (u8)"))?;
    *pos += 1;
    Ok(byte)
}

fn read_u16(data: &[u8], pos: &mut usize, malformed: &dyn Fn(&str) -> ConverterError) -> Result<u16> {
    let bytes = read_slice(data, pos, 2, malformed)?;
    Ok(u16::from_be_bytes(bytes.try_into().unwrap()))
}

fn read_u32(data: &[u8], pos: &mut usize, malformed: &dyn Fn(&str) -> ConverterError) -> Result<u32> {
    let bytes = read_slice(data, pos, 4, malformed)?;
    Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
}

fn read_slice<'a>(
    data: &'a [u8],
    pos: &mut usize,
    len: usize,
    malformed: &dyn Fn(&str) -> ConverterError,
) -> Result<&'a [u8]> {
    let end = pos.checked_add(len).filter(|&e| e <= data.len()).ok_or_else(|| malformed("truncated"))?;
    let slice = &data[*pos..end];
    *pos = end;
    Ok(slice)
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

    #[test]
    fn decodes_real_photoshop_samp_entry() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/RakeBrushpackPhotoshopFinal.abr"),
        )
        .unwrap();

        // Skip the 4-byte header and the `8BIM samp <len>` section wrapper.
        let samp_len = u32::from_be_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let samp_start = 16;
        let entries = read_samp_section(&bytes[samp_start..samp_start + samp_len]).unwrap();

        assert_eq!(entries.len(), 22);
        let first = &entries[0];
        assert_eq!(first.guid, "3cdb6467-4f93-c341-a8f8-f6962a380c9c");
        assert_eq!(first.width, 1003);
        assert_eq!(first.height, 2298);
        assert_eq!(first.mask.len(), 1003 * 2298);
        // A rake-brush tip should have transparent corners and non-trivial
        // texture in the middle, not a uniform fill.
        assert_eq!(first.mask[0], 0);
        assert!(first.mask.iter().any(|&b| b != 0));
    }
}
