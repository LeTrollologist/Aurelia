//! Layer styles: stroke, drop shadow and outer glow, evaluated at composite time.
//!
//! A style is a function of the layer's own alpha and colour. Nothing is baked into the
//! layer's pixels: the compositor asks for a styled copy of the layer each time it draws it,
//! exactly like a blend mode or an adjustment. That keeps styles non-destructive, undoable by
//! changing the style record only, and consistent between previews and exports.
//!
//! The renderer works in the layer's own pixel space (not the canvas), so offsets, masks,
//! clipping, opacity and blend modes keep their existing meaning. Effects that extend past the
//! layer's bounds are clipped to them; editor-created layers are canvas-sized, so in practice
//! that only clips at the canvas edge.
//!
//! Working precision is `f32` per channel for the masks and the composite; the result is
//! quantised once, into the layer's storage width (8 or 16 bits).

use crate::color::Color;
use crate::layer::{Layer, LayerBuffer};
use crate::pixel::Channel;
use serde::{Deserialize, Serialize};

/// One layer's style stack. `Default` (all `None`) is the common case and costs nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LayerStyles {
    #[serde(default)]
    pub stroke: Option<StrokeStyle>,
    #[serde(default)]
    pub drop_shadow: Option<DropShadowStyle>,
    #[serde(default)]
    pub outer_glow: Option<OuterGlowStyle>,
}

impl LayerStyles {
    pub fn is_empty(&self) -> bool {
        self.stroke.is_none() && self.drop_shadow.is_none() && self.outer_glow.is_none()
    }

