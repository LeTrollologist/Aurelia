use aurelia_core::blend::BlendMode;
use aurelia_core::color::ThemeMode;
use aurelia_core::document::{Document, EditorState, GuideLine, StraightRulerState};
use aurelia_core::layer::Layer;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use image::{ImageBuffer, ImageFormat, Rgba};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const AUR_MAGIC_V1: &[u8; 4] = b"AUR\x01";
/// v2 appends a per-layer mask section (flag byte, enabled byte, width*height bytes).
const AUR_MAGIC_V2: &[u8; 4] = b"AUR\x02";
const AUR_MAGIC_V3: &[u8; 4] = b"AUR\x03";

/// Document and per-layer format tags in AUR v3.
///
/// `INHERIT` is what an ordinary document writes for every layer: the layer uses the
/// document's format. The explicit values exist so a document *can* express a layer of a
/// different representation - the editor does not create such documents yet, but the format
/// round-trips them, which is what keeps mixed formats an additive change rather than another
/// format version.
const FORMAT_SRGB8: u8 = 0;
const FORMAT_INHERIT: u8 = 0;
const LAYER_FORMAT_SRGB8: u8 = 1;
const LAYER_FORMAT_SRGB16: u8 = 2;
const HCV_MAGIC_V2: &[u8; 4] = b"HCV\x02";
const HCV_MAGIC_V3: &[u8; 4] = b"HCV\x03";
const HCV_MAGIC_V4: &[u8; 4] = b"HCV\x04";
const HCV_MAGIC_V5: &[u8; 4] = b"HCV\x05";
/// Optional editor-state section appended after the preview. Older readers stop at the
/// preview, so this is an additive trailing section rather than another container version.
const EDITOR_MAGIC: &[u8; 4] = b"AED1";
const EDITOR_MAX_BYTES: u32 = 1024 * 1024;
/// Text content is an in-memory string, not pixel data. Bound the file's claim before
/// allocating; 8 MiB is far beyond any practical text layer and rejects a malicious 4 GiB
/// length before the allocation happens.
const MAX_TEXT_CONTENT_BYTES: u32 = 8 * 1024 * 1024;

// Aggregate decode limits, applied across the whole document rather than per layer. A
// single deflate stream is already bounded to its layer's expected size, but a project can
// contain many layers; without a total budget a small compressed file can request tens of
// gigabytes. These limits are deliberately generous enough for large real documents while
// keeping a corrupt project from driving the process into an abort.
const MAX_PROJECT_LAYERS: u32 = 4096;
const MAX_PROJECT_PIXELS: u64 = 512 * 1024 * 1024;
const MAX_PROJECT_DECODED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Tracks the decoded pixel, mask, text and metadata bytes retained by a project being loaded.
#[derive(Debug, Default)]
struct DecodeBudget {
    pixels: u64,
    bytes: u64,
}

impl DecodeBudget {
    fn new(layer_count: u32) -> Result<Self, ProjectError> {
        if layer_count > MAX_PROJECT_LAYERS {
            return Err(ProjectError::CorruptData);
        }
        Ok(Self::default())
    }

    /// Reserves space before the loader allocates it. Masks reserve bytes with `pixels = 0`.
    fn reserve(&mut self, pixels: u64, bytes: u64) -> Result<(), ProjectError> {
        self.pixels = self
            .pixels
            .checked_add(pixels)
            .ok_or(ProjectError::CorruptData)?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(ProjectError::CorruptData)?;
        if self.pixels > MAX_PROJECT_PIXELS || self.bytes > MAX_PROJECT_DECODED_BYTES {
            return Err(ProjectError::CorruptData);
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum ProjectError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image encode error: {0}")]
    Image(#[from] image::ImageError),
    #[error("Invalid .aur binary container signature")]
    InvalidMagic,
    #[error("Corrupt or unsupported .aur project data")]
    CorruptData,
    #[error("This project version cannot represent the document's storage format")]
    UnsupportedFormat,
    #[error("text layer content is {len} bytes; maximum is {max}")]
    TextTooLarge { len: usize, max: usize },
    #[error("{field} is {len} bytes; maximum is {max}")]
    TextFieldTooLong {
        field: &'static str,
        len: usize,
        max: usize,
    },
}

fn read_text_content<R: Read>(reader: &mut R) -> Result<String, ProjectError> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    let len = u32::from_le_bytes(buf);
    if len > MAX_TEXT_CONTENT_BYTES {
        return Err(ProjectError::TextTooLarge {
            len: len as usize,
            max: MAX_TEXT_CONTENT_BYTES as usize,
        });
    }
    let mut bytes = vec![0u8; len as usize];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes).unwrap_or_default())
}

pub fn save_project_to_writer<W: Write>(doc: &Document, writer: W) -> Result<(), ProjectError> {
    write_project_version(doc, writer, 3)
}

