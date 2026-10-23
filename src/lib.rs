//! rasterCAD: OpenSCAD-style scripts that render to raster images.

pub mod diag;
pub mod lang;
pub mod render;
pub mod scene;

use std::path::{Path, PathBuf};

pub use diag::Diagnostic;
pub use tiny_skia::Pixmap;

/// Stack for the evaluation thread, sized so `lang::eval::MAX_DEPTH` nested calls fit even in
/// debug builds. It is reserved address space; only the pages actually used are committed.
const EVAL_STACK_BYTES: usize = 1 << 30;

pub struct RenderOutput {
    pub image: Result<Pixmap, Diagnostic>,
    /// Lines printed by `echo()`, also available when rendering fails.
    pub log: Vec<String>,
}

/// Evaluates and renders a script. `path` is the script's location, used to resolve libraries,
/// images and fonts relative to it (the current directory when `None`).
pub fn render(source: &str, path: Option<&Path>) -> RenderOutput {
    let source = source.to_owned();
    let path: Option<PathBuf> = path.map(Path::to_path_buf);
    let worker = std::thread::Builder::new()
        .name("rastercad-eval".into())
        .stack_size(EVAL_STACK_BYTES)
        .spawn(move || {
            let mut ev = lang::eval::Evaluator::new();
            let image = ev
                .run(&source, path.as_deref())
                .and_then(|(canvas, nodes)| render::render(&canvas, &nodes))
                .map_err(|e| ev.sources.diagnostic(&e));
            RenderOutput {
                image,
                log: std::mem::take(&mut ev.log),
            }
        });
    let internal = |msg: String| RenderOutput {
        image: Err(Diagnostic {
            file: 0,
            path: None,
            line: 1,
            col: 1,
            msg: format!("internal error: {msg}"),
        }),
        log: Vec::new(),
    };
    match worker {
        Ok(handle) => handle.join().unwrap_or_else(|panic| {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "evaluation panicked".into());
            internal(msg)
        }),
        Err(e) => internal(format!("cannot start evaluation thread: {e}")),
    }
}
