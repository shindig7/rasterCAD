//! The scene tree: what evaluation produces and the renderer consumes.

use std::rc::Rc;

use tiny_skia::{Color, Path, Pixmap, Transform};

use crate::diag::Span;
use crate::render::filters::FilterDef;

#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub background: Color,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Node {
    /// A filled outline (shapes and text). Stroked instead when inside `stroke()`.
    Shape(Path),
    /// A raster image drawn with its top-left corner at the origin, one unit per image pixel.
    Image(Rc<Pixmap>),
    Transform(Transform, Vec<Node>),
    Color(Color, Vec<Node>),
    Stroke(f32, Vec<Node>),
    /// Children rendered to an offscreen layer, optionally filtered, then composited with `opacity`.
    Layer {
        opacity: f32,
        filter: Option<(&'static FilterDef, Vec<f64>)>,
        span: Span,
        children: Vec<Node>,
    },
}