/// Writes a project in a specific format version.
///
/// Versions 1 and 2 exist so tests can produce *authentic* old files with the real writer
/// instead of transforming today's bytes into something that only looks old. The differences
/// are all here: v2 added the mask section, v3 added the format tags.
pub(crate) fn write_project_version<W: Write>(
    doc: &Document,
    mut writer: W,
    version: u8,
) -> Result<(), ProjectError> {
    match version {
        1 => writer.write_all(AUR_MAGIC_V1)?,
        2 => writer.write_all(AUR_MAGIC_V2)?,
        _ => writer.write_all(AUR_MAGIC_V3)?,
    }
    writer.write_all(&doc.width.to_le_bytes())?;
    writer.write_all(&doc.height.to_le_bytes())?;
    writer.write_all(&doc.active_layer_id.to_le_bytes())?;
    writer.write_all(&[doc.background_value])?;
    writer.write_all(&[if doc.is_transparent { 1 } else { 0 }])?;

    let theme_byte = match doc.theme {
        ThemeMode::Dawn => 0u8,
        ThemeMode::Dusk => 1u8,
        ThemeMode::Ember => 2u8,
    };
    writer.write_all(&[theme_byte])?;

    // AUR v3: the document's storage format, then one byte per layer that either inherits it
    // or names its own. A pure 8-bit document writes zeros throughout, which is exactly what
    // older readers of the reserved byte would expect.
    //
    // The tag records `doc.pixel_format` - what the editor is creating in - rather than being
    // derived from the layers. Deriving it lost the format as soon as the last 16-bit layer
    // was deleted, so a 16-bit document could silently save as 8-bit.
    let document_is_16bit = doc.pixel_format == aurelia_core::layer::PixelFormat::Srgb16;
    let any_16bit_layer = doc
        .layers
        .iter()
        .any(|layer| layer.format() == aurelia_core::layer::PixelFormat::Srgb16);
    if version >= 3 {
        writer.write_all(&[if document_is_16bit { 1u8 } else { FORMAT_SRGB8 }])?;
    } else if document_is_16bit || any_16bit_layer {
        // Refuse rather than silently flatten: an old format cannot express 16-bit channels.
        return Err(ProjectError::UnsupportedFormat);
    }

    let layer_count = doc.layers.len() as u32;
    writer.write_all(&layer_count.to_le_bytes())?;

    for layer in &doc.layers {
        writer.write_all(&layer.id.to_le_bytes())?;

        let name_bytes = layer.name.as_bytes();
        if name_bytes.len() > u16::MAX as usize {
            return Err(ProjectError::TextFieldTooLong {
                field: "layer name",
                len: name_bytes.len(),
                max: u16::MAX as usize,
            });
        }
        let name_len = name_bytes.len() as u16;
        writer.write_all(&name_len.to_le_bytes())?;
        writer.write_all(name_bytes)?;

        // Kind (0 = Raster, 1 = Group, 2 = Adjustment, 3 = Text)
        let kind_byte = match layer.kind {
            aurelia_core::layer::LayerKind::Raster => 0u8,
            aurelia_core::layer::LayerKind::Group => 1u8,
            aurelia_core::layer::LayerKind::Adjustment => 2u8,
            aurelia_core::layer::LayerKind::Text => 3u8,
        };
        writer.write_all(&[kind_byte])?;

        // If text layer, serialize text configuration
        if let Some(txt) = &layer.text {
            let content_bytes = txt.content.as_bytes();
            if content_bytes.len() > MAX_TEXT_CONTENT_BYTES as usize {
                return Err(ProjectError::TextTooLarge {
                    len: content_bytes.len(),
                    max: MAX_TEXT_CONTENT_BYTES as usize,
                });
            }
            writer.write_all(&(content_bytes.len() as u32).to_le_bytes())?;
            writer.write_all(content_bytes)?;

            let font_bytes = txt.font_family.as_bytes();
            if font_bytes.len() > u16::MAX as usize {
                return Err(ProjectError::TextFieldTooLong {
                    field: "text font family",
                    len: font_bytes.len(),
                    max: u16::MAX as usize,
                });
            }
            writer.write_all(&(font_bytes.len() as u16).to_le_bytes())?;
            writer.write_all(font_bytes)?;

            writer.write_all(&txt.font_size.to_le_bytes())?;
            writer.write_all(&txt.color)?;

            let align_byte = match txt.align {
                aurelia_core::brush::TextAlign::Left => 0u8,
                aurelia_core::brush::TextAlign::Center => 1u8,
                aurelia_core::brush::TextAlign::Right => 2u8,
            };
            writer.write_all(&[align_byte])?;
            writer.write_all(&txt.line_spacing.to_le_bytes())?;
            writer.write_all(&txt.letter_spacing.to_le_bytes())?;
            writer.write_all(&txt.box_w.to_le_bytes())?;
            writer.write_all(&txt.box_h.to_le_bytes())?;
            writer.write_all(&txt.pos_x.to_le_bytes())?;
            writer.write_all(&txt.pos_y.to_le_bytes())?;
        }

        // If adjustment, serialize adjustment parameters
        if let Some(adj) = &layer.adjustment {
            match &adj.adjustment_type {
                aurelia_core::layer::AdjustmentType::BrightnessContrast {
                    brightness,
                    contrast,
                } => {
                    writer.write_all(&[0u8])?;
                    writer.write_all(&brightness.to_le_bytes())?;
                    writer.write_all(&contrast.to_le_bytes())?;
                }
                aurelia_core::layer::AdjustmentType::Hsl {
                    hue_shift,
                    saturation,
                    lightness,
                } => {
                    writer.write_all(&[1u8])?;
                    writer.write_all(&hue_shift.to_le_bytes())?;
                    writer.write_all(&saturation.to_le_bytes())?;
                    writer.write_all(&lightness.to_le_bytes())?;
                }
                aurelia_core::layer::AdjustmentType::ColorBalance {
                    cyan_red,
                    magenta_green,
                    yellow_blue,
                } => {
                    writer.write_all(&[2u8])?;
                    writer.write_all(&cyan_red.to_le_bytes())?;
                    writer.write_all(&magenta_green.to_le_bytes())?;
                    writer.write_all(&yellow_blue.to_le_bytes())?;
                }
                aurelia_core::layer::AdjustmentType::Invert => {
                    writer.write_all(&[3u8])?;
                }
                aurelia_core::layer::AdjustmentType::Posterize { levels } => {
                    writer.write_all(&[4u8])?;
                    writer.write_all(&levels.to_le_bytes())?;
                }
                aurelia_core::layer::AdjustmentType::Threshold { cutoff } => {
                    writer.write_all(&[5u8])?;
                    writer.write_all(&[*cutoff])?;
                }
                aurelia_core::layer::AdjustmentType::Sepia { strength } => {
                    writer.write_all(&[6u8])?;
                    writer.write_all(&strength.to_le_bytes())?;
                }
            }
        }

        // Parent ID (0 if None)
        let pid = layer.parent_id.unwrap_or(0);
        writer.write_all(&pid.to_le_bytes())?;

        // Flags bitmask
        let mut flags = 0u8;
        if layer.visible {
            flags |= 1 << 0;
        }
        if layer.locked {
            flags |= 1 << 1;
        }
        if layer.alpha_locked {
            flags |= 1 << 2;
        }
        if layer.clipping_mask {
            flags |= 1 << 3;
        }
        if layer.is_reference {
            flags |= 1 << 4;
        }
        if layer.is_expanded {
            flags |= 1 << 5;
        }
        if layer.pass_through {
            flags |= 1 << 6;
        }
        writer.write_all(&[flags])?;

        writer.write_all(&layer.opacity.to_le_bytes())?;

        let blend_byte = match layer.blend_mode {
            BlendMode::Normal => 0u8,
            BlendMode::Multiply => 1u8,
            BlendMode::Screen => 2u8,
            BlendMode::Overlay => 3u8,
            BlendMode::Darken => 4u8,
            BlendMode::Lighten => 5u8,
            BlendMode::ColorDodge => 6u8,
            BlendMode::ColorBurn => 7u8,
            BlendMode::HardLight => 8u8,
            BlendMode::SoftLight => 9u8,
            BlendMode::Difference => 10u8,
            BlendMode::Exclusion => 11u8,
            BlendMode::Luminosity => 12u8,
            BlendMode::Adaptive => 13u8,
        };
        writer.write_all(&[blend_byte])?;
        writer.write_all(&layer.offset_x.to_le_bytes())?;
        writer.write_all(&layer.offset_y.to_le_bytes())?;

        // Per-layer format: inherit the document's, or name this layer's own.
        let layer_is_16bit = layer.format() == aurelia_core::layer::PixelFormat::Srgb16;
        let layer_tag = if version < 3 {
            FORMAT_INHERIT
        } else if layer.is_group() || layer.is_adjustment() {
            FORMAT_INHERIT
        } else if layer_is_16bit == document_is_16bit {
            FORMAT_INHERIT
        } else if layer_is_16bit {
            LAYER_FORMAT_SRGB16
        } else {
            LAYER_FORMAT_SRGB8
        };
        if version >= 3 {
            writer.write_all(&[layer_tag])?;
        } else if layer_tag != FORMAT_INHERIT {
            return Err(ProjectError::UnsupportedFormat);
        }

        if layer.is_group() || layer.is_adjustment() {
            // Groups and Adjustments have no pixel buffer
            let comp_len = 0u32;
            writer.write_all(&comp_len.to_le_bytes())?;
        } else {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
            match layer.buffer() {
                aurelia_core::layer::LayerBuffer::Srgb8(bytes) => encoder.write_all(bytes)?,
                aurelia_core::layer::LayerBuffer::Srgb16(channels) => {
                    // Little-endian per channel, which is the same byte order the loader and
                    // the history bridge use.
                    for channel in channels {
                        encoder.write_all(&channel.to_le_bytes())?;
                    }
                }
            }
            let compressed = encoder.finish()?;

            let comp_len = compressed.len() as u32;
            writer.write_all(&comp_len.to_le_bytes())?;
            writer.write_all(&compressed)?;
        }

        // Layer mask (AUR v2+): 0 = none, 1 = enabled flag + raw mask bytes.
        if version >= 2 {
            match &layer.mask {
                Some(mask)
                    if mask.pixels.len() == (doc.width as usize) * (doc.height as usize)
                        && mask.width == doc.width
                        && mask.height == doc.height =>
                {
                    writer.write_all(&[1u8])?;
                    writer.write_all(&[u8::from(mask.enabled)])?;
                    writer.write_all(&mask.pixels)?;
                }
                _ => writer.write_all(&[0u8])?,
            }
        }
    }

    // Save flat preview thumbnail
    let thumb_flat = doc.composite_layers(true);
    let thumb_img: ImageBuffer<Rgba<u8>, &[u8]> =
        ImageBuffer::from_raw(doc.width, doc.height, &thumb_flat[..])
            .ok_or(ProjectError::CorruptData)?;
    let mut thumb_bytes = Cursor::new(Vec::new());
    thumb_img.write_to(&mut thumb_bytes, ImageFormat::Png)?;
    let thumb_ref = thumb_bytes.get_ref();
    let thumb_len = thumb_ref.len() as u32;
    writer.write_all(&thumb_len.to_le_bytes())?;
    writer.write_all(thumb_ref)?;

    write_editor_state(&doc.editor, &mut writer)?;
    write_layer_styles(doc, &mut writer)?;
    write_icc_profile(doc, &mut writer)?;
    Ok(())
}

