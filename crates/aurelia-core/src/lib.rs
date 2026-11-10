pub mod animation;
pub mod assets;
pub mod blend;
pub mod brush;
pub mod brush_import;
pub mod clipboard;
pub mod color;
pub mod color_management;
pub mod document;
pub mod filter;
pub mod history;
pub mod layer;
pub mod layer_style;
pub mod perspective;
pub mod pixel;
pub mod rasterizer;
pub mod selection;
pub mod symmetry;
pub mod transform;

pub use animation::{AnimationFrame, AnimationTimeline};
pub use blend::BlendMode;
pub use brush::{
    BrushPoint, BrushPreset, BrushSettings, EraserMode, GradientType, ShapeFillMode, TextAlign,
    TextSettings, ToolType,
};
pub use brush_import::{decode_brush_file, BrushImportError, BrushStamp};
pub use clipboard::ClipboardImage;
pub use color::{Color, ThemeMode, DEFAULT_PALETTE};
pub use color_management::{ColorProfileError, IccProfile};
pub use document::{CompositeError, Document, HierarchyError, MAX_LAYER_HIERARCHY_DEPTH};
pub use history::{Command, HistoryStack, LayerPixelsDeltaCommand, ProfileConversionCommand};
pub use layer::{
    truncate_layer_name, AdjustmentConfig, AdjustmentType, Layer, LayerId, LayerKind,
    MAX_LAYER_NAME_BYTES,
};
pub use layer_style::{DropShadowStyle, LayerStyles, OuterGlowStyle, StrokeStyle};
pub use perspective::{PerspectiveConfig, PerspectiveType};
pub use rasterizer::StrokeRasterizer;
pub use selection::{SelectionCombineMode, SelectionMask, StrokePosition};
pub use symmetry::{SymmetryConfig, SymmetryMode};
pub use transform::{render_transformed_patch, sample_bilinear, sample_nearest, AffineTransform2D};

#[cfg(test)]
mod tests;
