//! Image export.
//!
//! Every entry point here validates its geometry **before** allocating, and every rejection is
//! a structured error carrying the numbers that caused it. Three shape rules hold across the
//! module:
//!
//! * **Checked arithmetic.** Frame and sheet sizes are computed in `u64` with `checked_mul`;
//!   nothing multiplies two `u32`s into an overflow.
//! * **Exact buffers.** A frame must be exactly `width * height * 4` bytes. A short buffer is
//!   reported with its index, not passed to the encoder to fail later.
//! * **Streaming.** [`export_animated_gif_stream`], [`export_sprite_sheet_stream`] and
//!   [`export_frame_sequence_stream`] consume frames one at a time, so an animation export
//!   never has to hold every frame in memory. The owned-slice functions are thin wrappers kept
//!   for compatibility; they clone each frame because the encoder requires ownership.
//!
//! Output is written to a sibling `.part` file and renamed on success, so a rejected frame
//! never leaves a truncated image behind.

use aurelia_core::document::Document;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageBuffer, ImageEncoder, ImageFormat, Rgba};
use std::fs::File;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ExportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),
    #[error("Unsupported image format")]
    UnsupportedFormat,
    /// The requested frame size exceeds [`ExportLimits::max_dimension`].
    #[error("{width}x{height} is larger than the maximum export dimension of {max}")]
    DimensionTooLarge { width: u32, height: u32, max: u32 },
    /// The animation has more frames than [`ExportLimits::max_frames`].
    #[error("{frames} frames exceed the limit of {max}")]
    TooManyFrames { frames: usize, max: usize },
    /// The export would produce more pixel data than [`ExportLimits::max_total_bytes`].
    ///
    /// This is a policy bound on the size of one export job, applied uniformly. Peak *resident*
    /// memory differs per format: one frame for GIF and frame sequences, the whole sheet for a
    /// sprite sheet.
    #[error("the export needs {required} bytes of pixel data, above the {max}-byte limit")]
    AggregateTooLarge { required: u64, max: u64 },
    /// A frame buffer's length does not match the declared geometry.
    #[error("frame {index} has {actual} bytes, expected {expected}")]
    FrameSizeMismatch {
        index: usize,
        expected: usize,
        actual: usize,
    },
    /// The combined sprite-sheet dimensions do not fit the image container.
    #[error("a {width}x{height} sprite sheet cannot be encoded")]
    SheetTooLarge { width: u64, height: u64 },
}

/// Resource bounds applied to one export.
///
/// Defaults are sized for the editor's canvas clamps: 8192 px is the largest canvas the UI can
/// create, and 1 GiB allows roughly 125 frames at 1080p or 31 at 4K.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportLimits {
    /// Largest permitted width or height of a single frame, in pixels.
    pub max_dimension: u32,
    /// Largest permitted frame count.
    pub max_frames: usize,
    /// Largest permitted aggregate pixel data for one export, in bytes.
    pub max_total_bytes: u64,
}

impl Default for ExportLimits {
    fn default() -> Self {
        Self {
            max_dimension: 8192,
            max_frames: 600,
            max_total_bytes: 1 << 30,
        }
    }
}

impl ExportLimits {
    /// Bounds generous enough for any legitimate export, useful for tests and tooling that has
    /// already validated its own inputs.
    pub const fn unlimited() -> Self {
        Self {
            max_dimension: u32::MAX,
            max_frames: usize::MAX,
            max_total_bytes: u64::MAX,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
    Jpeg,
    WebP,
}

impl ExportFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
        }
    }

    pub fn image_format(&self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
            Self::WebP => ImageFormat::WebP,
        }
    }
}

// ── Validation ───────────────────────────────────────────────────────────────────────────────

/// Bytes in one `width * height` RGBA8 frame, or the reason the geometry is unacceptable.
fn frame_bytes(width: u32, height: u32, limits: &ExportLimits) -> Result<usize, ExportError> {
    if width == 0 || height == 0 {
        return Err(ExportError::UnsupportedFormat);
    }
    if width > limits.max_dimension || height > limits.max_dimension {
        return Err(ExportError::DimensionTooLarge {
            width,
            height,
            max: limits.max_dimension,
        });
    }
    let too_large = || ExportError::DimensionTooLarge {
        width,
        height,
        max: limits.max_dimension,
    };
    let bytes = (width as u64)
        .checked_mul(height as u64)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(too_large)?;
    usize::try_from(bytes).map_err(|_| too_large())
}

