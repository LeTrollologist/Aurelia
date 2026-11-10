use serde::{Deserialize, Serialize};

/// One blend mode's pixel maths as a plain item function.
///
/// `composite_pixel` is marked `#[inline]` and the mode here is a literal, so each of
/// these collapses to just that mode's branch of float maths. A compositing loop that is
/// driven by one of them therefore has no per-pixel `match` and no indirect call, while
/// the maths itself stays single-sourced in `composite_pixel`.
macro_rules! define_compositors {
    ($($name:ident => $mode:ident),+ $(,)?) => {
        $(
            #[inline]
            pub fn $name(dst: [f32; 4], src: [f32; 4], layer_opacity: f32) -> [f32; 4] {
                BlendMode::$mode.composite_working(dst, src, layer_opacity)
            }
        )+
    };
}

define_compositors! {
    compose_normal => Normal,
    compose_multiply => Multiply,
    compose_screen => Screen,
    compose_overlay => Overlay,
    compose_darken => Darken,
    compose_lighten => Lighten,
    compose_color_dodge => ColorDodge,
    compose_color_burn => ColorBurn,
    compose_hard_light => HardLight,
    compose_soft_light => SoftLight,
    compose_difference => Difference,
    compose_exclusion => Exclusion,
    compose_luminosity => Luminosity,
    compose_adaptive => Adaptive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BlendMode {
    #[default]
    #[serde(rename = "source-over")]
    Normal,
    #[serde(rename = "multiply")]
    Multiply,
    #[serde(rename = "screen")]
    Screen,
    #[serde(rename = "overlay")]
    Overlay,
    #[serde(rename = "darken")]
    Darken,
    #[serde(rename = "lighten")]
    Lighten,
    #[serde(rename = "color-dodge")]
    ColorDodge,
    #[serde(rename = "color-burn")]
    ColorBurn,
    #[serde(rename = "hard-light")]
    HardLight,
    #[serde(rename = "soft-light")]
    SoftLight,
    #[serde(rename = "difference")]
    Difference,
    #[serde(rename = "exclusion")]
    Exclusion,
    #[serde(rename = "luminosity")]
    Luminosity,
    /// Per-pixel auto-contrast: keeps the source colour when it is already the more
    /// contrasting of {colour, inverted colour} against the backdrop, and inverts it when
    /// that reads better. Adobe's closest mode is Difference, which always inverts; this
    /// chooses only when inversion helps, so neutral art on a neutral background is not
    /// needlessly flipped.
    #[serde(rename = "adaptive")]
    Adaptive,
}

impl BlendMode {
    pub const ALL: &'static [BlendMode] = &[
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Luminosity,
        BlendMode::Adaptive,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Multiply => "Multiply",
            Self::Screen => "Screen",
            Self::Overlay => "Overlay",
            Self::Darken => "Darken",
            Self::Lighten => "Lighten",
            Self::ColorDodge => "Color Dodge",
            Self::ColorBurn => "Color Burn",
            Self::HardLight => "Hard Light",
            Self::SoftLight => "Soft Light",
            Self::Difference => "Difference",
            Self::Exclusion => "Exclusion",
            Self::Luminosity => "Luminosity",
            Self::Adaptive => "Adaptive",
        }
    }

    pub fn from_str_name(s: &str) -> Self {
        match s {
            "multiply" => Self::Multiply,
            "screen" => Self::Screen,
            "overlay" => Self::Overlay,
            "darken" => Self::Darken,
            "lighten" => Self::Lighten,
            "color-dodge" => Self::ColorDodge,
            "color-burn" => Self::ColorBurn,
            "hard-light" => Self::HardLight,
            "soft-light" => Self::SoftLight,
            "difference" => Self::Difference,
            "exclusion" => Self::Exclusion,
            "luminosity" => Self::Luminosity,
            "adaptive" => Self::Adaptive,
            _ => Self::Normal,
        }
    }

    /// Blend single RGB component in 0.0..=1.0 range
    #[inline]
    pub fn blend_channel(&self, cb: f32, cs: f32) -> f32 {
        match self {
            Self::Normal => cs,
            Self::Multiply => cb * cs,
            Self::Screen => cb + cs - cb * cs,
            Self::Overlay => {
                if cb <= 0.5 {
                    2.0 * cb * cs
                } else {
                    1.0 - 2.0 * (1.0 - cb) * (1.0 - cs)
                }
            }
            Self::Darken => cb.min(cs),
            Self::Lighten => cb.max(cs),
            Self::ColorDodge => {
                if cb <= 0.0 {
                    0.0
                } else if cs >= 1.0 {
                    1.0
                } else {
                    (cb / (1.0 - cs)).min(1.0)
                }
            }
            Self::ColorBurn => {
                if cb >= 1.0 {
                    1.0
                } else if cs <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - cb) / cs).min(1.0)
                }
            }
            Self::HardLight => {
                if cs <= 0.5 {
                    2.0 * cb * cs
                } else {
                    1.0 - 2.0 * (1.0 - cb) * (1.0 - cs)
                }
            }
            Self::SoftLight => {
                if cs <= 0.5 {
                    cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
                } else {
                    let d = if cb <= 0.25 {
                        ((16.0 * cb - 12.0) * cb + 4.0) * cb
                    } else {
                        cb.sqrt()
                    };
                    cb + (2.0 * cs - 1.0) * (d - cb)
                }
            }
            Self::Difference => (cb - cs).abs(),
            Self::Exclusion => cb + cs - 2.0 * cb * cs,
            Self::Luminosity | Self::Adaptive => cs, // handled in full RGB conversion
        }
    }

    /// Full pixel compositing: dst is background, src is foreground with layer opacity
    #[inline]
    /// The blend maths, in working precision.
    ///
    /// This is the single source of truth for how pixels combine. `composite_pixel` is a
    /// thin 8-bit wrapper over it, and the compositor calls it through the monomorphised
    /// `compose_*` functions below, so there is one implementation rather than one per
    /// storage width.
    pub fn composite_working(&self, dst: [f32; 4], src: [f32; 4], layer_opacity: f32) -> [f32; 4] {
        let sa = src[3] * layer_opacity.clamp(0.0, 1.0);
        if sa <= 0.0001 {
            return dst;
        }

        let da = dst[3];
        let out_a = sa + da * (1.0 - sa);
        if out_a <= 0.0001 {
            return [0.0, 0.0, 0.0, 0.0];
        }

        let sr = src[0];
        let sg = src[1];
        let sb = src[2];

        let dr = dst[0];
        let dg = dst[1];
        let db = dst[2];

        let (br, bg, bb) = match self {
            Self::Luminosity => {
                let lum_dst = 0.299 * dr + 0.587 * dg + 0.114 * db;
                let lum_src = 0.299 * sr + 0.587 * sg + 0.114 * sb;
                let diff = lum_src - lum_dst;
                let mut cr = dr + diff;
                let mut cg = dg + diff;
                let mut cb_val = db + diff;
                // W3C ClipColor: proportionally scale towards luma to stay in gamut
                let lum = 0.299 * cr + 0.587 * cg + 0.114 * cb_val;
                let cmin = cr.min(cg).min(cb_val);
                let cmax = cr.max(cg).max(cb_val);
                if cmin < 0.0 && (lum - cmin).abs() > 1e-6 {
                    let s = lum / (lum - cmin);
                    cr = lum + (cr - lum) * s;
                    cg = lum + (cg - lum) * s;
                    cb_val = lum + (cb_val - lum) * s;
                }
                if cmax > 1.0 && (cmax - lum).abs() > 1e-6 {
                    let s = (1.0 - lum) / (cmax - lum);
                    cr = lum + (cr - lum) * s;
                    cg = lum + (cg - lum) * s;
                    cb_val = lum + (cb_val - lum) * s;
                }
                (
                    cr.clamp(0.0, 1.0),
                    cg.clamp(0.0, 1.0),
                    cb_val.clamp(0.0, 1.0),
                )
            }
            Self::Adaptive => {
                // Auto-contrast: pick the reading of the source (as authored, or inverted)
                // that is farther in luminance from the backdrop. Ties keep the source, so a
                // mid-grey layer on a mid-grey backdrop is not arbitrarily flipped.
                let lum_dst = 0.299 * dr + 0.587 * dg + 0.114 * db;
                let lum_src = 0.299 * sr + 0.587 * sg + 0.114 * sb;
                let as_is = (lum_src - lum_dst).abs();
                let inverted = ((1.0 - lum_src) - lum_dst).abs();
                if as_is >= inverted {
                    (sr, sg, sb)
                } else {
                    (1.0 - sr, 1.0 - sg, 1.0 - sb)
                }
            }
            _ => (
                self.blend_channel(dr, sr).clamp(0.0, 1.0),
                self.blend_channel(dg, sg).clamp(0.0, 1.0),
                self.blend_channel(db, sb).clamp(0.0, 1.0),
            ),
        };

        // Alpha compositing formula:
        // C_out = (1 - sa) * C_dst + (1 - da) * C_src + sa * da * B(C_dst, C_src)
        let cr = ((1.0 - sa) * da * dr + (1.0 - da) * sa * sr + sa * da * br) / out_a;
        let cg = ((1.0 - sa) * da * dg + (1.0 - da) * sa * sg + sa * da * bg) / out_a;
        let cb = ((1.0 - sa) * da * db + (1.0 - da) * sa * sb + sa * da * bb) / out_a;

        [cr, cg, cb, out_a]
    }

    /// Composites one 8-bit pixel.
    ///
    /// Storage wrapper over `composite_working`: the maths lives there, and this exists so
    /// callers holding 8-bit storage (the adjustment pass, the rasterizer) keep working.
    #[inline]
    pub fn composite_pixel(&self, dst: [u8; 4], src: [u8; 4], layer_opacity: f32) -> [u8; 4] {
        // Guard kept here too, so a transparent source hands the destination straight back
        // without a round trip through working precision.
        let sa = (src[3] as f32 / 255.0) * layer_opacity.clamp(0.0, 1.0);
        if sa <= 0.0001 {
            return dst;
        }
        let out = self.composite_working(
            [
                dst[0] as f32 / 255.0,
                dst[1] as f32 / 255.0,
                dst[2] as f32 / 255.0,
                dst[3] as f32 / 255.0,
            ],
            [
                src[0] as f32 / 255.0,
                src[1] as f32 / 255.0,
                src[2] as f32 / 255.0,
                src[3] as f32 / 255.0,
            ],
            layer_opacity,
        );
        [
            crate::pixel::Channel::from_f32(out[0]),
            crate::pixel::Channel::from_f32(out[1]),
            crate::pixel::Channel::from_f32(out[2]),
            crate::pixel::Channel::from_f32(out[3]),
        ]
    }
}

