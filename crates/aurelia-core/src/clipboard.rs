//! Copy/paste payloads and the bitmap clipboard encoding.
//!
//! Two things live here, deliberately separate from the Win32 clipboard calls:
//!
//! * [`ClipboardImage`] is the **internal** clipboard. It keeps the document's storage
//!   width, so copying and pasting inside Aurelia is lossless for RGBA16 documents and does
//!   not round-trip through the 8-bit system clipboard.
//! * [`encode_dibv5`] / [`decode_dib`] are the 8-bit DIB (device-independent bitmap)
//!   conversions used for interoperability with other applications. Windows' clipboard image
//!   formats are 8 bits per channel; the internal clipboard is what preserves precision.
//!
//! The compositor applies a layer's offset when drawing, so every canvas coordinate here is
//! translated by the source layer's offset on copy and written back with offset zero on paste.

use crate::layer::{Layer, LayerBuffer, PixelFormat};
use crate::selection::SelectionMask;

/// A rectangular canvas-space region captured for paste.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipboardImage {
    pub width: u32,
    pub height: u32,
    /// Canvas position of the region's top-left corner.
    pub origin_x: i32,
    pub origin_y: i32,
    pub format: PixelFormat,
    /// Tightly packed RGBA storage in `format`: `width * height * 4` channels.
    pub pixels: LayerBuffer,
}

impl ClipboardImage {
    /// Captures the active layer through `selection` (or the whole canvas when there is none).
    ///
    /// Selection coverage scales the copied alpha, the same way a feathered copy behaves in
    /// GIMP/Photoshop: pixels outside the selection become transparent rather than being
    /// cropped, so later paste/transform edits see the soft edge.
    pub fn from_layer(
        layer: &Layer,
        canvas_width: u32,
        canvas_height: u32,
        selection: Option<&SelectionMask>,
    ) -> Option<Self> {
        let bounds = selection
            .and_then(SelectionMask::bounds)
            .unwrap_or((0, 0, canvas_width, canvas_height));
        let (x0, y0, x1, y1) = bounds;
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let width = x1 - x0;
        let height = y1 - y0;
        let format = layer.format();
        let mut pixels = match format {
            PixelFormat::Srgb8 => LayerBuffer::Srgb8(vec![0u8; (width * height * 4) as usize]),
            PixelFormat::Srgb16 => LayerBuffer::Srgb16(vec![0u16; (width * height * 4) as usize]),
        };

        for y in 0..height {
            for x in 0..width {
                let canvas_x = x0 + x;
                let canvas_y = y0 + y;
                let coverage = selection
                    .map(|mask| mask.get_value(canvas_x, canvas_y) as u32)
                    .unwrap_or(255);
                if coverage == 0 {
                    continue;
                }
                // Canvas -> layer space; offsets can be negative, so reject out-of-layer points.
                let lx = canvas_x as i32 - layer.offset_x;
                let ly = canvas_y as i32 - layer.offset_y;
                if lx < 0 || ly < 0 {
                    continue;
                }
                let Some(source) = layer.pixel_index(lx as u32, ly as u32) else {
                    continue;
                };
                let destination = ((y * width + x) * 4) as usize;
                match (&mut pixels, layer.buffer()) {
                    (LayerBuffer::Srgb8(dst), LayerBuffer::Srgb8(src)) => {
                        if source + 4 <= src.len() && destination + 4 <= dst.len() {
                            let alpha = (src[source + 3] as u32 * coverage + 127) / 255;
                            dst[destination..destination + 3]
                                .copy_from_slice(&src[source..source + 3]);
                            dst[destination + 3] = alpha as u8;
                        }
                    }
                    (LayerBuffer::Srgb16(dst), LayerBuffer::Srgb16(src)) => {
                        if source + 4 <= src.len() && destination + 4 <= dst.len() {
                            let alpha = (src[source + 3] as u32 * coverage + 127) / 255;
                            dst[destination..destination + 3]
                                .copy_from_slice(&src[source..source + 3]);
                            dst[destination + 3] = alpha.min(65535) as u16;
                        }
                    }
                    _ => return None,
                }
            }
        }

        Some(Self {
            width,
            height,
            origin_x: x0 as i32,
            origin_y: y0 as i32,
            format,
            pixels,
        })
    }