/// Checks the frame count and the aggregate pixel-data budget for one export.
fn check_budget(frames: usize, per_frame: usize, limits: &ExportLimits) -> Result<(), ExportError> {
    if frames > limits.max_frames {
        return Err(ExportError::TooManyFrames {
            frames,
            max: limits.max_frames,
        });
    }
    let required = (per_frame as u64).saturating_mul(frames as u64);
    if required > limits.max_total_bytes {
        return Err(ExportError::AggregateTooLarge {
            required,
            max: limits.max_total_bytes,
        });
    }
    Ok(())
}

fn check_frame_len(frame: &[u8], index: usize, expected: usize) -> Result<(), ExportError> {
    if frame.len() != expected {
        return Err(ExportError::FrameSizeMismatch {
            index,
            expected,
            actual: frame.len(),
        });
    }
    Ok(())
}

fn check_frame_len_16(frame: &[u16], index: usize, expected: usize) -> Result<(), ExportError> {
    if frame.len() != expected {
        return Err(ExportError::FrameSizeMismatch {
            index,
            expected,
            actual: frame.len(),
        });
    }
    Ok(())
}

/// Bytes in one RGBA16 frame, with the same geometry validation as [`frame_bytes`].
fn frame_bytes_16(width: u32, height: u32, limits: &ExportLimits) -> Result<usize, ExportError> {
    // Reuse the dimension checks (and their error shape) before widening the pixel size.
    let _ = frame_bytes(width, height, limits)?;
    let bytes = (width as u64)
        .checked_mul(height as u64)
        .and_then(|pixels| pixels.checked_mul(8))
        .ok_or(ExportError::DimensionTooLarge {
            width,
            height,
            max: limits.max_dimension,
        })?;
    usize::try_from(bytes).map_err(|_| ExportError::DimensionTooLarge {
        width,
        height,
        max: limits.max_dimension,
    })
}

/// Whether a PNG export of this document will be written at 16 bits.
///
/// Exposed for the UI's status text so the user knows which depth was written.
pub fn png_will_be_16bit(doc: &Document) -> bool {
    document_needs_16bit(doc)
}

/// Whether a PNG export should be written at 16 bits.
///
/// The document tag is authoritative, but a mixed document with a 16-bit layer also needs the
/// wide path: compositing it at 8 bits would discard the layer's sub-lattice values.
fn document_needs_16bit(doc: &Document) -> bool {
    doc.pixel_format == aurelia_core::layer::PixelFormat::Srgb16
        || doc
            .layers
            .iter()
            .any(|layer| layer.format() == aurelia_core::layer::PixelFormat::Srgb16)
}

// ── Atomic output ────────────────────────────────────────────────────────────────────────────

/// Sibling path the encoder writes to before the result is moved into place.
fn part_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".to_string());
    path.with_file_name(format!("{name}.part"))
}

/// Moves a finished `.part` file onto its destination, replacing any previous export.
fn commit_part(part: &Path, path: &Path) -> Result<(), ExportError> {
    if path.exists() {
        // Windows refuses to rename over an existing file.
        std::fs::remove_file(path)?;
    }
    std::fs::rename(part, path)?;
    Ok(())
}

/// Runs `encode` against a `.part` path and only publishes the file if it succeeds.
fn write_atomically<F>(path: &Path, encode: F) -> Result<(), ExportError>
where
    F: FnOnce(&Path) -> Result<(), ExportError>,
{
    let part = part_path(path);
    match encode(&part) {
        Ok(()) => commit_part(&part, path),
        Err(error) => {
            let _ = std::fs::remove_file(&part);
            Err(error)
        }
    }
}

// ── Single image ─────────────────────────────────────────────────────────────────────────────

pub fn export_flat_image(
    doc: &Document,
    path: impl AsRef<Path>,
    format: ExportFormat,
    include_background: bool,
) -> Result<(), ExportError> {
    export_flat_image_with_quality(doc, path, format, include_background, None)
}

/// Same as [`export_flat_image`], but lets the caller pick JPEG quality (1..=100).
///
/// JPEG has no alpha channel, so the composite is flattened onto white before
/// encoding (writing RGBA straight into the JPEG encoder is unsupported).
pub fn export_flat_image_with_quality(
    doc: &Document,
    path: impl AsRef<Path>,
    format: ExportFormat,
    include_background: bool,
    jpeg_quality: Option<u8>,
) -> Result<(), ExportError> {
    export_flat_image_with_limits(
        doc,
        path,
        format,
        include_background,
        jpeg_quality,
        &ExportLimits::default(),
    )
}

