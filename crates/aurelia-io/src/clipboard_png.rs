//! PNG clipboard format helpers.
//!
//! Windows does not have a fixed clipboard constant for PNG; applications register a format named
//! `PNG` (Chromium and others) or `image/png`. Keeping the codec here means the Win32 layer only
//! deals with handles and bytes, and the conversion is unit-tested without a clipboard.

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};

/// Upper bound accepted from an external clipboard image (64 megapixels, 256 MiB of RGBA).
pub const MAX_CLIPBOARD_PIXELS: u64 = 64 * 1024 * 1024;

/// Encodes straight RGBA8 as a PNG for the registered clipboard format.
pub fn encode_png_rgba8(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if width == 0 || height == 0 || rgba.len() < expected {
        return None;
    }
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(&rgba[..expected], width, height, ExtendedColorType::Rgba8)
        .ok()?;
    Some(out)
}

/// Decodes a PNG from the clipboard into straight RGBA8.
///
/// The IHDR dimensions are checked before decoding, so a crafted header cannot make the decoder
/// allocate a canvas larger than [`MAX_CLIPBOARD_PIXELS`] just to be rejected afterwards.
pub fn decode_png_rgba8(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if width == 0
        || height == 0
        || (width as u64) * (height as u64) > MAX_CLIPBOARD_PIXELS
    {
        return None;
    }
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
    let rgba = image.to_rgba8();
    if rgba.dimensions() != (width, height) {
        return None;
    }
    Some((width, height, rgba.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip_preserves_rgba() {
        let rgba = vec![
            255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 255, 254,
        ];
        let png = encode_png_rgba8(3, 2, &rgba).expect("encode");
        let (width, height, decoded) = decode_png_rgba8(&png).expect("decode");
        assert_eq!((width, height), (3, 2));
        assert_eq!(decoded, rgba);
    }

    #[test]
    fn malformed_and_oversized_pngs_are_rejected() {
        assert!(decode_png_rgba8(&[]).is_none());
        assert!(decode_png_rgba8(b"not a png").is_none());

        let mut huge = Vec::new();
        huge.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        huge.extend_from_slice(&13u32.to_be_bytes());
        huge.extend_from_slice(b"IHDR");
        huge.extend_from_slice(&100_000u32.to_be_bytes());
        huge.extend_from_slice(&100_000u32.to_be_bytes());
        assert!(
            decode_png_rgba8(&huge).is_none(),
            "dimensions are checked before decode"
        );
    }
}