    /// Builds a canvas-sized layer with the captured region at its original position.
    ///
    /// The layer is deliberately the canvas size and offset zero: a smaller layer with an
    /// offset would be a second coordinate model for every tool that follows, and paste-at-
    /// origin is the behaviour users expect from both references.
    pub fn to_layer(&self, id: u64, name: impl Into<String>, canvas_width: u32, canvas_height: u32) -> Layer {
        let mut layer = Layer::new(id, name, canvas_width, canvas_height);
        if self.format == PixelFormat::Srgb16 {
            layer.convert_format(PixelFormat::Srgb16);
        }
        for y in 0..self.height {
            for x in 0..self.width {
                let canvas_x = self.origin_x + x as i32;
                let canvas_y = self.origin_y + y as i32;
                if canvas_x < 0 || canvas_y < 0 {
                    continue;
                }
                let (cx, cy) = (canvas_x as u32, canvas_y as u32);
                if cx >= canvas_width || cy >= canvas_height {
                    continue;
                }
                let source = ((y * self.width + x) * 4) as usize;
                let Some(destination) = layer.pixel_index(cx, cy) else {
                    continue;
                };
                match (&mut layer.buffer_mut(), &self.pixels) {
                    (LayerBuffer::Srgb8(dst), LayerBuffer::Srgb8(src)) => {
                        dst[destination..destination + 4]
                            .copy_from_slice(&src[source..source + 4]);
                    }
                    (LayerBuffer::Srgb16(dst), LayerBuffer::Srgb16(src)) => {
                        dst[destination..destination + 4]
                            .copy_from_slice(&src[source..source + 4]);
                    }
                    _ => {}
                }
            }
        }
        layer
    }

    /// An 8-bit RGBA view for the system clipboard (straight, not premultiplied).
    pub fn to_rgba8(&self) -> Vec<u8> {
        let size = (self.width * self.height * 4) as usize;
        let mut out = vec![0u8; size];
        match &self.pixels {
            LayerBuffer::Srgb8(src) => {
                let copy = size.min(src.len());
                out[..copy].copy_from_slice(&src[..copy]);
            }
            LayerBuffer::Srgb16(src) => {
                for (index, chunk) in src.chunks_exact(4).enumerate() {
                    let at = index * 4;
                    if at + 3 >= out.len() {
                        break;
                    }
                    out[at] = crate::pixel::srgb16_to_srgb8(chunk[0]);
                    out[at + 1] = crate::pixel::srgb16_to_srgb8(chunk[1]);
                    out[at + 2] = crate::pixel::srgb16_to_srgb8(chunk[2]);
                    out[at + 3] = crate::pixel::srgb16_to_srgb8(chunk[3]);
                }
            }
        }
        out
    }

    /// Wraps an externally supplied 8-bit image, e.g. from another application's clipboard.
    pub fn from_rgba8(width: u32, height: u32, origin_x: i32, origin_y: i32, rgba: &[u8]) -> Option<Self> {
        let size = (width * height * 4) as usize;
        if rgba.len() < size || width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            width,
            height,
            origin_x,
            origin_y,
            format: PixelFormat::Srgb8,
            pixels: LayerBuffer::Srgb8(rgba[..size].to_vec()),
        })
    }
}