/// [`export_flat_image_with_quality`] with explicit resource bounds.
pub fn export_flat_image_with_limits(
    doc: &Document,
    path: impl AsRef<Path>,
    format: ExportFormat,
    include_background: bool,
    jpeg_quality: Option<u8>,
    limits: &ExportLimits,
) -> Result<(), ExportError> {
    // Validate before compositing: a rejected size must not cost a canvas-sized allocation.
    let wide_png = format == ExportFormat::Png && document_needs_16bit(doc);
    let expected = if wide_png {
        frame_bytes_16(doc.width, doc.height, limits)?
    } else {
        frame_bytes(doc.width, doc.height, limits)?
    };
    check_budget(1, expected, limits)?;

    if wide_png {
        let pixels = doc.composite_layers_srgb16(include_background);
        let pixel_count = (doc.width as usize)
            .checked_mul(doc.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(ExportError::UnsupportedFormat)?;
        check_frame_len_16(&pixels, 0, pixel_count)?;
        return write_rgba16_to_path(
            &pixels,
            doc.width,
            doc.height,
            doc.icc_profile(),
            path.as_ref(),
        );
    }

    let pixels = doc.composite_layers(include_background);
    check_frame_len(&pixels, 0, expected)?;
    write_rgba_to_path(
        &pixels,
        doc.width,
        doc.height,
        0,
        path.as_ref(),
        format,
        jpeg_quality,
        doc.icc_profile(),
    )
}

/// Writes one RGBA16 buffer as a 16-bit PNG with the document's ICC profile when present.
fn write_rgba16_to_path(
    pixels: &[u16],
    width: u32,
    height: u32,
    icc_profile: Option<&[u8]>,
    path: &Path,
) -> Result<(), ExportError> {
    write_atomically(path, |target| {
        let file = File::create(target)?;
        let mut encoder = PngEncoder::new(file);
        if let Some(icc) = icc_profile {
            let _ = encoder.set_icc_profile(icc.to_vec());
        }
        let bytes: Vec<u8> = pixels
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect();
        encoder.write_image(&bytes, width, height, ExtendedColorType::Rgba16)?;
        Ok(())
    })
}

/// Writes one RGBA8 buffer to `path`. Geometry must already have been validated.
#[allow(clippy::too_many_arguments)]
fn write_rgba_to_path(
    pixels: &[u8],
    width: u32,
    height: u32,
    frame_index: usize,
    path: &Path,
    format: ExportFormat,
    jpeg_quality: Option<u8>,
    icc_profile: Option<&[u8]>,
) -> Result<(), ExportError> {
    let expected = (width as u64)
        .checked_mul(height as u64)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ExportError::UnsupportedFormat)?;
    let expected = usize::try_from(expected).map_err(|_| ExportError::UnsupportedFormat)?;
    check_frame_len(pixels, frame_index, expected)?;

    write_atomically(path, |target| {
        match format {
            ExportFormat::Jpeg => {
                let mut rgb = Vec::with_capacity(expected / 4 * 3);
                for chunk in pixels.chunks_exact(4) {
                    let alpha = chunk[3] as u32;
                    for &value in &chunk[..3] {
                        // Alpha-composite over opaque white.
                        let blended = (value as u32 * alpha + 255 * (255 - alpha)) / 255;
                        rgb.push(blended as u8);
                    }
                }
                let quality = jpeg_quality.unwrap_or(90).clamp(1, 100);
                let file = File::create(target)?;
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
                if let Some(icc) = icc_profile {
                    let _ = encoder.set_icc_profile(icc.to_vec());
                }
                encoder.encode(&rgb, width, height, image::ExtendedColorType::Rgb8)?;
            }
            ExportFormat::Png => {
                let file = File::create(target)?;
                let mut encoder = PngEncoder::new(file);
                if let Some(icc) = icc_profile {
                    let _ = encoder.set_icc_profile(icc.to_vec());
                }
                encoder.write_image(pixels, width, height, ExtendedColorType::Rgba8)?;
            }
            ExportFormat::WebP => {
                let image: ImageBuffer<Rgba<u8>, &[u8]> =
                    ImageBuffer::from_raw(width, height, pixels)
                        .ok_or(ExportError::UnsupportedFormat)?;
                let mut encoded = Vec::new();
                image.write_to(&mut std::io::Cursor::new(&mut encoded), ImageFormat::WebP)?;
                if let Some(icc) = icc_profile {
                    let has_alpha = pixels.chunks_exact(4).any(|pixel| pixel[3] != 255);
                    encoded = inject_webp_icc(&encoded, icc, width, height, has_alpha)?;
                }
                std::fs::write(target, encoded)?;
            }
        }
        Ok(())
    })
}

