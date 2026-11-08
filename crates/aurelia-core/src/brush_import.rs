//! Brush stamp import: GIMP `.gbr` and Photoshop `.abr` sampled tips.
//!
//! The imported payload is a single 8-bit alpha mask; the brush colour remains the active
//! foreground colour, which is how both formats behave in their host applications. Files are
//! parsed from bytes through the `assets` seam, so the importer never opens paths itself.
//!
//! `.gbr` is fully supported. `.abr` legacy v1/v2 sampled brushes are decoded from their entry
//! stream. Modern (v6+) `.abr` containers are not fully described here; the decoder falls back to
//! scanning for the sampled-tip bitmap header used inside them, which covers raw (uncompressed)
//! tips and reports `NoStamps` when it cannot find any. RLE tips inside v6+ containers are not
//! decoded yet.

use std::path::Path;

/// One imported alpha stamp, already normalized to 8-bit coverage.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushStamp {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// `width * height` coverage values, row-major.
    pub alpha: Vec<u8>,
}

impl BrushStamp {
    /// Bilinear coverage at a point relative to the stamp centre, for a requested radius.
    ///
    /// The stamp keeps its aspect ratio: the larger dimension spans `2 * radius`, the smaller
    /// dimension is proportionally narrower. Points outside are zero.
    pub fn coverage(&self, dx: f32, dy: f32, radius: f32) -> f32 {
        if self.width == 0 || self.height == 0 || radius <= 0.0 {
            return 0.0;
        }
        let max_dim = self.width.max(self.height) as f32;
        let half_w = radius * (self.width as f32 / max_dim);
        let half_h = radius * (self.height as f32 / max_dim);
        if half_w <= 0.0 || half_h <= 0.0 || dx.abs() > half_w || dy.abs() > half_h {
            return 0.0;
        }
        let u = ((dx / half_w) * 0.5 + 0.5) * (self.width - 1) as f32;
        let v = ((dy / half_h) * 0.5 + 0.5) * (self.height - 1) as f32;
        let x0 = u.floor().clamp(0.0, (self.width - 1) as f32) as u32;
        let y0 = v.floor().clamp(0.0, (self.height - 1) as f32) as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fx = u - x0 as f32;
        let fy = v - y0 as f32;
        let at = |x: u32, y: u32| self.alpha[(y * self.width + x) as usize] as f32;
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        ((top * (1.0 - fy) + bottom * fy) / 255.0).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrushImportError {
    TooSmall,
    UnsupportedFormat,
    NoStamps,
    Corrupt(&'static str),
}

impl std::fmt::Display for BrushImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooSmall => write!(formatter, "file is too small to be a brush"),
            Self::UnsupportedFormat => write!(formatter, "unsupported brush file format"),
            Self::NoStamps => write!(formatter, "no sampled brush stamps found"),
            Self::Corrupt(reason) => write!(formatter, "corrupt brush file: {reason}"),
        }
    }
}

impl std::error::Error for BrushImportError {}

const MAX_STAMP_DIMENSION: u32 = 4096;
const MAX_STAMPS: usize = 256;

/// Decodes a brush file by its extension (`.gbr` or `.abr`), falling back to content sniffing.
pub fn decode_brush_file(
    bytes: &[u8],
    path: Option<&Path>,
) -> Result<Vec<BrushStamp>, BrushImportError> {
    let extension = path
        .and_then(|path| path.extension())
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("gbr") => decode_gbr(bytes),
        Some("abr") => decode_abr(bytes),
        _ => decode_abr(bytes).or_else(|_| decode_gbr(bytes)),
    }
}

fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn be_i32(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// GIMP `.gbr`: 4 big-endian u32 values, a NUL-terminated name, then 1 or 4 bytes per pixel.
fn decode_gbr(bytes: &[u8]) -> Result<Vec<BrushStamp>, BrushImportError> {
    if bytes.len() < 16 {
        return Err(BrushImportError::TooSmall);
    }
    let width = be_u32(bytes, 0).ok_or(BrushImportError::TooSmall)?;
    let height = be_u32(bytes, 4).ok_or(BrushImportError::TooSmall)?;
    let bpp = be_u32(bytes, 8).ok_or(BrushImportError::TooSmall)?;
    let magic = be_u32(bytes, 12).ok_or(BrushImportError::TooSmall)?;
    if magic != 2 || !matches!(bpp, 1 | 4) {
        return Err(BrushImportError::UnsupportedFormat);
    }
    if width == 0
        || height == 0
        || width > MAX_STAMP_DIMENSION
        || height > MAX_STAMP_DIMENSION
    {
        return Err(BrushImportError::Corrupt("stamp dimensions out of range"));
    }

    let mut name_end = 16;
    while name_end < bytes.len() && bytes[name_end] != 0 {
        name_end += 1;
    }
    if name_end >= bytes.len() {
        return Err(BrushImportError::Corrupt("unterminated brush name"));
    }
    let name = String::from_utf8_lossy(&bytes[16..name_end]).trim().to_string();
    let data_start = name_end + 1;
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(BrushImportError::Corrupt("stamp size overflow"))?;
    let expected = pixels
        .checked_mul(bpp as usize)
        .ok_or(BrushImportError::Corrupt("stamp size overflow"))?;
    let data = bytes
        .get(data_start..data_start + expected)
        .ok_or(BrushImportError::Corrupt("truncated pixel data"))?;

    let mut alpha = Vec::with_capacity(pixels);
    for index in 0..pixels {
        alpha.push(if bpp == 1 {
            data[index]
        } else {
            data[index * 4 + 3]
        });
    }
    Ok(vec![BrushStamp {
        name: if name.is_empty() { "Imported Brush".into() } else { name },
        width,
        height,
        alpha,
    }])
}

/// Photoshop `.abr`: legacy v1/v2 entry stream, or a sampled-tip scan for later containers.
fn decode_abr(bytes: &[u8]) -> Result<Vec<BrushStamp>, BrushImportError> {
    if bytes.len() < 4 {
        return Err(BrushImportError::TooSmall);
    }
    match be_u16(bytes, 0) {
        Some(1 | 2) => decode_abr_legacy(bytes),
        Some(6 | 7 | 9 | 10) => scan_abr_tips(bytes),
        _ => Err(BrushImportError::UnsupportedFormat),
    }
}

fn decode_abr_legacy(bytes: &[u8]) -> Result<Vec<BrushStamp>, BrushImportError> {
    let version = be_u16(bytes, 0).unwrap_or(0);
    let count = be_u16(bytes, 2).ok_or(BrushImportError::TooSmall)? as usize;
    let mut cursor = 4usize;
    let mut stamps = Vec::new();
    for index in 0..count.min(MAX_STAMPS) {
        let brush_type = be_u16(bytes, cursor).ok_or(BrushImportError::Corrupt("truncated entry"))?;
        cursor += 2;
        let entry_len =
            be_u32(bytes, cursor).ok_or(BrushImportError::Corrupt("truncated entry"))? as usize;
        cursor += 4;
        let entry_end = cursor
            .checked_add(entry_len)
            .ok_or(BrushImportError::Corrupt("entry length overflow"))?;
        if entry_end > bytes.len() {
            return Err(BrushImportError::Corrupt("entry exceeds file"));
        }
        if brush_type != 2 {
            cursor = entry_end;
            continue;
        }

        // spacing (u32), anti-aliasing (u16)
        cursor = cursor
            .checked_add(6)
            .ok_or(BrushImportError::Corrupt("entry header overflow"))?;
        if cursor > entry_end {
            return Err(BrushImportError::Corrupt("entry header truncated"));
        }
        let name = if version == 2 {
            let chars = be_u32(bytes, cursor).unwrap_or(0) as usize;
            cursor += 4;
            let name_bytes = chars
                .checked_mul(2)
                .ok_or(BrushImportError::Corrupt("name length overflow"))?;
            if cursor + name_bytes > entry_end {
                return Err(BrushImportError::Corrupt("brush name truncated"));
            }
            let mut units = Vec::with_capacity(chars);
            for i in 0..chars {
                units.push(be_u16(bytes, cursor + i * 2).unwrap_or(0));
            }
            if units.last() == Some(&0) {
                units.pop();
            }
            cursor += name_bytes;
            String::from_utf16_lossy(&units)
        } else {
            String::new()
        };

        // reserved byte + four i16 bounds/resolution values
        cursor = cursor
            .checked_add(1 + 8)
            .ok_or(BrushImportError::Corrupt("entry header overflow"))?;
        let stamp = parse_bitmap_header(bytes, &mut cursor, entry_end, &name, index)?;
        stamps.push(stamp);
        cursor = entry_end.max(cursor);
    }
    if stamps.is_empty() {
        Err(BrushImportError::NoStamps)
    } else {
        Ok(stamps)
    }
}

/// Parses the sampled-tip bitmap header shared by legacy entries and modern `samp` blocks.
fn parse_bitmap_header(
    bytes: &[u8],
    cursor: &mut usize,
    limit: usize,
    name: &str,
    index: usize,
) -> Result<BrushStamp, BrushImportError> {
    let top = be_i32(bytes, *cursor).ok_or(BrushImportError::Corrupt("bitmap header truncated"))?;
    let left =
        be_i32(bytes, *cursor + 4).ok_or(BrushImportError::Corrupt("bitmap header truncated"))?;
    let bottom =
        be_i32(bytes, *cursor + 8).ok_or(BrushImportError::Corrupt("bitmap header truncated"))?;
    let right =
        be_i32(bytes, *cursor + 12).ok_or(BrushImportError::Corrupt("bitmap header truncated"))?;
    let depth = be_u16(bytes, *cursor + 16)
        .ok_or(BrushImportError::Corrupt("bitmap header truncated"))?;
    let compression =
        *bytes.get(*cursor + 18).ok_or(BrushImportError::Corrupt("bitmap header truncated"))?;
    *cursor += 19;

    let width = right.saturating_sub(left);
    let height = bottom.saturating_sub(top);
    if width <= 0
        || height <= 0
        || width as u32 > MAX_STAMP_DIMENSION
        || height as u32 > MAX_STAMP_DIMENSION
        || !matches!(depth, 8 | 16)
        || compression > 1
    {
        return Err(BrushImportError::Corrupt("invalid sampled tip"));
    }
    let bpp = (depth / 8) as usize;
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(BrushImportError::Corrupt("stamp size overflow"))?;
    let expected = pixels
        .checked_mul(bpp)
        .ok_or(BrushImportError::Corrupt("stamp size overflow"))?;

    let alpha = if compression == 0 {
        if *cursor + expected > limit {
            return Err(BrushImportError::Corrupt("truncated sampled pixels"));
        }
        let data = &bytes[*cursor..*cursor + expected];
        *cursor += expected;
        let mut alpha = Vec::with_capacity(pixels);
        for index in 0..pixels {
            alpha.push(if depth == 8 {
                data[index]
            } else {
                let value = u16::from_be_bytes([data[index * 2], data[index * 2 + 1]]);
                (value >> 8) as u8
            });
        }
        alpha
    } else {
        decode_abr_rle(bytes, cursor, limit, width as usize, height as usize, bpp)?
    };

    let fallback = format!("Imported Brush {}", index + 1);
    Ok(BrushStamp {
        name: if name.trim().is_empty() { fallback } else { name.trim().to_string() },
        width: width as u32,
        height: height as u32,
        alpha,
    })
}

/// Photoshop row-wise PackBits RLE: `height` big-endian u16 row lengths, then packed rows.
fn decode_abr_rle(
    bytes: &[u8],
    cursor: &mut usize,
    limit: usize,
    width: usize,
    height: usize,
    bpp: usize,
) -> Result<Vec<u8>, BrushImportError> {
    if *cursor + height * 2 > limit {
        return Err(BrushImportError::Corrupt("truncated RLE row table"));
    }
    let mut row_lengths = Vec::with_capacity(height);
    for row in 0..height {
        let at = *cursor + row * 2;
        row_lengths.push(be_u16(bytes, at).unwrap_or(0) as usize);
    }
    *cursor += height * 2;

    let row_bytes = width
        .checked_mul(bpp)
        .ok_or(BrushImportError::Corrupt("row size overflow"))?;
    let mut alpha = Vec::with_capacity(width * height);
    for length in row_lengths {
        if *cursor + length > limit {
            return Err(BrushImportError::Corrupt("truncated RLE row"));
        }
        let packed = &bytes[*cursor..*cursor + length];
        *cursor += length;
        let mut decoded = vec![0u8; row_bytes];
        if !unpack_bits_row(packed, &mut decoded) {
            return Err(BrushImportError::Corrupt("invalid PackBits row"));
        }
        for x in 0..width {
            alpha.push(if bpp == 1 {
                decoded[x]
            } else {
                decoded[x * bpp + bpp - 1]
            });
        }
    }
    Ok(alpha)
}

fn unpack_bits_row(packed: &[u8], out: &mut [u8]) -> bool {
    let mut source = 0usize;
    let mut destination = 0usize;
    while source < packed.len() && destination < out.len() {
        let control = packed[source] as i8;
        source += 1;
        if control >= 0 {
            let count = control as usize + 1;
            if source + count > packed.len() || destination + count > out.len() {
                return false;
            }
            out[destination..destination + count].copy_from_slice(&packed[source..source + count]);
            source += count;
            destination += count;
        } else if control != -128 {
            let count = (1 - control as i32) as usize;
            if source >= packed.len() || destination + count > out.len() {
                return false;
            }
            let value = packed[source];
            source += 1;
            out[destination..destination + count].fill(value);
            destination += count;
        }
    }
    destination == out.len()
}

/// Modern `.abr` containers put sampled-tip bitmap headers inside `samp` blocks. Without a full
/// descriptor parser, scan for the shared header shape, accept raw tips only, and skip over the
/// pixels already consumed so a nested payload cannot be mistaken for another header.
fn scan_abr_tips(bytes: &[u8]) -> Result<Vec<BrushStamp>, BrushImportError> {
    let mut stamps = Vec::new();
    let mut at = 4usize;
    while at + 19 <= bytes.len() && stamps.len() < MAX_STAMPS {
        let top = be_i32(bytes, at).unwrap_or(0);
        let left = be_i32(bytes, at + 4).unwrap_or(0);
        let bottom = be_i32(bytes, at + 8).unwrap_or(0);
        let right = be_i32(bytes, at + 12).unwrap_or(0);
        let width = right.saturating_sub(left);
        let height = bottom.saturating_sub(top);
        let depth = be_u16(bytes, at + 16).unwrap_or(0);
        let compression = bytes.get(at + 18).copied().unwrap_or(255);
        if width > 0
            && height > 0
            && width as u32 <= MAX_STAMP_DIMENSION
            && height as u32 <= MAX_STAMP_DIMENSION
            && matches!(depth, 8 | 16)
            && compression == 0
        {
            let bpp = (depth / 8) as usize;
            let expected = (width as usize)
                .saturating_mul(height as usize)
                .saturating_mul(bpp);
            if at + 19 + expected <= bytes.len() {
                let mut cursor = at;
                if let Ok(stamp) =
                    parse_bitmap_header(bytes, &mut cursor, bytes.len(), "", stamps.len())
                {
                    stamps.push(stamp);
                    at = cursor;
                    continue;
                }
            }
        }
        at += 1;
    }
    if stamps.is_empty() {
        Err(BrushImportError::NoStamps)
    } else {
        Ok(stamps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gbr(width: u32, height: u32, name: &str, alpha: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(alpha);
        bytes
    }

    #[test]
    fn gbr_is_decoded_with_its_name_and_alpha() {
        let bytes = gbr(3, 2, "Soft Dot", &[0, 64, 128, 192, 255, 32]);
        let stamps = decode_gbr(&bytes).expect("decode");
        assert_eq!(stamps.len(), 1);
        let stamp = &stamps[0];
        assert_eq!(stamp.name, "Soft Dot");
        assert_eq!((stamp.width, stamp.height), (3, 2));
        assert_eq!(stamp.alpha, vec![0, 64, 128, 192, 255, 32]);
        assert!(stamp.coverage(0.0, 0.0, 10.0) > 0.0);
        assert_eq!(stamp.coverage(11.0, 0.0, 10.0), 0.0);
    }

    #[test]
    fn malformed_gbr_is_rejected_not_panicked() {
        assert!(decode_gbr(b"short").is_err());
        let mut bad = gbr(2, 2, "bad", &[0; 4]);
        bad[12..16].copy_from_slice(&9u32.to_be_bytes());
        assert!(decode_gbr(&bad).is_err());
        let mut truncated = gbr(2, 2, "bad", &[0; 4]);
        truncated.pop();
        assert!(decode_gbr(&truncated).is_err());
        let mut huge = gbr(1, 1, "huge", &[0]);
        huge[0..4].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(decode_gbr(&huge).is_err());
    }

    fn abr_legacy_raw(name: &str, alpha: &[u8], width: u32, height: u32) -> Vec<u8> {
        let name_units: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_be_bytes()); // spacing (placeholder)
        body.extend_from_slice(&0u16.to_be_bytes()); // anti-aliasing
        body.extend_from_slice(&(name_units.len() as u32).to_be_bytes());
        for unit in &name_units {
            body.extend_from_slice(&unit.to_be_bytes());
        }
        body.push(0); // reserved
        for _ in 0..4 {
            body.extend_from_slice(&0i16.to_be_bytes());
        }
        body.extend_from_slice(&0i32.to_be_bytes()); // top
        body.extend_from_slice(&0i32.to_be_bytes()); // left
        body.extend_from_slice(&(height as i32).to_be_bytes());
        body.extend_from_slice(&(width as i32).to_be_bytes());
        body.extend_from_slice(&8u16.to_be_bytes());
        body.push(0); // raw
        body.extend_from_slice(alpha);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&2u16.to_be_bytes()); // sampled
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&body);
        bytes
    }

    #[test]
    fn legacy_abr_raw_stamp_is_decoded() {
        let bytes = abr_legacy_raw("Legacy", &[0, 100, 200, 255], 2, 2);
        let stamps = decode_abr(&bytes).expect("decode");
        assert_eq!(stamps[0].name, "Legacy");
        assert_eq!(stamps[0].alpha, vec![0, 100, 200, 255]);
    }

    #[test]
    fn modern_abr_raw_tip_scan_finds_the_stamp() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&6u16.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(b"8BIMsamp");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        // Deliberately no descriptor parser: the bitmap header itself is enough for a raw tip.
        bytes.extend_from_slice(&0i32.to_be_bytes());
        bytes.extend_from_slice(&0i32.to_be_bytes());
        bytes.extend_from_slice(&2i32.to_be_bytes());
        bytes.extend_from_slice(&2i32.to_be_bytes());
        bytes.extend_from_slice(&8u16.to_be_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&[10, 20, 30, 40]);
        let stamps = decode_abr(&bytes).expect("scan");
        assert_eq!(stamps.len(), 1);
        assert_eq!(stamps[0].alpha, vec![10, 20, 30, 40]);
    }

    #[test]
    fn unknown_abr_versions_are_refused() {
        let bytes = [0u8, 3, 0, 1, 0, 0];
        assert_eq!(decode_abr(&bytes), Err(BrushImportError::UnsupportedFormat));
    }
}

#[cfg(test)]
mod rle_tests {
    use super::*;

    #[test]
    fn packbits_rows_expand_literals_and_runs() {
        let mut out = [0u8; 6];
        assert!(unpack_bits_row(&[0x02, 7, 8, 9, 0xfe, 5], &mut out));
        assert_eq!(out, [7, 8, 9, 5, 5, 5]);

        let mut short = [0u8; 4];
        assert!(!unpack_bits_row(&[0x05, 1], &mut short), "literal overruns");
        assert!(!unpack_bits_row(&[0xfd], &mut short), "run missing its byte");
    }

    #[test]
    fn rle_abr_rows_are_unpacked() {
        // One 2x2 8-bit row pair: row table (3, 2), then PackBits rows.
        let bytes = [0u8, 3, 0, 3, 0x01, 10, 20, 0x01, 30, 30];
        let mut cursor = 0;
        let alpha = decode_abr_rle(&bytes, &mut cursor, bytes.len(), 2, 2, 1).expect("rle");
        assert_eq!(alpha, vec![10, 20, 30, 30]);
        assert_eq!(cursor, bytes.len());
    }
}