#[cfg(test)]
mod adaptive_tests {
    use super::BlendMode;

    fn over(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
        BlendMode::Adaptive.composite_working(dst, src, 1.0)
    }

    fn approx(actual: [f32; 4], expected: [f32; 4]) {
        for channel in 0..4 {
            assert!(
                (actual[channel] - expected[channel]).abs() < 0.002,
                "channel {channel}: {actual:?} != {expected:?}"
            );
        }
    }

    /// The point of the mode: the same artwork stays legible on either backdrop.
    #[test]
    fn black_art_flips_on_black_and_stays_on_white() {
        approx(
            over([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]),
            [1.0, 1.0, 1.0, 1.0],
        );
        approx(
            over([1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]),
            [0.0, 0.0, 0.0, 1.0],
        );
    }

    #[test]
    fn white_art_flips_on_white_and_stays_on_black() {
        approx(
            over([1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]),
            [0.0, 0.0, 0.0, 1.0],
        );
        approx(
            over([0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]),
            [1.0, 1.0, 1.0, 1.0],
        );
    }

    /// Equal distance is a tie; the authored colour wins rather than being flipped at random.
    #[test]
    fn a_neutral_tie_keeps_the_source() {
        approx(
            over([0.5, 0.5, 0.5, 1.0], [0.5, 0.5, 0.5, 1.0]),
            [0.5, 0.5, 0.5, 1.0],
        );
    }

    #[test]
    fn colour_is_inverted_as_a_whole_not_per_channel() {
        // Red against red has zero contrast as-is; cyan has 0.4 luminance distance.
        approx(
            over([1.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0]),
            [0.0, 1.0, 1.0, 1.0],
        );
    }

    #[test]
    fn partial_alpha_composites_the_chosen_colour_normally() {
        // Half-opaque black text on black chooses white, then blends: 0.5 grey.
        approx(
            over([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 0.5]),
            [0.5, 0.5, 0.5, 1.0],
        );
        // Half-opaque black on white stays black and lightens to 0.5.
        approx(
            over([1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.5]),
            [0.5, 0.5, 0.5, 1.0],
        );
    }

    #[test]
    fn transparent_source_leaves_the_backdrop_untouched() {
        approx(
            over([0.2, 0.4, 0.6, 1.0], [1.0, 0.0, 0.0, 0.0]),
            [0.2, 0.4, 0.6, 1.0],
        );
    }
}