/// Adds an `ICCP` chunk to a WebP file, upgrading a simple still image to the extended (`VP8X`)
/// container. The image bitstream itself is untouched.
fn inject_webp_icc(
    raw: &[u8],
    icc: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
) -> Result<Vec<u8>, ExportError> {
    if raw.len() < 12 || &raw[..4] != b"RIFF" || &raw[8..12] != b"WEBP" {
        return Err(ExportError::UnsupportedFormat);
    }
    let mut chunks: Vec<&[u8]> = Vec::new();
    let mut at = 12usize;
    while at < raw.len() {
        if at + 8 > raw.len() {
            break;
        }
        let size =
            u32::from_le_bytes([raw[at + 4], raw[at + 5], raw[at + 6], raw[at + 7]]) as usize;
        let end = at
            .checked_add(8)
            .and_then(|value| value.checked_add(size))
            .ok_or(ExportError::UnsupportedFormat)?;
        if end > raw.len() {
            return Err(ExportError::UnsupportedFormat);
        }
        let padded_end = (end + (size & 1)).min(raw.len());
        chunks.push(&raw[at..padded_end]);
        at = padded_end;
    }
    if chunks.is_empty() {
        return Err(ExportError::UnsupportedFormat);
    }

    let mut vp8x_payload: Option<Vec<u8>> = None;
    let mut rest: Vec<&[u8]> = Vec::new();
    for chunk in chunks {
        let fourcc = &chunk[..4];
        if fourcc == b"VP8X" {
            let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as usize;
            if size != 10 || chunk.len() < 18 {
                return Err(ExportError::UnsupportedFormat);
            }
            let mut payload = chunk[8..18].to_vec();
            payload[0] |= 0x20; // ICC profile present
            if has_alpha {
                payload[0] |= 0x10;
            }
            vp8x_payload = Some(payload);
        } else if fourcc != b"ICCP" {
            rest.push(chunk);
        }
    }

    let payload = match vp8x_payload {
        Some(payload) => payload,
        None => {
            if width == 0 || height == 0 || width > 16_777_216 || height > 16_777_216 {
                return Err(ExportError::UnsupportedFormat);
            }
            let mut payload = Vec::with_capacity(10);
            payload.push(0x20 | if has_alpha { 0x10 } else { 0 });
            payload.extend_from_slice(&[0, 0, 0]); // reserved
            let w = (width - 1).to_le_bytes();
            let h = (height - 1).to_le_bytes();
            payload.extend_from_slice(&w[..3]);
            payload.extend_from_slice(&h[..3]);
            payload
        }
    };

    let mut out = Vec::with_capacity(raw.len() + icc.len() + 32);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(b"WEBP");
    push_webp_chunk(&mut out, b"VP8X", &payload)?;
    push_webp_chunk(&mut out, b"ICCP", icc)?;
    for chunk in rest {
        out.extend_from_slice(chunk);
    }
    let riff_size = u32::try_from(out.len() - 8).map_err(|_| ExportError::UnsupportedFormat)?;
    out[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Ok(out)
}

fn push_webp_chunk(out: &mut Vec<u8>, fourcc: &[u8; 4], payload: &[u8]) -> Result<(), ExportError> {
    let size = u32::try_from(payload.len()).map_err(|_| ExportError::UnsupportedFormat)?;
    out.extend_from_slice(fourcc);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() & 1 == 1 {
        out.push(0);
    }
    Ok(())
}

// ── Animated GIF ─────────────────────────────────────────────────────────────────────────────

/// Encodes frames from an owned iterator as an animated GIF, holding one frame at a time.
///
/// `limits.max_frames` and `limits.max_total_bytes` are enforced as frames arrive, so an
/// oversized job fails with a structured error instead of growing without bound.
pub fn export_animated_gif_stream<I>(
    frames: I,
    width: u32,
    height: u32,
    fps: u32,
    repeat: bool,
    path: impl AsRef<Path>,
    limits: &ExportLimits,
) -> Result<(), ExportError>
where
    I: IntoIterator<Item = Vec<u8>>,
{
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, RgbaImage};

    let expected = frame_bytes(width, height, limits)?;
    let path = path.as_ref();
    let delay_ms = (1000.0 / fps.max(1) as f32).round() as u32;
    let delay = Delay::from_numer_denom_ms(delay_ms, 1);

    write_atomically(path, |target| {
        let file = File::create(target)?;
        let mut encoder = GifEncoder::new(file);
        encoder.set_repeat(if repeat {
            Repeat::Infinite
        } else {
            Repeat::Finite(0)
        })?;

        let mut count = 0usize;
        for (index, data) in frames.into_iter().enumerate() {
            if index >= limits.max_frames {
                return Err(ExportError::TooManyFrames {
                    frames: index + 1,
                    max: limits.max_frames,
                });
            }
            let required = (expected as u64).saturating_mul(index as u64 + 1);
            if required > limits.max_total_bytes {
                return Err(ExportError::AggregateTooLarge {
                    required,
                    max: limits.max_total_bytes,
                });
            }
            check_frame_len(&data, index, expected)?;
            let img =
                RgbaImage::from_raw(width, height, data).ok_or(ExportError::UnsupportedFormat)?;
            encoder.encode_frame(Frame::from_parts(img, 0, 0, delay))?;
            count += 1;
        }

        if count == 0 {
            return Err(ExportError::UnsupportedFormat);
        }
        Ok(())
    })
}