/// Optional trailing section for per-layer styles (v3.8.0).
///
/// It is a separate, length-prefixed section after the editor block rather than a new field in
/// the layer record, so the v3 layer format does not change and older files (which simply end
/// earlier) keep loading. Unknown trailing sections are ignored by older readers.
const STYLE_MAGIC: &[u8; 4] = b"ASTY";
const STYLE_MAX_BYTES: u32 = 1024 * 1024;

fn write_layer_styles<W: Write>(doc: &Document, writer: &mut W) -> Result<(), ProjectError> {
    let styled: Vec<&aurelia_core::layer::Layer> = doc
        .layers
        .iter()
        .filter(|layer| !layer.styles.is_empty())
        .collect();
    if styled.is_empty() {
        return Ok(());
    }
    let mut payload = Vec::new();
    payload.push(1u8); // styles payload version
    payload.extend_from_slice(&(styled.len() as u32).to_le_bytes());
    for layer in styled {
        payload.extend_from_slice(&layer.id.to_le_bytes());
        let styles = &layer.styles;
        let mut flags = 0u8;
        if styles.stroke.is_some() {
            flags |= 1;
        }
        if styles.drop_shadow.is_some() {
            flags |= 2;
        }
        if styles.outer_glow.is_some() {
            flags |= 4;
        }
        payload.push(flags);
        if let Some(stroke) = &styles.stroke {
            payload.extend_from_slice(&stroke.width.to_le_bytes());
            payload.extend_from_slice(&stroke.opacity.to_le_bytes());
            payload.extend_from_slice(&stroke.color.to_rgba8());
        }
        if let Some(shadow) = &styles.drop_shadow {
            payload.extend_from_slice(&shadow.offset_x.to_le_bytes());
            payload.extend_from_slice(&shadow.offset_y.to_le_bytes());
            payload.extend_from_slice(&shadow.blur.to_le_bytes());
            payload.extend_from_slice(&shadow.opacity.to_le_bytes());
            payload.extend_from_slice(&shadow.color.to_rgba8());
        }
        if let Some(glow) = &styles.outer_glow {
            payload.extend_from_slice(&glow.radius.to_le_bytes());
            payload.extend_from_slice(&glow.opacity.to_le_bytes());
            payload.extend_from_slice(&glow.color.to_rgba8());
        }
    }
    if payload.len() > STYLE_MAX_BYTES as usize {
        // A style stack this large is a bug, not user data; refuse the save rather than write
        // a section no reader will accept.
        return Err(ProjectError::CorruptData);
    }
    writer.write_all(STYLE_MAGIC)?;
    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(&payload)?;
    Ok(())
}

/// Reads the optional trailing sections as a length-prefixed sequence.
///
/// Returns `(styles, icc_profile)`. Unknown sections are skipped; a malformed/truncated section
/// stops parsing and leaves the already-decoded data in place. These sections are optional
/// metadata, never required for the document itself.
type LayerStyleEntries = Vec<(u64, aurelia_core::layer_style::LayerStyles)>;

fn read_trailing_sections<R: Read>(reader: &mut R) -> (Option<LayerStyleEntries>, Option<Vec<u8>>) {
    const TRAILING_CAP: u64 = 8 * 1024 * 1024;
    let mut rest = Vec::new();
    if reader
        .by_ref()
        .take(TRAILING_CAP)
        .read_to_end(&mut rest)
        .is_err()
    {
        return (None, None);
    }

    let mut styles = None;
    let mut icc_profile = None;
    let mut at = 0usize;
    while at + 8 <= rest.len() {
        let magic = &rest[at..at + 4];
        let length =
            u32::from_le_bytes([rest[at + 4], rest[at + 5], rest[at + 6], rest[at + 7]]) as usize;
        let payload_start = at + 8;
        let Some(payload_end) = payload_start.checked_add(length) else {
            break;
        };
        if payload_end > rest.len() {
            break;
        }
        let payload = &rest[payload_start..payload_end];
        if magic == STYLE_MAGIC && length <= STYLE_MAX_BYTES as usize {
            if let Some(parsed) = parse_layer_styles(payload) {
                styles = Some(parsed);
            }
        } else if magic == ICC_MAGIC
            && length <= ICC_MAX_BYTES as usize
            && aurelia_core::IccProfile::from_icc(payload.to_vec()).is_ok()
        {
            icc_profile = Some(payload.to_vec());
        }
        at = payload_end;
    }
    (styles, icc_profile)
}