/// Encodes 8-bit straight RGBA as a `BITMAPV5HEADER` DIB with 32-bit BGRA pixels.
///
/// The clipboard DIB is bottom-up unless the height is negative; this writes a positive
/// height and rows in bottom-up order, which is what every Windows application expects. Alpha
/// is written to the fourth byte and `BI_RGB` is used: some consumers only understand that
/// pair, and modern ones read the alpha byte from a 32-bit `BI_RGB` DIB.
pub fn encode_dibv5(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let pixel_bytes = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
    if width == 0 || height == 0 || rgba.len() < pixel_bytes {
        return None;
    }
    const HEADER: usize = 124;
    let mut out = vec![0u8; HEADER + pixel_bytes];

    fn put_u32(out: &mut [u8], offset: usize, value: u32) {
        out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    put_u32(&mut out, 0, HEADER as u32); // bV5Size
    put_u32(&mut out, 4, width); // bV5Width
    put_u32(&mut out, 8, height); // bV5Height (bottom-up)
    out[12..14].copy_from_slice(&1u16.to_le_bytes()); // bV5Planes
    out[14..16].copy_from_slice(&32u16.to_le_bytes()); // bV5BitCount
    put_u32(&mut out, 16, 3); // bV5Compression = BI_BITFIELDS
    put_u32(&mut out, 20, pixel_bytes as u32); // bV5SizeImage
    put_u32(&mut out, 24, 2835); // bV5XPelsPerMeter (~72 DPI)
    put_u32(&mut out, 28, 2835);
    put_u32(&mut out, 32, 0); // bV5ClrUsed
    put_u32(&mut out, 36, 0); // bV5ClrImportant
    put_u32(&mut out, 40, 0x00ff0000); // red mask
    put_u32(&mut out, 44, 0x0000ff00); // green mask
    put_u32(&mut out, 48, 0x000000ff); // blue mask
    put_u32(&mut out, 52, 0xff000000); // alpha mask
    put_u32(&mut out, 56, 0x73524742); // bV5CSType = 'sRGB'
    // Remaining V5 fields (endpoints, gamma) stay zero, which means "use sRGB defaults".

    for y in 0..height as usize {
        let source_row = y * width as usize * 4;
        let destination_row = HEADER + (height as usize - 1 - y) * width as usize * 4;
        for x in 0..width as usize {
            let s = source_row + x * 4;
            let d = destination_row + x * 4;
            out[d] = rgba[s + 2]; // B
            out[d + 1] = rgba[s + 1]; // G
            out[d + 2] = rgba[s]; // R
            out[d + 3] = rgba[s + 3]; // A
        }
    }
    Some(out)
}

/// Decodes a clipboard DIB (BITMAPINFOHEADER, V4 or V5) into straight 8-bit RGBA.
///
/// Accepts uncompressed `BI_RGB` and `BI_BITFIELDS` inputs. `BI_BITFIELDS` channel masks are
/// honoured, so an RGBA-ordered DIB from another application decodes with the right channels
/// rather than assuming BGRA. 24-bit DIBs expand with opaque alpha. For plain 32-bit `BI_RGB`
/// the fourth byte is alpha, and an entirely zero alpha channel is treated as "unused" (some
/// producers leave it clear); an explicit alpha mask disables that heuristic.
pub fn decode_dib(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if bytes.len() < 40 {
        return None;
    }
    let read_u32 = |offset: usize| -> Option<u32> {
        bytes
            .get(offset..offset + 4)
            .map(|slice| u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    };
    let header_size = read_u32(0)? as usize;
    if header_size < 40 || header_size > bytes.len() {
        return None;
    }
    let width = read_u32(4)? as i32;
    let height_raw = read_u32(8)? as i32;
    let planes = u16::from_le_bytes([*bytes.get(12)?, *bytes.get(13)?]);
    let bit_count = u16::from_le_bytes([*bytes.get(14)?, *bytes.get(15)?]);
    let compression = read_u32(16)?;
    if planes != 1 || width <= 0 || height_raw == 0 || !matches!(bit_count, 24 | 32) {
        return None;
    }
    // 0 = BI_RGB, 3 = BI_BITFIELDS. Anything else (RLE, JPEG, PNG) is not a plain DIB.
    if compression != 0 && compression != 3 {
        return None;
    }
    if compression == 3 && bit_count != 32 {
        return None;
    }

    // Where the pixel data starts and, for BI_BITFIELDS, the channel masks.
    let (pixel_offset, masks) = if compression == 3 {
        if header_size >= 52 {
            let masks = (
                read_u32(40)?,
                read_u32(44)?,
                read_u32(48)?,
                read_u32(52)?,
            );
            (header_size, Some(masks))
        } else {
            // BITMAPINFOHEADER + BI_BITFIELDS: the three masks follow the header; for a
            // 32-bit DIB an alpha mask follows them, and the pixels start after that.
            let offset = header_size + 12;
            if bytes.len() < offset {
                return None;
            }
            let masks = (read_u32(header_size)?, read_u32(header_size + 4)?, read_u32(header_size + 8)?, 0);
            (offset, Some(masks))
        }
    } else {
        (header_size, None)
    };

    let width = width as u32;
    let height = height_raw.unsigned_abs();
    let top_down = height_raw < 0;
    let stride = (width as usize * bit_count as usize).div_ceil(32) * 4;
    let needed = pixel_offset.checked_add(stride.checked_mul(height as usize)?)?;
    if bytes.len() < needed {
        return None;
    }
    let pixels = &bytes[pixel_offset..needed];
    let mut rgba = vec![0u8; (width * height * 4) as usize];

    fn masked_channel(raw: u32, mask: u32) -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let bits = (mask >> shift).count_ones();
        let value = (raw & mask) >> shift;
        let quantised = if bits >= 8 {
            value >> (bits - 8)
        } else {
            (value * 255) / ((1u32 << bits) - 1).max(1)
        };
        quantised.min(255) as u8
    }

    for y in 0..height as usize {
        let source_row = if top_down { y } else { height as usize - 1 - y } * stride;
        let destination_row = y * width as usize * 4;
        for x in 0..width as usize {
            let s = source_row + x * (bit_count as usize / 8);
            let d = destination_row + x * 4;
            if s + 2 >= pixels.len() {
                continue;
            }
            match masks {
                Some((red, green, blue, alpha)) => {
                    if s + 3 >= pixels.len() {
                        continue;
                    }
                    let raw = u32::from_le_bytes([
                        pixels[s],
                        pixels[s + 1],
                        pixels[s + 2],
                        pixels[s + 3],
                    ]);
                    rgba[d] = masked_channel(raw, red);
                    rgba[d + 1] = masked_channel(raw, green);
                    rgba[d + 2] = masked_channel(raw, blue);
                    rgba[d + 3] = if alpha == 0 { 255 } else { masked_channel(raw, alpha) };
                }
                None => {
                    rgba[d] = pixels[s + 2];
                    rgba[d + 1] = pixels[s + 1];
                    rgba[d + 2] = pixels[s];
                    rgba[d + 3] = if bit_count == 32 && s + 3 < pixels.len() {
                        pixels[s + 3]
                    } else {
                        255
                    };
                }
            }
        }
    }

    // A plain 32-bit DIB whose alpha is entirely zero carries no alpha information. An
    // explicit alpha mask is authoritative instead, so it is excluded.
    if compression == 0 && bit_count == 32 && rgba.chunks_exact(4).all(|px| px[3] == 0) {
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }
    Some((width, height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerMask;
    use glam::Vec2;

    fn eight_bit_layer() -> Layer {
        let mut layer = Layer::new(1, "Source", 8, 8);
        for (index, px) in layer.as_srgb8_exact_mut().chunks_exact_mut(4).enumerate() {
            px[0] = (index * 3) as u8;
            px[1] = (index * 5) as u8;
            px[2] = (index * 7) as u8;
            px[3] = 255;
        }
        layer
    }

    #[test]
    fn copy_and_paste_keep_position_and_bytes_at_8_and_16_bits() {
        let mut layer = eight_bit_layer();
        layer.offset_x = 2;
        layer.offset_y = 1;
        let clip = ClipboardImage::from_layer(&layer, 8, 8, None).expect("copy");
        assert_eq!((clip.origin_x, clip.origin_y), (0, 0));

        let pasted = clip.to_layer(2, "Pasted", 8, 8);
        for y in 0..8u32 {
            for x in 0..8u32 {
                let sx = x as i32 - 2;
                let sy = y as i32 - 1;
                let expected = if sx >= 0 && sy >= 0 {
                    layer.get_pixel(sx as u32, sy as u32)
                } else {
                    Some([0, 0, 0, 0])
                };
                assert_eq!(pasted.get_pixel(x, y), expected, "8-bit at ({x},{y})");
            }
        }

        // 16-bit keeps its storage exactly.
        let mut wide = Layer::new(3, "Wide", 4, 4);
        wide.convert_format(PixelFormat::Srgb16);
        for (index, px) in wide.as_srgb16_mut().expect("16-bit").chunks_exact_mut(4).enumerate() {
            px[0] = (index as u16).wrapping_mul(4099);
            px[1] = 1234;
            px[2] = 65535;
            px[3] = 65535;
        }
        let clip = ClipboardImage::from_layer(&wide, 4, 4, None).expect("wide copy");
        assert_eq!(clip.format, PixelFormat::Srgb16);
        let pasted = clip.to_layer(4, "Wide paste", 4, 4);
        assert_eq!(pasted.as_srgb16().expect("16-bit"), wide.as_srgb16().expect("16-bit"));
    }

    #[test]
    fn selection_bounds_and_feathered_coverage_are_honoured() {
        let layer = eight_bit_layer();
        let mut selection = SelectionMask::new(8, 8);
        selection.paint_circle(Vec2::new(4.0, 4.0), 2.0, 1.0, 1.0);
        let bounds = selection.bounds().expect("bounds");
        let clip = ClipboardImage::from_layer(&layer, 8, 8, Some(&selection)).expect("copy");
        assert_eq!((clip.width, clip.height), (bounds.2 - bounds.0, bounds.3 - bounds.1));
        assert_eq!((clip.origin_x, clip.origin_y), (bounds.0 as i32, bounds.1 as i32));

        // A pixel inside the dab is opaque; one outside it is transparent in the payload.
        let centre = (4 - bounds.0, 4 - bounds.1);
        let outside = (0, 0);
        if let LayerBuffer::Srgb8(px) = &clip.pixels {
            let at = ((centre.1 * clip.width + centre.0) * 4) as usize;
            assert_eq!(px[at + 3], 255);
            let at = ((outside.1 * clip.width + outside.0) * 4) as usize;
            assert_eq!(px[at + 3], 0);
        } else {
            panic!("expected 8-bit payload");
        }
    }

    #[test]
    fn dib_round_trip_preserves_rgba_and_row_order() {
        let (w, h) = (4u32, 3u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for (index, px) in rgba.chunks_exact_mut(4).enumerate() {
            px[0] = index as u8;
            px[1] = 200 - index as u8;
            px[2] = 17;
            px[3] = 128;
        }
        let dib = encode_dibv5(w, h, &rgba).expect("encode");
        assert_eq!(dib.len(), 124 + (w * h * 4) as usize);
        let (dw, dh, decoded) = decode_dib(&dib).expect("decode");
        assert_eq!((dw, dh), (w, h));
        assert_eq!(decoded, rgba);
    }

    #[test]
    fn twenty_four_bit_dib_expands_to_opaque_rgba() {
        // One 2x1 bottom-up 24-bit DIB: bottom row is blue, top row is red.
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        // 2 px * 3 bytes = 6, padded to 8.
        dib.extend_from_slice(&[255, 0, 0, 0, 0, 255, 0, 0]);
        let (w, h, rgba) = decode_dib(&dib).expect("decode 24-bit");
        assert_eq!((w, h), (2, 1));
        // DIB bytes are B,G,R: the first pixel is blue, the second red, both opaque.
        assert_eq!(&rgba[0..4], &[0, 0, 255, 255]);
        assert_eq!(&rgba[4..8], &[255, 0, 0, 255]);
    }

    #[test]
    fn malformed_dibs_are_rejected_not_panicked() {
        assert!(decode_dib(&[]).is_none());
        assert!(decode_dib(&[0u8; 39]).is_none());
        let mut bad = vec![0u8; 40];
        bad[0..4].copy_from_slice(&40u32.to_le_bytes());
        bad[4..8].copy_from_slice(&4i32.to_le_bytes());
        bad[8..12].copy_from_slice(&4i32.to_le_bytes());
        bad[12..14].copy_from_slice(&1u16.to_le_bytes());
        bad[14..16].copy_from_slice(&24u16.to_le_bytes());
        // Header claims a 4x4 image but no pixel data follows.
        assert!(decode_dib(&bad).is_none());
        // Unsupported compression is refused rather than guessed.
        bad[16..20].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode_dib(&bad).is_none());
    }

    #[test]
    fn converting_to_the_system_view_uses_rounded_16_bit_values() {
        let mut wide = Layer::new(1, "Wide", 1, 1);
        wide.convert_format(PixelFormat::Srgb16);
        if let Some(px) = wide.as_srgb16_mut() {
            px[0] = 65535;
            px[1] = 32896; // 128 * 257
            px[2] = 0;
            px[3] = 65535;
        }
        let clip = ClipboardImage::from_layer(&wide, 1, 1, None).expect("copy");
        let rgba = clip.to_rgba8();
        assert_eq!(rgba, vec![255, 128, 0, 255]);
    }

    /// A layer mask is not part of the copied pixels; the payload is the visible RGBA only.
    #[test]
    fn mask_is_not_copied_into_the_payload() {
        let mut layer = eight_bit_layer();
        layer.mask = Some(LayerMask::full(8, 8, 0));
        let clip = ClipboardImage::from_layer(&layer, 8, 8, None).expect("copy");
        if let LayerBuffer::Srgb8(px) = &clip.pixels {
            assert_eq!(px[3], 255, "mask coverage must not leak into the copied alpha");
        }
    }
}

#[cfg(test)]
mod dib_masks {
    use super::decode_dib;

    fn v5_header(width: u32, height: u32, masks: [u32; 4]) -> Vec<u8> {
        let mut header = vec![0u8; 124];
        header[0..4].copy_from_slice(&124u32.to_le_bytes());
        header[4..8].copy_from_slice(&width.to_le_bytes());
        header[8..12].copy_from_slice(&height.to_le_bytes());
        header[12..14].copy_from_slice(&1u16.to_le_bytes());
        header[14..16].copy_from_slice(&32u16.to_le_bytes());
        header[16..20].copy_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
        for (index, mask) in masks.iter().enumerate() {
            let at = 40 + index * 4;
            header[at..at + 4].copy_from_slice(&mask.to_le_bytes());
        }
        header
    }

    /// RGBA-ordered masks must be honoured; the old decoder always assumed BGRA.
    #[test]
    fn rgab_masks_decode_to_the_right_channels() {
        let mut dib = v5_header(1, 1, [0x000000ff, 0x0000ff00, 0x00ff0000, 0xff000000]);
        let pixel = (0x40u32 << 24) | (0x20u32 << 16) | (0x10u32 << 8) | 0x08u32;
        dib.extend_from_slice(&pixel.to_le_bytes());
        let (w, h, rgba) = decode_dib(&dib).expect("decode");
        assert_eq!((w, h), (1, 1));
        assert_eq!(rgba, vec![0x08, 0x10, 0x20, 0x40]);
    }

    /// An explicit alpha mask is authoritative: an all-zero alpha must stay transparent rather
    /// than being rewritten to opaque by the BI_RGB heuristic.
    #[test]
    fn explicit_alpha_mask_is_not_overridden() {
        let mut dib = v5_header(2, 1, [0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000]);
        dib.extend_from_slice(&0x00000000u32.to_le_bytes());
        dib.extend_from_slice(&0x80000000u32.to_le_bytes());
        let (_, _, rgba) = decode_dib(&dib).expect("decode");
        assert_eq!(rgba[3], 0);
        assert_eq!(rgba[7], 0x80);
    }

    /// A BITMAPINFOHEADER (`biSize = 40`) BI_BITFIELDS DIB carries its three masks immediately
    /// after the header; the pixel data starts after them.
    #[test]
    fn forty_byte_header_masks_shift_the_pixel_offset() {
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[16..20].copy_from_slice(&3u32.to_le_bytes());
        for mask in [0x00ff0000u32, 0x0000ff00, 0x000000ff] {
            dib.extend_from_slice(&mask.to_le_bytes());
        }
        dib.extend_from_slice(&0x00332211u32.to_le_bytes()); // B=0x11 G=0x22 R=0x33
        let (_, _, rgba) = decode_dib(&dib).expect("decode");
        assert_eq!(&rgba[0..3], &[0x33, 0x22, 0x11]);
        assert_eq!(rgba[3], 255, "no alpha mask means opaque");
    }
}