/// [`export_animated_gif_stream`] over borrowed frames, with default limits.
///
/// Each frame is cloned because the encoder takes ownership; prefer the stream variant when
/// the caller can hand frames over.
pub fn export_animated_gif(
    frames_rgba: &[Vec<u8>],
    width: u32,
    height: u32,
    fps: u32,
    repeat: bool,
    path: impl AsRef<Path>,
) -> Result<(), ExportError> {
    export_animated_gif_with_limits(
        frames_rgba,
        width,
        height,
        fps,
        repeat,
        path,
        &ExportLimits::default(),
    )
}

/// [`export_animated_gif`] with explicit resource bounds.
pub fn export_animated_gif_with_limits(
    frames_rgba: &[Vec<u8>],
    width: u32,
    height: u32,
    fps: u32,
    repeat: bool,
    path: impl AsRef<Path>,
    limits: &ExportLimits,
) -> Result<(), ExportError> {
    export_animated_gif_stream(
        frames_rgba.iter().cloned(),
        width,
        height,
        fps,
        repeat,
        path,
        limits,
    )
}

// ── Frame sequence ───────────────────────────────────────────────────────────────────────────

/// Writes each frame as its own image file, holding one frame at a time.
pub fn export_frame_sequence_stream<I>(
    frames: I,
    width: u32,
    height: u32,
    dir_path: impl AsRef<Path>,
    prefix: &str,
    format: ExportFormat,
    limits: &ExportLimits,
) -> Result<Vec<PathBuf>, ExportError>
where
    I: IntoIterator<Item = Vec<u8>>,
{
    let expected = frame_bytes(width, height, limits)?;
    let dir = dir_path.as_ref();
    std::fs::create_dir_all(dir)?;

    let mut saved_paths = Vec::new();
    for (index, frame_data) in frames.into_iter().enumerate() {
        if index >= limits.max_frames {
            return Err(ExportError::TooManyFrames {
                frames: index + 1,
                max: limits.max_frames,
            });
        }
        let required = (expected as u64).saturating_mul(index as u64 + 1);
        if required > limits.max_total_bytes {
            return Err(ExportError::AggregateTooLarge {
                required,
                max: limits.max_total_bytes,
            });
        }
        check_frame_len(&frame_data, index, expected)?;

        let filename = format!("{}_{:04}.{}", prefix, index + 1, format.extension());
        let full_path = dir.join(filename);
        write_rgba_to_path(
            &frame_data,
            width,
            height,
            index,
            &full_path,
            format,
            None,
            None,
        )?;
        saved_paths.push(full_path);
    }

    if saved_paths.is_empty() {
        return Err(ExportError::UnsupportedFormat);
    }
    Ok(saved_paths)
}