fn parse_layer_styles(
    payload: &[u8],
) -> Option<Vec<(u64, aurelia_core::layer_style::LayerStyles)>> {
    use aurelia_core::color::Color;
    use aurelia_core::layer_style::{DropShadowStyle, LayerStyles, OuterGlowStyle, StrokeStyle};

    struct Cursor<'a> {
        data: &'a [u8],
        at: usize,
    }
    impl<'a> Cursor<'a> {
        fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
            let slice = self.data.get(self.at..self.at + n)?;
            self.at += n;
            Some(slice)
        }
        fn u8(&mut self) -> Option<u8> {
            Some(*self.bytes(1)?.first()?)
        }
        fn u32(&mut self) -> Option<u32> {
            Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
        }
        fn u64(&mut self) -> Option<u64> {
            Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
        }
        fn f32(&mut self) -> Option<f32> {
            let value = f32::from_le_bytes(self.bytes(4)?.try_into().ok()?);
            value.is_finite().then_some(value)
        }
        fn color(&mut self) -> Option<Color> {
            let rgba = self.bytes(4)?;
            Some(Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]))
        }
    }

    let mut cursor = Cursor {
        data: payload,
        at: 0,
    };
    let version = cursor.u8()?;
    if version != 1 {
        return None;
    }
    let count = cursor.u32()? as usize;
    // The smallest entry is 9 bytes (id + flags); a count that cannot fit in the payload is a
    // lie. Reject it before reserving, so a 1 MiB section cannot ask for 100k records and make
    // the allocator do the work before the first EOF.
    if count > 100_000 || count > payload.len().saturating_sub(5) / 9 {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let id = cursor.u64()?;
        let flags = cursor.u8()?;
        let mut styles = LayerStyles::default();
        if flags & 1 != 0 {
            let width = cursor.f32()?.max(0.0);
            let opacity = cursor.f32()?.clamp(0.0, 1.0);
            let color = cursor.color()?;
            styles.stroke = Some(StrokeStyle {
                width,
                color,
                opacity,
            });
        }
        if flags & 2 != 0 {
            let offset_x = cursor.f32()?;
            let offset_y = cursor.f32()?;
            let blur = cursor.f32()?.max(0.0);
            let opacity = cursor.f32()?.clamp(0.0, 1.0);
            let color = cursor.color()?;
            styles.drop_shadow = Some(DropShadowStyle {
                offset_x,
                offset_y,
                blur,
                color,
                opacity,
            });
        }
        if flags & 4 != 0 {
            let radius = cursor.f32()?.max(0.0);
            let opacity = cursor.f32()?.clamp(0.0, 1.0);
            let color = cursor.color()?;
            styles.outer_glow = Some(OuterGlowStyle {
                radius,
                color,
                opacity,
            });
        }
        out.push((id, styles));
    }
    Some(out)
}

const ICC_MAGIC: &[u8; 4] = b"AICC";
const ICC_MAX_BYTES: u32 = 4 * 1024 * 1024;

/// Writes the optional colour-profile section after styles. Absent for untagged documents.
fn write_icc_profile<W: Write>(doc: &Document, writer: &mut W) -> Result<(), ProjectError> {
    let Some(bytes) = doc.icc_profile() else {
        return Ok(());
    };
    if bytes.is_empty() || bytes.len() > ICC_MAX_BYTES as usize {
        // An empty/oversized profile is a bug, not a reason to write a section no reader accepts.
        return Err(ProjectError::CorruptData);
    }
    writer.write_all(ICC_MAGIC)?;
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    Ok(())
}

fn write_editor_state<W: Write>(editor: &EditorState, writer: &mut W) -> Result<(), ProjectError> {
    let mut payload = Vec::new();
    payload.push(1u8); // editor-state payload version
    payload.extend_from_slice(&(editor.guides.len() as u32).to_le_bytes());
    for guide in &editor.guides {
        payload.push(u8::from(guide.vertical));
        payload.extend_from_slice(&guide.position.to_le_bytes());
    }
    payload.extend_from_slice(&editor.grid_origin[0].to_le_bytes());
    payload.extend_from_slice(&editor.grid_origin[1].to_le_bytes());
    match &editor.straight_ruler {
        Some(ruler) => {
            payload.push(1u8);
            payload.extend_from_slice(&ruler.origin[0].to_le_bytes());
            payload.extend_from_slice(&ruler.origin[1].to_le_bytes());
            payload.extend_from_slice(&ruler.angle_rad.to_le_bytes());
            payload.extend_from_slice(&ruler.length.to_le_bytes());
        }
        None => payload.push(0u8),
    }
    writer.write_all(EDITOR_MAGIC)?;
    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(&payload)?;
    Ok(())
}

/// Skips the preview PNG and reads the optional trailing editor section. Older files have
/// nothing after the preview, and unreadable/absent sections simply leave editor defaults.
fn read_editor_state<R: Read>(reader: &mut R) -> Option<EditorState> {
    let mut preview_len = [0u8; 4];
    if reader.read_exact(&mut preview_len).is_err() {
        return None;
    }
    let preview_len = u32::from_le_bytes(preview_len);
    if preview_len > 256 * 1024 * 1024 {
        return None;
    }
    if std::io::copy(
        &mut reader.by_ref().take(preview_len as u64),
        &mut std::io::sink(),
    )
    .ok()?
        != preview_len as u64
    {
        return None;
    }

    let mut magic = [0u8; 4];
    if reader.read_exact(&mut magic).is_err() || &magic != EDITOR_MAGIC {
        return None;
    }
    let mut length = [0u8; 4];
    reader.read_exact(&mut length).ok()?;
    let length = u32::from_le_bytes(length);
    if length > EDITOR_MAX_BYTES {
        return None;
    }
    let mut payload = vec![0u8; length as usize];
    reader.read_exact(&mut payload).ok()?;
    parse_editor_payload(&payload)
}

fn parse_editor_payload(payload: &[u8]) -> Option<EditorState> {
    let mut cursor = Cursor::new(payload);
    let mut byte = [0u8; 1];
    cursor.read_exact(&mut byte).ok()?;
    if byte[0] != 1 {
        return None;
    }
    let mut count = [0u8; 4];
    cursor.read_exact(&mut count).ok()?;
    let count = u32::from_le_bytes(count);
    // Each guide is 5 bytes; a larger count cannot fit in the payload.
    if count > 4096 || count as usize > payload.len().saturating_sub(14) / 5 {
        return None;
    }
    let mut editor = EditorState::default();
    for _ in 0..count {
        cursor.read_exact(&mut byte).ok()?;
        let vertical = byte[0] != 0;
        let mut position = [0u8; 4];
        cursor.read_exact(&mut position).ok()?;
        let position = f32::from_le_bytes(position);
        if !position.is_finite() {
            return None;
        }
        editor.guides.push(GuideLine { vertical, position });
    }
    let mut origin = [0u8; 8];
    cursor.read_exact(&mut origin).ok()?;
    editor.grid_origin = [
        f32::from_le_bytes(origin[0..4].try_into().ok()?),
        f32::from_le_bytes(origin[4..8].try_into().ok()?),
    ];
    if !editor.grid_origin.iter().all(|value| value.is_finite()) {
        return None;
    }
    cursor.read_exact(&mut byte).ok()?;
    if byte[0] != 0 {
        let mut numbers = [0u8; 16];
        cursor.read_exact(&mut numbers).ok()?;
        let ruler = StraightRulerState {
            origin: [
                f32::from_le_bytes(numbers[0..4].try_into().ok()?),
                f32::from_le_bytes(numbers[4..8].try_into().ok()?),
            ],
            angle_rad: f32::from_le_bytes(numbers[8..12].try_into().ok()?),
            length: f32::from_le_bytes(numbers[12..16].try_into().ok()?),
        };
        if !ruler.origin.iter().all(|value| value.is_finite())
            || !ruler.angle_rad.is_finite()
            || !ruler.length.is_finite()
        {
            return None;
        }
        editor.straight_ruler = Some(ruler);
    }
    Some(editor)
}