    /// Styles that would actually draw: present, with non-zero width/radius and opacity.
    pub fn has_visible_effect(&self) -> bool {
        self.stroke.as_ref().is_some_and(|s| s.width > 0.0 && s.opacity > 0.0)
            || self
                .drop_shadow
                .as_ref()
                .is_some_and(|s| s.opacity > 0.0 && (s.blur > 0.0 || s.offset_x != 0.0 || s.offset_y != 0.0))
            || self
                .outer_glow
                .as_ref()
                .is_some_and(|s| s.radius > 0.0 && s.opacity > 0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokeStyle {
    /// Stroke width in pixels, measured outward from the silhouette.
    pub width: f32,
    pub color: Color,
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropShadowStyle {
    pub offset_x: f32,
    pub offset_y: f32,
    /// Gaussian-ish blur radius in pixels; 0 is a hard shadow.
    pub blur: f32,
    pub color: Color,
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OuterGlowStyle {
    pub radius: f32,
    pub color: Color,
    pub opacity: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            width: 3.0,
            color: Color::BLACK,
            opacity: 1.0,
        }
    }
}

impl Default for DropShadowStyle {
    fn default() -> Self {
        Self {
            offset_x: 6.0,
            offset_y: 6.0,
            blur: 6.0,
            color: Color::BLACK,
            opacity: 0.6,
        }
    }
}

impl Default for OuterGlowStyle {
    fn default() -> Self {
        Self {
            radius: 8.0,
            color: Color::BRAND_GOLD,
            opacity: 0.8,
        }
    }
}

/// Renders the layer plus its styles into a new buffer of the layer's storage width.
pub fn render_styled(layer: &Layer) -> LayerBuffer {
    match layer.buffer() {
        LayerBuffer::Srgb8(source) => {
            LayerBuffer::Srgb8(render(source, layer.width, layer.height, &layer.styles))
        }
        LayerBuffer::Srgb16(source) => {
            LayerBuffer::Srgb16(render(source, layer.width, layer.height, &layer.styles))
        }
    }
}

/// Renders styles at 16-bit precision regardless of the layer's storage width.
///
/// The mixed-document compositor promotes 8-bit layers to 16 bits before compositing; styles
/// must be evaluated at that width too, or a glow would be quantised to the 8-bit lattice
/// before it ever reaches the working-precision path.
pub fn render_styled_srgb16(layer: &Layer) -> Vec<u16> {
    match layer.buffer() {
        LayerBuffer::Srgb16(source) => render(source, layer.width, layer.height, &layer.styles),
        LayerBuffer::Srgb8(source) => {
            let promoted: Vec<u16> = source.iter().map(|&v| crate::pixel::srgb8_to_srgb16(v)).collect();
            render(&promoted, layer.width, layer.height, &layer.styles)
        }
    }
}

/// Generic renderer: straight RGBA in the channel's own scale out, same size as the source.
pub fn render<C: Channel>(source: &[C], width: u32, height: u32, styles: &LayerStyles) -> Vec<C> {
    let mut out = vec![C::from_scaled_f32(0.0); source.len().min((width * height * 4) as usize)];
    if width == 0 || height == 0 || out.len() < (width * height * 4) as usize {
        return out;
    }
    let n = (width * height) as usize;

    // Alpha silhouette in working precision.
    let mut alpha = vec![0.0_f32; n];
    for (index, pixel) in source.chunks_exact(4).take(n).enumerate() {
        alpha[index] = pixel[3].to_scaled_f32() / C::MAX_SCALED as f32;
    }

    // Composite canvas, straight alpha.
    let mut canvas = vec![[0.0_f32; 4]; n];

    if let Some(shadow) = &styles.drop_shadow {
        if shadow.opacity > 0.0 {
            let mut mask = vec![0.0_f32; n];
            let dx = shadow.offset_x.round() as i32;
            let dy = shadow.offset_y.round() as i32;
            for y in 0..height as i32 {
                for x in 0..width as i32 {
                    let sx = x - dx;
                    let sy = y - dy;
                    if sx < 0 || sy < 0 || sx >= width as i32 || sy >= height as i32 {
                        continue;
                    }
                    mask[(y as usize) * width as usize + x as usize] =
                        alpha[(sy as usize) * width as usize + sx as usize];
                }
            }
            if shadow.blur > 0.0 {
                blur(&mut mask, width, height, shadow.blur);
            }
            let color = shadow.color.to_rgba8();
            let opacity = shadow.opacity.clamp(0.0, 1.0);
            for (index, coverage) in mask.iter().enumerate() {
                let a = coverage.clamp(0.0, 1.0) * opacity;
                if a > 0.0 {
                    blend_over(
                        &mut canvas[index],
                        [color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0, a],
                    );
                }
            }
        }
    }

    if let Some(glow) = &styles.outer_glow {
        if glow.opacity > 0.0 && glow.radius > 0.0 {
            let mut mask = alpha.clone();
            blur(&mut mask, width, height, glow.radius);
            let color = glow.color.to_rgba8();
            let opacity = glow.opacity.clamp(0.0, 1.0);
            for (index, coverage) in mask.iter().enumerate() {
                // Outside only: the layer itself is painted over the glow afterwards, and
                // multiplying by (1 - alpha) keeps a bright glow from washing out the art.
                let outside = (1.0 - alpha[index]).clamp(0.0, 1.0);
                let a = coverage.clamp(0.0, 1.0) * outside * opacity;
                if a > 0.0 {
                    blend_over(
                        &mut canvas[index],
                        [color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0, a],
                    );
                }
            }
        }
    }

    if let Some(stroke) = &styles.stroke {
        if stroke.opacity > 0.0 && stroke.width > 0.0 {
            let distance = outside_distance(&alpha, width, height);
            let color = stroke.color.to_rgba8();
            let opacity = stroke.opacity.clamp(0.0, 1.0);
            for index in 0..n {
                if alpha[index] > 0.5 {
                    continue;
                }
                let coverage = (stroke.width - distance[index] + 0.5).clamp(0.0, 1.0);
                let a = coverage * opacity;
                if a > 0.0 {
                    blend_over(
                        &mut canvas[index],
                        [color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0, a],
                    );
                }
            }
        }
    }

    // The layer itself, painted straight over the effects.
    for (index, pixel) in source.chunks_exact(4).take(n).enumerate() {
        blend_over(
            &mut canvas[index],
            [
                pixel[0].to_scaled_f32() / C::MAX_SCALED as f32,
                pixel[1].to_scaled_f32() / C::MAX_SCALED as f32,
                pixel[2].to_scaled_f32() / C::MAX_SCALED as f32,
                pixel[3].to_scaled_f32() / C::MAX_SCALED as f32,
            ],
        );
    }

    for (index, pixel) in canvas.iter().enumerate() {
        out[index * 4] = C::from_scaled_f32(pixel[0] * C::MAX_SCALED as f32);
        out[index * 4 + 1] = C::from_scaled_f32(pixel[1] * C::MAX_SCALED as f32);
        out[index * 4 + 2] = C::from_scaled_f32(pixel[2] * C::MAX_SCALED as f32);
        out[index * 4 + 3] = C::from_scaled_f32(pixel[3] * C::MAX_SCALED as f32);
    }
    out
}

/// Straight-alpha source-over into a straight-alpha destination.
#[inline]
fn blend_over(dst: &mut [f32; 4], src: [f32; 4]) {
    let sa = src[3].clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    let da = dst[3];
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    for channel in 0..3 {
        dst[channel] = (src[channel] * sa + dst[channel] * da * (1.0 - sa)) / out_a;
    }
    dst[3] = out_a;
}

/// Three-pass separable box blur, which is close enough to a Gaussian for an effect mask.
fn blur(mask: &mut [f32], width: u32, height: u32, radius: f32) {
    let radius = radius.max(0.0);
    if radius < 0.5 || width == 0 || height == 0 {
        return;
    }
    let kernel = ((radius / 1.5).round() as u32).max(1);
    let mut temp = vec![0.0_f32; mask.len()];
    for _ in 0..3 {
        blur_horizontal(mask, &mut temp, width, height, kernel);
        blur_vertical(&temp, mask, width, height, kernel);
    }
}

fn blur_horizontal(src: &[f32], dst: &mut [f32], width: u32, height: u32, radius: u32) {
    let w = width as usize;
    let window = (radius as usize * 2 + 1) as f32;
    for y in 0..height as usize {
        let row = y * w;
        let mut sum: f32 = 0.0;
        for x in 0..=(radius as usize).min(w.saturating_sub(1)) {
            sum += src[row + x];
        }
        for x in 0..w {
            dst[row + x] = sum / window;
            let out_index = x.saturating_sub(radius as usize);
            let in_index = x + radius as usize + 1;
            if x >= radius as usize {
                sum -= src[row + out_index];
            }
            if in_index < w {
                sum += src[row + in_index];
            }
        }
    }
}

fn blur_vertical(src: &[f32], dst: &mut [f32], width: u32, height: u32, radius: u32) {
    let w = width as usize;
    let h = height as usize;
    let window = (radius as usize * 2 + 1) as f32;
    for x in 0..w {
        let mut sum: f32 = 0.0;
        for y in 0..=(radius as usize).min(h.saturating_sub(1)) {
            sum += src[y * w + x];
        }
        for y in 0..h {
            dst[y * w + x] = sum / window;
            if y >= radius as usize {
                sum -= src[(y - radius as usize) * w + x];
            }
            let next = y + radius as usize + 1;
            if next < h {
                sum += src[next * w + x];
            }
        }
    }
}

/// Chamfer distance from every transparent pixel to the nearest covered pixel (> 0.5 alpha).
///
/// Three-to-four chamfer weights approximate Euclidean distance closely enough for a stroke
/// band, and the two-pass sweep is linear in pixels regardless of stroke width.
fn outside_distance(alpha: &[f32], width: u32, height: u32) -> Vec<f32> {
    let w = width as usize;
    let h = height as usize;
    let big = (w + h + 2) as f32 * 4.0;
    let mut distance = vec![big; w * h];
    for (index, value) in alpha.iter().enumerate() {
        if *value > 0.5 {
            distance[index] = 0.0;
        }
    }
    fn at(distance: &[f32], w: usize, h: usize, big: f32, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            big
        } else {
            distance[(y as usize) * w + x as usize]
        }
    }
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let index = (y as usize) * w + x as usize;
            let mut best = distance[index];
            best = best.min(at(&distance, w, h, big, x - 1, y) + 3.0);
            best = best.min(at(&distance, w, h, big, x, y - 1) + 3.0);
            best = best.min(at(&distance, w, h, big, x - 1, y - 1) + 4.0);
            best = best.min(at(&distance, w, h, big, x + 1, y - 1) + 4.0);
            distance[index] = best;
        }
    }
    for y in (0..h as i32).rev() {
        for x in (0..w as i32).rev() {
            let index = (y as usize) * w + x as usize;
            let mut best = distance[index];
            best = best.min(at(&distance, w, h, big, x + 1, y) + 3.0);
            best = best.min(at(&distance, w, h, big, x, y + 1) + 3.0);
            best = best.min(at(&distance, w, h, big, x + 1, y + 1) + 4.0);
            best = best.min(at(&distance, w, h, big, x - 1, y + 1) + 4.0);
            distance[index] = best;
        }
    }
    for value in &mut distance {
        *value /= 3.0;
    }
    distance
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::Layer;