pub fn export_frame_sequence(
    frames_rgba: &[Vec<u8>],
    width: u32,
    height: u32,
    dir_path: impl AsRef<Path>,
    prefix: &str,
    format: ExportFormat,
) -> Result<Vec<PathBuf>, ExportError> {
    export_frame_sequence_stream(
        frames_rgba.iter().cloned(),
        width,
        height,
        dir_path,
        prefix,
        format,
        &ExportLimits::default(),
    )
}

// ── Sprite sheet ────────────────────────────────────────────────────────────────────────────

/// Packs every frame into a single grid (sprite sheet) PNG and returns its dimensions.
///
/// Frames are laid out left-to-right, top-to-bottom with `columns` cells per row; unused
/// trailing cells stay fully transparent. The sheet is sized from the iterator's length up
/// front, then each frame is copied in as it arrives — so one frame is live at a time, not the
/// whole animation.
pub fn export_sprite_sheet_stream<I>(
    frames: I,
    width: u32,
    height: u32,
    columns: u32,
    path: impl AsRef<Path>,
    limits: &ExportLimits,
) -> Result<(u32, u32), ExportError>
where
    I: ExactSizeIterator<Item = Vec<u8>>,
{
    let expected = frame_bytes(width, height, limits)?;
    let count = frames.len();
    if count == 0 {
        return Err(ExportError::UnsupportedFormat);
    }
    check_budget(count, expected, limits)?;

    let cols = columns.max(1).min(count as u32);
    let rows = (count as u64).div_ceil(cols as u64);

    let sheet_w = (width as u64) * (cols as u64);
    let sheet_h = (height as u64) * rows;
    if sheet_w > u32::MAX as u64 || sheet_h > u32::MAX as u64 {
        return Err(ExportError::SheetTooLarge {
            width: sheet_w,
            height: sheet_h,
        });
    }
    let sheet_bytes = sheet_w
        .checked_mul(sheet_h)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ExportError::SheetTooLarge {
            width: sheet_w,
            height: sheet_h,
        })?;
    if sheet_bytes > limits.max_total_bytes {
        return Err(ExportError::AggregateTooLarge {
            required: sheet_bytes,
            max: limits.max_total_bytes,
        });
    }
    let sheet_bytes = usize::try_from(sheet_bytes).map_err(|_| ExportError::SheetTooLarge {
        width: sheet_w,
        height: sheet_h,
    })?;

    let (sheet_w, sheet_h) = (sheet_w as u32, sheet_h as u32);
    let mut sheet = vec![0u8; sheet_bytes];
    let row_bytes = (width as usize) * 4;

    for (index, frame) in frames.into_iter().enumerate() {
        check_frame_len(&frame, index, expected)?;
        let cx = (index as u32 % cols) * width;
        let cy = (index as u32 / cols) * height;
        for y in 0..height as usize {
            let src_start = y * row_bytes;
            let dst_start = ((cy as usize + y) * sheet_w as usize + cx as usize) * 4;
            sheet[dst_start..dst_start + row_bytes]
                .copy_from_slice(&frame[src_start..src_start + row_bytes]);
        }
    }

    let path = path.as_ref();
    write_atomically(path, |target| {
        let file = File::create(target)?;
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_raw(sheet_w, sheet_h, sheet).ok_or(ExportError::UnsupportedFormat)?;
        img.write_to(&mut std::io::BufWriter::new(file), ImageFormat::Png)?;
        Ok(())
    })?;

    Ok((sheet_w, sheet_h))
}

/// [`export_sprite_sheet_stream`] over borrowed frames, with default limits.
pub fn export_sprite_sheet(
    frames_rgba: &[Vec<u8>],
    width: u32,
    height: u32,
    columns: u32,
    path: impl AsRef<Path>,
) -> Result<(u32, u32), ExportError> {
    export_sprite_sheet_with_limits(
        frames_rgba,
        width,
        height,
        columns,
        path,
        &ExportLimits::default(),
    )
}

/// [`export_sprite_sheet`] with explicit resource bounds.
pub fn export_sprite_sheet_with_limits(
    frames_rgba: &[Vec<u8>],
    width: u32,
    height: u32,
    columns: u32,
    path: impl AsRef<Path>,
    limits: &ExportLimits,
) -> Result<(u32, u32), ExportError> {
    export_sprite_sheet_stream(
        frames_rgba.iter().cloned(),
        width,
        height,
        columns,
        path,
        limits,
    )
}
