//! Rasterizes a scene tree onto a pixmap.

pub mod filters;
pub mod text;

use tiny_skia::{
    Color, FillRule, FilterQuality, LineCap, LineJoin, Paint, Pixmap, PixmapPaint, Stroke,
    Transform,
};

use crate::diag::{Error, Result};
use crate::scene::{Canvas, Node};

#[derive(Clone, Copy)]
struct Ctx {
    transform: Transform,
    color: Color,
    stroke: Option<f32>,
}

pub fn render(canvas: &Canvas, nodes: &[Node]) -> Result<Pixmap> {
    let mut pixmap = Pixmap::new(canvas.width, canvas.height).ok_or_else(|| {
        let msg = format!(
            "cannot allocate a {}x{} canvas",
            canvas.width, canvas.height
        );
        Error::new(canvas.span, msg)
    })?;
    pixmap.fill(canvas.background);
    let ctx = Ctx {
        transform: Transform::identity(),
        color: Color::BLACK,
        stroke: None,
    };
    render_nodes(nodes, &mut pixmap, ctx)?;
    Ok(pixmap)
}

fn render_nodes(nodes: &[Node], target: &mut Pixmap, ctx: Ctx) -> Result<()> {
    nodes.iter().try_for_each(|n| render_node(n, target, ctx))
}

fn render_node(node: &Node, target: &mut Pixmap, ctx: Ctx) -> Result<()> {
    match node {
        Node::Shape(path) => {
            let mut paint = Paint::default();
            paint.set_color(ctx.color);
            match ctx.stroke {
                Some(width) => {
                    let stroke = Stroke {
                        width,
                        line_cap: LineCap::Round,
                        line_join: LineJoin::Round,
                        ..Stroke::default()
                    };
                    target.stroke_path(path, &paint, &stroke, ctx.transform, None);
                }
                None => target.fill_path(path, &paint, FillRule::Winding, ctx.transform, None),
            }
        }
        Node::Image(image) => {
            let paint = PixmapPaint {
                quality: FilterQuality::Bicubic,
                ..PixmapPaint::default()
            };
            target.draw_pixmap(0, 0, image.as_ref().as_ref(), &paint, ctx.transform, None);
        }
        Node::Transform(t, children) => {
            let ctx = Ctx {
                transform: ctx.transform.pre_concat(*t),
                ..ctx
            };
            render_nodes(children, target, ctx)?;
        }
        Node::Color(color, children) => render_nodes(
            children,
            target,
            Ctx {
                color: *color,
                ..ctx
            },
        )?,
        Node::Stroke(width, children) => {
            render_nodes(
                children,
                target,
                Ctx {
                    stroke: Some(*width),
                    ..ctx
                },
            )?;
        }
        Node::Layer {
            opacity,
            filter,
            span,
            children,
        } => {
            let mut layer = Pixmap::new(target.width(), target.height())
                .ok_or_else(|| Error::new(*span, "cannot allocate layer"))?;
            render_nodes(children, &mut layer, ctx)?;
            if let Some((def, args)) = filter {
                (def.apply)(&mut layer, args).map_err(|msg| Error::new(*span, msg))?;
            }
            let paint = PixmapPaint {
                opacity: *opacity,
                ..PixmapPaint::default()
            };
            target.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::identity(), None);
        }
    }
    Ok(())
}