    fn layer_with_square(size: u32) -> Layer {
        let mut layer = Layer::new(1, "Square", size, size);
        if let Some(pixels) = layer.as_srgb8_mut() {
            for y in 10..12 {
                for x in 10..12 {
                    let at = ((y * size + x) * 4) as usize;
                    pixels[at] = 255;
                    pixels[at + 1] = 0;
                    pixels[at + 2] = 0;
                    pixels[at + 3] = 255;
                }
            }
        }
        layer
    }

    fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }

    #[test]
    fn no_styles_returns_the_source_exactly() {
        let layer = layer_with_square(32);
        let styled = render_styled(&layer);
        match (styled, layer.buffer()) {
            (LayerBuffer::Srgb8(a), LayerBuffer::Srgb8(b)) => assert_eq!(&a, b),
            _ => panic!("format mismatch"),
        }
    }

    #[test]
    fn drop_shadow_is_offset_and_coloured() {
        let mut layer = layer_with_square(32);
        layer.styles.drop_shadow = Some(DropShadowStyle {
            offset_x: 5.0,
            offset_y: 0.0,
            blur: 0.0,
            color: Color::BLACK,
            opacity: 1.0,
        });
        let LayerBuffer::Srgb8(out) = render_styled(&layer) else {
            panic!("8-bit");
        };
        assert_eq!(pixel(&out, 32, 16, 10), [0, 0, 0, 255], "shadow follows the offset");
        assert_eq!(pixel(&out, 32, 10, 10), [255, 0, 0, 255], "layer paints over its shadow");
        assert_eq!(pixel(&out, 32, 5, 5)[3], 0, "no shadow where the shifted silhouette is absent");
    }

    #[test]
    fn glow_and_stroke_extend_beyond_the_silhouette() {
        let mut layer = layer_with_square(32);
        layer.styles.outer_glow = Some(OuterGlowStyle {
            radius: 3.0,
            color: Color::from_hex("#00ff00").expect("green"),
            opacity: 1.0,
        });
        layer.styles.stroke = Some(StrokeStyle {
            width: 1.5,
            color: Color::from_hex("#0000ff").expect("blue"),
            opacity: 1.0,
        });
        let LayerBuffer::Srgb8(out) = render_styled(&layer) else {
            panic!("8-bit");
        };
        // The stroke band sits immediately outside the square and is drawn above the glow.
        let edge = pixel(&out, 32, 9, 10);
        assert!(edge[3] > 0, "stroke must cover the pixel just outside the silhouette");
        assert!(edge[2] > edge[1], "stroke colour should dominate at the edge: {edge:?}");
        let glow = pixel(&out, 32, 9, 6);
        assert!(glow[3] > 0, "glow reaches a few pixels out");
        assert!(glow[1] > glow[2], "glow is green, not blue: {glow:?}");
    }

    #[test]
    fn sixteen_bit_effects_keep_sub_lattice_alpha() {
        let mut layer = Layer::new(1, "Wide", 16, 16);
        layer.convert_format(crate::layer::PixelFormat::Srgb16);
        if let Some(pixels) = layer.as_srgb16_mut() {
            for y in 4..8 {
                for x in 4..8 {
                    let at = ((y * 16 + x) * 4) as usize;
                    pixels[at] = 65535;
                    pixels[at + 3] = 65535;
                }
            }
        }
        layer.styles.outer_glow = Some(OuterGlowStyle {
            radius: 2.0,
            color: Color::WHITE,
            opacity: 0.5,
        });
        let LayerBuffer::Srgb16(out) = render_styled(&layer) else {
            panic!("16-bit");
        };
        let at = ((6 * 16 + 6) * 4) as usize;
        assert_eq!(out[at + 3], 65535, "layer alpha stays exact");
        let outside = ((3 * 16 + 6) * 4) as usize;
        let alpha = out[outside + 3];
        assert!(alpha > 0, "glow must be present");
        assert_ne!(alpha % 257, 0, "16-bit glow alpha must not collapse to the 8-bit lattice");
    }

    #[test]
    fn styles_are_clipped_to_the_layer_buffer() {
        let mut layer = layer_with_square(12);
        layer.styles.drop_shadow = Some(DropShadowStyle {
            offset_x: 40.0,
            offset_y: 40.0,
            blur: 0.0,
            color: Color::BLACK,
            opacity: 1.0,
        });
        let LayerBuffer::Srgb8(out) = render_styled(&layer) else {
            panic!("8-bit");
        };
        // The shadow falls entirely outside a 12x12 buffer; nothing may panic or wrap.
        assert!(out.chunks_exact(4).all(|px| px[3] == 0 || px[0] == 255));
    }
}