/// Writes a project through a unique sibling temporary file.
///
/// The temporary file is synchronized to durable storage before the atomic replacement, so a
/// successful rename cannot publish a partially written image of the document. A failed
/// write/sync/rename removes the temporary file and leaves the previous destination untouched.
/// Parent-directory synchronization is attempted after a successful rename where the platform
/// permits opening a directory as a file.
pub fn save_project_file(doc: &Document, path: impl AsRef<Path>) -> Result<(), ProjectError> {
    let path = path.as_ref();
    let tmp_path = unique_temp_path(path);

    let result = (|| -> Result<(), ProjectError> {
        let mut file = File::create(&tmp_path)?;
        save_project_to_writer(doc, &mut file)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp_path, path)?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            // Directory fsync is not available on every Windows filesystem; it is a
            // best-effort strengthening, not part of the file's own durability.
            if let Ok(directory) = File::open(parent) {
                let _ = directory.sync_all();
            }
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

static SAVE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A unique sibling path: process id, nanosecond timestamp and an in-process counter.
fn unique_temp_path(path: &Path) -> PathBuf {
    let base = path.file_name().unwrap_or_else(|| OsStr::new("project"));
    let sequence = SAVE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let mut name = OsString::from(".");
    name.push(base);
    name.push(format!(
        ".{}.{}.{}.tmp",
        std::process::id(),
        nanos,
        sequence
    ));
    path.with_file_name(name)
}

pub fn load_project_from_reader<R: Read>(mut reader: R) -> Result<Document, ProjectError> {
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    let is_aur_v3 = &magic == AUR_MAGIC_V3;
    let is_aur_v2 = &magic == AUR_MAGIC_V2;
    let is_aur = &magic == AUR_MAGIC_V1 || is_aur_v2 || is_aur_v3;
    // AUR v1 reuses the legacy HCV v5 body layout, so legacy files stay readable.
    let is_v5 = &magic == HCV_MAGIC_V5 || is_aur;
    let is_v4 = &magic == HCV_MAGIC_V4;
    let is_v3 = &magic == HCV_MAGIC_V3;
    let is_v2 = &magic == HCV_MAGIC_V2;
    if !is_aur && !is_v5 && !is_v4 && !is_v3 && !is_v2 {
        return Err(ProjectError::InvalidMagic);
    }

    let mut buf4 = [0u8; 4];
    let mut buf8 = [0u8; 8];
    let mut buf1 = [0u8; 1];

    reader.read_exact(&mut buf4)?;
    let width = u32::from_le_bytes(buf4);

    reader.read_exact(&mut buf4)?;
    let height = u32::from_le_bytes(buf4);

    // A corrupt header must not be allowed to drive an allocation. The cap is per layer and
    // generous (8 bytes per pixel covers 16-bit RGBA) but finite.
    let layer_bytes = (width as u64)
        .checked_mul(height as u64)
        .and_then(|pixels| pixels.checked_mul(8));
    match layer_bytes {
        Some(bytes) if width > 0 && height > 0 && bytes <= (1u64 << 31) => {}
        _ => return Err(ProjectError::CorruptData),
    }

    reader.read_exact(&mut buf8)?;
    let active_layer_id = u64::from_le_bytes(buf8);

    reader.read_exact(&mut buf1)?;
    let background_value = buf1[0];

    reader.read_exact(&mut buf1)?;
    let is_transparent = buf1[0] != 0;

    reader.read_exact(&mut buf1)?;
    let theme = match buf1[0] {
        1 => ThemeMode::Dusk,
        2 => ThemeMode::Ember,
        _ => ThemeMode::Dawn,
    };

    // AUR v3 stores the document's storage format; older versions were implicitly 8-bit.
    let document_format = if is_aur_v3 {
        reader.read_exact(&mut buf1)?;
        match buf1[0] {
            0 => aurelia_core::layer::PixelFormat::Srgb8,
            1 => aurelia_core::layer::PixelFormat::Srgb16,
            _ => return Err(ProjectError::CorruptData),
        }
    } else {
        aurelia_core::layer::PixelFormat::Srgb8
    };

    reader.read_exact(&mut buf4)?;
    let layer_count = u32::from_le_bytes(buf4);
    let mut budget = DecodeBudget::new(layer_count)?;

    // The count comes straight from the file. Reserving for all of it up front would let a
    // tiny header ask for terabytes before the first layer (and its EOF) is ever read.
    let mut layers = Vec::with_capacity((layer_count as usize).min(1024));
    let mut max_id = active_layer_id;
    let layer_pixels = (width as u64) * (height as u64);

    for _ in 0..layer_count {
        reader.read_exact(&mut buf8)?;
        let id = u64::from_le_bytes(buf8);
        if id > max_id {
            max_id = id;
        }

        let mut buf2 = [0u8; 2];
        reader.read_exact(&mut buf2)?;
        let name_len = u16::from_le_bytes(buf2) as usize;

        let mut name_vec = vec![0u8; name_len];
        reader.read_exact(&mut name_vec)?;
        budget.reserve(0, name_len as u64)?;
        let name = String::from_utf8(name_vec).unwrap_or_else(|_| format!("Layer {}", id));

        let mut adjustment = None;
        let mut text_config = None;

        let (
            kind,
            parent_id,
            visible,
            locked,
            alpha_locked,
            clipping_mask,
            is_reference,
            is_expanded,
            pass_through,
        ) = if is_v5 {
            reader.read_exact(&mut buf1)?;
            let kind = match buf1[0] {
                1 => aurelia_core::layer::LayerKind::Group,
                2 => {
                    reader.read_exact(&mut buf1)?;
                    let adj_type = match buf1[0] {
                        0 => {
                            reader.read_exact(&mut buf4)?;
                            let brightness = f32::from_le_bytes(buf4);
                            reader.read_exact(&mut buf4)?;
                            let contrast = f32::from_le_bytes(buf4);
                            aurelia_core::layer::AdjustmentType::BrightnessContrast {
                                brightness,
                                contrast,
                            }
                        }
                        1 => {
                            reader.read_exact(&mut buf4)?;
                            let hue_shift = f32::from_le_bytes(buf4);
                            reader.read_exact(&mut buf4)?;
                            let saturation = f32::from_le_bytes(buf4);
                            reader.read_exact(&mut buf4)?;
                            let lightness = f32::from_le_bytes(buf4);
                            aurelia_core::layer::AdjustmentType::Hsl {
                                hue_shift,
                                saturation,
                                lightness,
                            }
                        }
                        2 => {
                            reader.read_exact(&mut buf4)?;
                            let cyan_red = f32::from_le_bytes(buf4);
                            reader.read_exact(&mut buf4)?;
                            let magenta_green = f32::from_le_bytes(buf4);
                            reader.read_exact(&mut buf4)?;
                            let yellow_blue = f32::from_le_bytes(buf4);
                            aurelia_core::layer::AdjustmentType::ColorBalance {
                                cyan_red,
                                magenta_green,
                                yellow_blue,
                            }
                        }
                        3 => aurelia_core::layer::AdjustmentType::Invert,
                        4 => {
                            reader.read_exact(&mut buf4)?;
                            let levels = u32::from_le_bytes(buf4);
                            aurelia_core::layer::AdjustmentType::Posterize { levels }
                        }
                        5 => {
                            reader.read_exact(&mut buf1)?;
                            let cutoff = buf1[0];
                            aurelia_core::layer::AdjustmentType::Threshold { cutoff }
                        }
                        6 => {
                            reader.read_exact(&mut buf4)?;
                            let strength = f32::from_le_bytes(buf4);
                            aurelia_core::layer::AdjustmentType::Sepia { strength }
                        }
                        _ => aurelia_core::layer::AdjustmentType::Invert,
                    };
                    adjustment = Some(aurelia_core::layer::AdjustmentConfig {
                        adjustment_type: adj_type,
                    });
                    aurelia_core::layer::LayerKind::Adjustment
                }
                3 => {
                    let content = read_text_content(&mut reader)?;
                    budget.reserve(0, content.len() as u64)?;

                    reader.read_exact(&mut buf2)?;
                    let font_len = u16::from_le_bytes(buf2) as usize;
                    let mut font_vec = vec![0u8; font_len];
                    reader.read_exact(&mut font_vec)?;
                    budget.reserve(0, font_vec.len() as u64)?;
                    let font_family = String::from_utf8(font_vec).unwrap_or_default();

                    reader.read_exact(&mut buf4)?;
                    let font_size = f32::from_le_bytes(buf4);

                    let mut col_buf = [0u8; 4];
                    reader.read_exact(&mut col_buf)?;

                    reader.read_exact(&mut buf1)?;
                    let align = match buf1[0] {
                        1 => aurelia_core::brush::TextAlign::Center,
                        2 => aurelia_core::brush::TextAlign::Right,
                        _ => aurelia_core::brush::TextAlign::Left,
                    };

                    reader.read_exact(&mut buf4)?;
                    let line_spacing = f32::from_le_bytes(buf4);

                    reader.read_exact(&mut buf4)?;
                    let letter_spacing = f32::from_le_bytes(buf4);

                    reader.read_exact(&mut buf4)?;
                    let box_w = f32::from_le_bytes(buf4);

                    reader.read_exact(&mut buf4)?;
                    let box_h = f32::from_le_bytes(buf4);

                    reader.read_exact(&mut buf4)?;
                    let pos_x = f32::from_le_bytes(buf4);

                    reader.read_exact(&mut buf4)?;
                    let pos_y = f32::from_le_bytes(buf4);

                    text_config = Some(aurelia_core::layer::TextLayerConfig {
                        content,
                        font_family,
                        font_size,
                        color: col_buf,
                        align,
                        line_spacing,
                        letter_spacing,
                        box_w,
                        box_h,
                        pos_x,
                        pos_y,
                    });

                    aurelia_core::layer::LayerKind::Text
                }
                _ => aurelia_core::layer::LayerKind::Raster,
            };

            reader.read_exact(&mut buf8)?;
            let raw_pid = u64::from_le_bytes(buf8);
            let parent_id = if raw_pid > 0 { Some(raw_pid) } else { None };

            reader.read_exact(&mut buf1)?;
            let flags = buf1[0];
            let visible = (flags & (1 << 0)) != 0;
            let locked = (flags & (1 << 1)) != 0;
            let alpha_locked = (flags & (1 << 2)) != 0;
            let clipping_mask = (flags & (1 << 3)) != 0;
            let is_reference = (flags & (1 << 4)) != 0;
            let is_expanded = (flags & (1 << 5)) != 0;
            let pass_through = (flags & (1 << 6)) != 0;

            (
                kind,
                parent_id,
                visible,
                locked,
                alpha_locked,
                clipping_mask,
                is_reference,
                is_expanded,
                pass_through,
            )
        } else if is_v4 {
            reader.read_exact(&mut buf1)?;
            let kind = match buf1[0] {
                1 => aurelia_core::layer::LayerKind::Group,
                2 => {
                    reader.read_exact(&mut buf1)?;
                    let adj_type = match buf1[0] {
                        0 => {
                            reader.read_exact(&mut buf4)?;
                            let brightness = f32::from_le_bytes(buf4);
                            reader.read_exact(&mut buf4)?;
                            let contrast = f32::from_le_bytes(buf4);
                            aurelia_core::layer::AdjustmentType::BrightnessContrast {
                                brightness,
                                contrast,
                            }
                        }
                        1 => {
                            reader.read_exact(&mut buf4)?;
                            let hue_shift = f32::from_le_bytes(buf4);
                            reader.read_exact(&mut buf4)?;
                            let saturation = f32::from_le_bytes(buf4);
                            reader.read_exact(&mut buf4)?;
                            let lightness = f32::from_le_bytes(buf4);
                            aurelia_core::layer::AdjustmentType::Hsl {
                                hue_shift,
                                saturation,
                                lightness,
                            }
                        }
                        2 => {
                            reader.read_exact(&mut buf4)?;
                            let cyan_red = f32::from_le_bytes(buf4);
                            reader.read_exact(&mut buf4)?;
                            let magenta_green = f32::from_le_bytes(buf4);
                            reader.read_exact(&mut buf4)?;
                            let yellow_blue = f32::from_le_bytes(buf4);
                            aurelia_core::layer::AdjustmentType::ColorBalance {
                                cyan_red,
                                magenta_green,
                                yellow_blue,
                            }
                        }
                        3 => aurelia_core::layer::AdjustmentType::Invert,
                        4 => {
                            reader.read_exact(&mut buf4)?;
                            let levels = u32::from_le_bytes(buf4);
                            aurelia_core::layer::AdjustmentType::Posterize { levels }
                        }
                        5 => {
                            reader.read_exact(&mut buf1)?;
                            let cutoff = buf1[0];
                            aurelia_core::layer::AdjustmentType::Threshold { cutoff }
                        }
                        6 => {
                            reader.read_exact(&mut buf4)?;
                            let strength = f32::from_le_bytes(buf4);
                            aurelia_core::layer::AdjustmentType::Sepia { strength }
                        }
                        _ => aurelia_core::layer::AdjustmentType::Invert,
                    };
                    adjustment = Some(aurelia_core::layer::AdjustmentConfig {
                        adjustment_type: adj_type,
                    });
                    aurelia_core::layer::LayerKind::Adjustment
                }
                _ => aurelia_core::layer::LayerKind::Raster,
            };

            reader.read_exact(&mut buf8)?;
            let raw_pid = u64::from_le_bytes(buf8);
            let parent_id = if raw_pid > 0 { Some(raw_pid) } else { None };

            reader.read_exact(&mut buf1)?;
            let flags = buf1[0];
            let visible = (flags & (1 << 0)) != 0;
            let locked = (flags & (1 << 1)) != 0;
            let alpha_locked = (flags & (1 << 2)) != 0;
            let clipping_mask = (flags & (1 << 3)) != 0;
            let is_reference = (flags & (1 << 4)) != 0;
            let is_expanded = (flags & (1 << 5)) != 0;
            let pass_through = (flags & (1 << 6)) != 0;

            (
                kind,
                parent_id,
                visible,
                locked,
                alpha_locked,
                clipping_mask,
                is_reference,
                is_expanded,
                pass_through,
            )
        } else if is_v3 {
            reader.read_exact(&mut buf1)?;
            let kind = if buf1[0] == 1 {
                aurelia_core::layer::LayerKind::Group
            } else {
                aurelia_core::layer::LayerKind::Raster
            };

            reader.read_exact(&mut buf8)?;
            let raw_pid = u64::from_le_bytes(buf8);
            let parent_id = if raw_pid > 0 { Some(raw_pid) } else { None };

            reader.read_exact(&mut buf1)?;
            let flags = buf1[0];
            let visible = (flags & (1 << 0)) != 0;
            let locked = (flags & (1 << 1)) != 0;
            let alpha_locked = (flags & (1 << 2)) != 0;
            let clipping_mask = (flags & (1 << 3)) != 0;
            let is_reference = (flags & (1 << 4)) != 0;
            let is_expanded = (flags & (1 << 5)) != 0;
            let pass_through = (flags & (1 << 6)) != 0;

            (
                kind,
                parent_id,
                visible,
                locked,
                alpha_locked,
                clipping_mask,
                is_reference,
                is_expanded,
                pass_through,
            )
        } else {
            reader.read_exact(&mut buf1)?;
            let visible = buf1[0] != 0;

            reader.read_exact(&mut buf1)?;
            let locked = buf1[0] != 0;

            (
                aurelia_core::layer::LayerKind::Raster,
                None,
                visible,
                locked,
                false,
                false,
                false,
                true,
                false,
            )
        };

        reader.read_exact(&mut buf4)?;
        let opacity = f32::from_le_bytes(buf4);

        reader.read_exact(&mut buf1)?;
        let blend_mode = match buf1[0] {
            1 => BlendMode::Multiply,
            2 => BlendMode::Screen,
            3 => BlendMode::Overlay,
            4 => BlendMode::Darken,
            5 => BlendMode::Lighten,
            6 => BlendMode::ColorDodge,
            7 => BlendMode::ColorBurn,
            8 => BlendMode::HardLight,
            9 => BlendMode::SoftLight,
            10 => BlendMode::Difference,
            11 => BlendMode::Exclusion,
            12 => BlendMode::Luminosity,
            13 => BlendMode::Adaptive,
            _ => BlendMode::Normal,
        };

        reader.read_exact(&mut buf4)?;
        let offset_x = i32::from_le_bytes(buf4);

        reader.read_exact(&mut buf4)?;
        let offset_y = i32::from_le_bytes(buf4);

        // v3: inherit the document format, or name this layer's own.
        let layer_format = if is_aur_v3 {
            reader.read_exact(&mut buf1)?;
            match buf1[0] {
                FORMAT_INHERIT => document_format,
                LAYER_FORMAT_SRGB8 => aurelia_core::layer::PixelFormat::Srgb8,
                LAYER_FORMAT_SRGB16 => aurelia_core::layer::PixelFormat::Srgb16,
                _ => return Err(ProjectError::CorruptData),
            }
        } else {
            aurelia_core::layer::PixelFormat::Srgb8
        };

        // Reserve the aggregate budget before reading or decompressing this layer's payload.
        // Groups and adjustment layers have no pixel buffer; raster and text layers do.
        let storage_bytes = if matches!(
            kind,
            aurelia_core::layer::LayerKind::Group | aurelia_core::layer::LayerKind::Adjustment
        ) {
            None
        } else {
            let bytes_per_pixel = if layer_format == aurelia_core::layer::PixelFormat::Srgb16 {
                8u64
            } else {
                4u64
            };
            let expected = layer_pixels
                .checked_mul(bytes_per_pixel)
                .ok_or(ProjectError::CorruptData)?;
            budget.reserve(layer_pixels, expected)?;
            Some(expected)
        };

        reader.read_exact(&mut buf4)?;
        let comp_len = u32::from_le_bytes(buf4) as usize;

        let mut layer = if kind == aurelia_core::layer::LayerKind::Adjustment {
            let adj_type = adjustment
                .as_ref()
                .map(|a| a.adjustment_type.clone())
                .unwrap_or(aurelia_core::layer::AdjustmentType::Invert);
            let mut adj_layer = Layer::new_adjustment(id, name, adj_type);
            adj_layer.is_expanded = is_expanded;
            adj_layer
        } else if kind == aurelia_core::layer::LayerKind::Group {
            let mut group = Layer::new_group(id, name);
            group.is_expanded = is_expanded;
            group.pass_through = pass_through;
            group
        } else {
            // The header check above caps width*height, so the exact payload size is known
            // before a single compressed byte is decoded. Bound the decoder to it: a small
            // high-ratio deflate stream must not be able to allocate gigabytes before the
            // length check below gets a chance to reject it.
            let expected = storage_bytes.ok_or(ProjectError::CorruptData)? as usize;

            let mut compressed = reader.by_ref().take(comp_len as u64);
            let mut raw = Vec::new();
            {
                let mut decoder = DeflateDecoder::new(&mut compressed).take(expected as u64 + 1);
                decoder.read_to_end(&mut raw)?;
            }
            // A deflate stream can end before comp_len; discard the remainder so the reader
            // is positioned at the mask section that follows.
            std::io::copy(&mut compressed, &mut std::io::sink())?;

            if raw.len() != expected {
                return Err(ProjectError::CorruptData);
            }
            match layer_format {
                aurelia_core::layer::PixelFormat::Srgb8 => {
                    Layer::from_pixels(id, name, width, height, raw)
                }
                aurelia_core::layer::PixelFormat::Srgb16 => {
                    // A 16-bit payload that is short or long is corrupt, not a reason to
                    // reinterpret it as 8-bit.
                    let channels: Vec<u16> = raw
                        .chunks_exact(2)
                        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                        .collect();
                    let mut layer = Layer::from_pixels(id, name, width, height, Vec::new());
                    *layer.buffer_mut() = aurelia_core::layer::LayerBuffer::Srgb16(channels);
                    layer
                }
            }
        };

        layer.kind = kind;
        layer.adjustment = adjustment;
        layer.text = text_config;
        layer.parent_id = parent_id;
        layer.visible = visible;
        layer.locked = locked;
        layer.alpha_locked = alpha_locked;
        layer.clipping_mask = clipping_mask;
        layer.is_reference = is_reference;
        layer.opacity = opacity;
        layer.blend_mode = blend_mode;
        layer.offset_x = offset_x;
        layer.offset_y = offset_y;

        if is_aur_v2 || is_aur_v3 {
            reader.read_exact(&mut buf1)?;
            if buf1[0] == 1 {
                reader.read_exact(&mut buf1)?;
                let enabled = buf1[0] != 0;
                // A mask is another width*height buffer; charge it to the same document budget
                // before allocating it.
                budget.reserve(0, layer_pixels)?;
                let mask_len = layer_pixels as usize;
                let mut mask_pixels = vec![0u8; mask_len];
                reader.read_exact(&mut mask_pixels)?;
                layer.mask = Some(aurelia_core::layer::LayerMask {
                    width,
                    height,
                    pixels: mask_pixels,
                    enabled,
                });
            }
        }

        layers.push(layer);
    }

    let editor = read_editor_state(&mut reader).unwrap_or_default();
    // Optional trailing sections are read as a sequence: AUR v3 files may end after the preview,
    // carry styles, carry an ICC profile, or both in either order. Reading each "magic" naively
    // would consume the next section's magic when one is absent, so parse the remaining bytes as
    // length-prefixed sections and ignore unknown ones.
    let (styles, icc_profile) = read_trailing_sections(&mut reader);
    if let Some(styles) = styles {
        for (id, layer_styles) in styles {
            if let Some(layer) = layers.iter_mut().find(|layer| layer.id == id) {
                layer.styles = layer_styles;
            }
        }
    }

    let document = Document {
        width,
        height,
        layers,
        active_layer_id,
        background_value,
        is_transparent,
        theme,
        next_layer_id: max_id + 1,
        // The tag is authoritative for new layers; per-layer tags above resolved to their own
        // formats, so a mixed file still loads exactly as it was saved.
        pixel_format: document_format,
        editor,
        icc_profile,
    };

    // The file is untrusted input: `parent_id` values are read verbatim and can describe
    // missing parents, non-group parents, self-parenting or a cycle. Reject the whole project
    // instead of handing a graph to traversals that assume a forest.
    document
        .validate_layer_hierarchy()
        .map_err(|_| ProjectError::CorruptData)?;

    Ok(document)
}

pub fn load_project_file(path: impl AsRef<Path>) -> Result<Document, ProjectError> {
    let file = File::open(path)?;
    load_project_from_reader(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurelia_core::brush::TextAlign;
    use aurelia_core::layer::{LayerKind, TextLayerConfig};

    #[test]
    fn oversized_text_claim_is_rejected_before_reading_the_payload() {
        let mut cursor = Cursor::new((MAX_TEXT_CONTENT_BYTES + 1).to_le_bytes().to_vec());
        assert!(matches!(
            read_text_content(&mut cursor),
            Err(ProjectError::TextTooLarge { .. })
        ));
        assert_eq!(
            cursor.position(),
            4,
            "only the length should be consumed before the bound check"
        );
    }

    #[test]
    fn oversized_preview_and_editor_claims_are_ignored() {
        let mut huge_preview = Cursor::new(u32::MAX.to_le_bytes().to_vec());
        assert!(read_editor_state(&mut huge_preview).is_none());

        let mut huge_editor = Vec::new();
        huge_editor.extend_from_slice(&0u32.to_le_bytes()); // no preview
        huge_editor.extend_from_slice(b"AED1");
        huge_editor.extend_from_slice(&(EDITOR_MAX_BYTES + 1).to_le_bytes());
        assert!(read_editor_state(&mut Cursor::new(huge_editor)).is_none());
    }

    #[test]
    fn saving_an_oversized_text_layer_is_rejected() {
        let mut doc = Document::new(1, 1);
        let layer = &mut doc.layers[0];
        layer.kind = LayerKind::Text;
        layer.text = Some(TextLayerConfig {
            content: "x".repeat(MAX_TEXT_CONTENT_BYTES as usize + 1),
            font_family: "sans".into(),
            font_size: 12.0,
            color: [0, 0, 0, 255],
            align: TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            box_w: 1.0,
            box_h: 1.0,
            pos_x: 0.0,
            pos_y: 0.0,
        });
        let mut out = Vec::new();
        assert!(matches!(
            save_project_to_writer(&doc, &mut out),
            Err(ProjectError::TextTooLarge { .. })
        ));
    }

    #[test]
    fn temporary_save_paths_are_unique_siblings() {
        let target = Path::new("art.aur");
        let first = unique_temp_path(target);
        let second = unique_temp_path(target);
        assert_ne!(first, second);
        assert_eq!(first.parent(), target.parent());
        assert!(first.to_string_lossy().ends_with(".tmp"));
    }

    #[test]
    fn layer_name_length_boundary_is_enforced() {
        let mut doc = Document::new(1, 1);
        doc.layers[0].name = "x".repeat(u16::MAX as usize);
        let mut bytes = Vec::new();
        save_project_to_writer(&doc, &mut bytes).expect("maximum name must save");
        let loaded = load_project_from_reader(&bytes[..]).expect("maximum name must load");
        assert_eq!(loaded.layers[0].name.len(), u16::MAX as usize);

        doc.layers[0].name.push('x');
        let mut out = Vec::new();
        assert!(matches!(
            save_project_to_writer(&doc, &mut out),
            Err(ProjectError::TextFieldTooLong {
                field: "layer name",
                ..
            })
        ));
    }

    #[test]
    fn too_many_layers_are_rejected_before_reading_them() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(AUR_MAGIC_V3);
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.push(255); // background
        bytes.push(0); // transparent
        bytes.push(0); // theme
        bytes.push(0); // sRGB8
        bytes.extend_from_slice(&(MAX_PROJECT_LAYERS + 1).to_le_bytes());

        assert!(matches!(
            load_project_from_reader(Cursor::new(&bytes)),
            Err(ProjectError::CorruptData)
        ));
        assert!(DecodeBudget::new(MAX_PROJECT_LAYERS).is_ok());
    }

    #[test]
    fn aggregate_decode_budget_rejects_excess_pixels_and_bytes() {
        let mut pixels = DecodeBudget::new(0).expect("budget");
        assert!(pixels.reserve(MAX_PROJECT_PIXELS, 0).is_ok());
        assert!(pixels.reserve(1, 0).is_err());

        let mut bytes = DecodeBudget::new(0).expect("budget");
        assert!(bytes.reserve(0, MAX_PROJECT_DECODED_BYTES).is_ok());
        assert!(bytes.reserve(0, 1).is_err());
    }
}
