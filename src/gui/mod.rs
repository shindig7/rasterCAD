//! Desktop editor: code on the left, live preview on the right, messages at the bottom.

mod guide;
mod highlight;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{
    self, Align2, CentralPanel, Color32, FontId, Id, Key, KeyboardShortcut, Modifiers, Panel, Pos2,
    Rect, RichText, ScrollArea, Sense, TextEdit, TextureHandle, TextureOptions, Vec2,
    ViewportCommand,
    text::{CCursor, CCursorRange},
    widgets::text_edit::TextEditState,
};
use rastercad::{Diagnostic, Pixmap, RenderOutput};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};

const NEW_FILE: &str = r#"canvas(400, 300, bg="white");

color("steelblue") translate([200, 150]) circle(80);
"#;
const EDITOR_FONT_SIZE: f32 = 14.0;
const EDITOR_ID: &str = "code_editor";
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "tga", "ico", "qoi",
];
const SHORTCUT_NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const SHORTCUT_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SHORTCUT_SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SHORTCUT_SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const SHORTCUT_EXPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::E);
const SHORTCUT_RENDER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
const SHORTCUT_GUIDE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F1);

pub fn run(path: Option<PathBuf>) -> ExitCode {
    let (code, startup_error) = match &path {
        Some(p) if p.exists() => match std::fs::read_to_string(p) {
            Ok(code) => (code, None),
            Err(e) => (
                String::new(),
                Some(format!("cannot read {}: {e}", p.display())),
            ),
        },
        // A path that does not exist yet becomes a new file saved there.
        _ => (NEW_FILE.to_string(), None),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("rasterCAD"),
        ..Default::default()
    };
    let result = eframe::run_native(
        "rasterCAD",
        options,
        Box::new(move |cc| {
            let mut app = App::new(path, code);
            app.status = startup_error.unwrap_or_default();
            app.start_render(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Zoom and pan of the preview.
struct View {
    zoom: f32,
    pan: Vec2,
    fit_requested: bool,
}

struct Preview {
    texture: TextureHandle,
    pixmap: Pixmap,
}

struct App {
    path: Option<PathBuf>,
    code: String,
    saved_code: String,
    preview: Option<Preview>,
    error: Option<Diagnostic>,
    log: Vec<String>,
    status: String,
    in_flight: Option<Receiver<RenderOutput>>,
    rerender_queued: bool,
    view: View,
    /// Character index to move the editor cursor to on the next frame.
    jump_to: Option<usize>,
    title: String,
    close_confirmed: bool,
    guide_open: bool,
    guide_query: String,
}

impl App {
    fn new(path: Option<PathBuf>, code: String) -> Self {
        let saved_code = if path.as_deref().is_some_and(Path::exists) {
            code.clone()
        } else {
            String::new()
        };
        Self {
            path,
            code,
            saved_code,
            preview: None,
            error: None,
            log: Vec::new(),
            status: String::new(),
            in_flight: None,
            rerender_queued: false,
            view: View {
                zoom: 1.0,
                pan: Vec2::ZERO,
                fit_requested: true,
            },
            jump_to: None,
            title: String::new(),
            close_confirmed: false,
            guide_open: false,
            guide_query: String::new(),
        }
    }

    /// Switches to another document, keeping the guide panel as it is.
    fn replace_document(&mut self, mut app: App) {
        app.guide_open = self.guide_open;
        app.guide_query = std::mem::take(&mut self.guide_query);
        *self = app;
    }

    fn dirty(&self) -> bool {
        self.code != self.saved_code
    }

    // ------------------------------------------------------------------ rendering

    /// Renders on a background thread so a slow script never freezes the UI.
    fn start_render(&mut self, ctx: &egui::Context) {
        if self.in_flight.is_some() {
            self.rerender_queued = true;
            return;
        }
        let (tx, rx) = channel();
        let (code, path, ctx) = (self.code.clone(), self.path.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(rastercad::render(&code, path.as_deref()));
            ctx.request_repaint();
        });
        self.in_flight = Some(rx);
        self.status = "Rendering…".into();
    }

    fn poll_render(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.in_flight else { return };
        let Ok(output) = rx.try_recv() else { return };
        self.in_flight = None;
        self.log = output.log;
        match output.image {
            Ok(pixmap) => {
                let size = [pixmap.width() as usize, pixmap.height() as usize];
                let image = egui::ColorImage::from_rgba_premultiplied(size, pixmap.data());
                let options = TextureOptions {
                    magnification: egui::TextureFilter::Nearest, // inspect pixels when zoomed in
                    minification: egui::TextureFilter::Linear,
                    ..TextureOptions::default()
                };
                let size_changed = self.preview.as_ref().is_none_or(|p| {
                    p.pixmap.width() != pixmap.width() || p.pixmap.height() != pixmap.height()
                });
                self.view.fit_requested |= size_changed;
                self.preview = Some(Preview {
                    texture: ctx.load_texture("preview", image, options),
                    pixmap,
                });
                self.error = None;
                self.status = format!("Rendered {}×{}", size[0], size[1]);
            }
            Err(diag) => {
                self.status = "Render failed".into();
                self.error = Some(diag);
            }
        }
        if std::mem::take(&mut self.rerender_queued) {
            self.start_render(ctx);
        }
    }

    // ------------------------------------------------------------------ files

    /// Asks before throwing away unsaved edits. Returns true if it is fine to proceed.
    fn confirm_discard(&self) -> bool {
        !self.dirty()
            || MessageDialog::new()
                .set_title("Unsaved changes")
                .set_description("Discard unsaved changes?")
                .set_buttons(MessageButtons::YesNo)
                .show()
                == MessageDialogResult::Yes
    }

    fn file_dialog(&self) -> FileDialog {
        let dialog = FileDialog::new().add_filter("rasterCAD script", &["rcad"]);
        match self.path.as_deref().and_then(Path::parent) {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        }
    }

    fn new_file(&mut self, ctx: &egui::Context) {
        if self.confirm_discard() {
            self.replace_document(App::new(None, NEW_FILE.to_string()));
            self.start_render(ctx);
        }
    }

    fn open(&mut self, ctx: &egui::Context) {
        if !self.confirm_discard() {
            return;
        }
        let Some(path) = self.file_dialog().pick_file() else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(code) => {
                self.replace_document(App::new(Some(path), code));
                self.start_render(ctx);
            }
            Err(e) => self.status = format!("Cannot open {}: {e}", path.display()),
        }
    }

    fn save(&mut self, ctx: &egui::Context, choose_path: bool) {
        let path = match (&self.path, choose_path) {
            (Some(p), false) => p.clone(),
            _ => {
                let name = self
                    .path
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|n| n.to_string_lossy().into_owned());
                let Some(mut p) = self
                    .file_dialog()
                    .set_file_name(name.unwrap_or_else(|| "untitled.rcad".into()))
                    .save_file()
                else {
                    return;
                };
                if p.extension().is_none() {
                    p.set_extension("rcad");
                }
                p
            }
        };
        match std::fs::write(&path, &self.code) {
            Ok(()) => {
                self.saved_code = self.code.clone();
                self.path = Some(path);
                self.start_render(ctx);
            }
            Err(e) => self.status = format!("Cannot save {}: {e}", path.display()),
        }
    }

    /// Picks an image file and inserts an `image("...")` statement at the editor cursor.
    fn insert_image(&mut self, ctx: &egui::Context) {
        let Some(file) = FileDialog::new()
            .add_filter("Images", IMAGE_EXTENSIONS)
            .set_directory("/home")
            .pick_file()
        else {
            return;
        };
        let Some(path) = file.to_str() else {
            self.status = format!("Cannot insert {}: path is not valid UTF-8", file.display());
            return;
        };
        self.insert_at_cursor(ctx, format!("image({});", string_literal(path)));
    }

    /// Inserts `snippet` at the editor cursor and re-renders.
    fn insert_at_cursor(&mut self, ctx: &egui::Context, snippet: String) {
        // Without a cursor (the editor was never focused), append on a new line.
        let cursor = TextEditState::load(ctx, Id::new(EDITOR_ID))
            .and_then(|s| s.cursor.char_range())
            .map(|r| r.primary.index.0);
        let (at, snippet) = match cursor {
            Some(at) => (at, snippet),
            None if self.code.is_empty() || self.code.ends_with('\n') => {
                (self.code.chars().count(), snippet + "\n")
            }
            None => (self.code.chars().count(), format!("\n{snippet}\n")),
        };
        self.code.insert_str(byte_index(&self.code, at), &snippet);
        self.jump_to = Some(at + snippet.chars().count());
        self.start_render(ctx);
    }

    fn export_png(&mut self) {
        let Some(preview) = &self.preview else {
            self.status = "Nothing rendered to export".into();
            return;
        };
        let stem = self
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().into_owned());
        let dialog = FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(format!("{}.png", stem.as_deref().unwrap_or("untitled")));
        let dialog = match self.path.as_deref().and_then(Path::parent) {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        };
        let Some(mut out) = dialog.save_file() else {
            return;
        };
        if out.extension().is_none() {
            out.set_extension("png");
        }
        self.status = match preview.pixmap.save_png(&out) {
            Ok(()) => format!("Exported {}", out.display()),
            Err(e) => format!("Cannot export {}: {e}", out.display()),
        };
    }

    // ------------------------------------------------------------------ UI

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Check Save As before Save: consume_shortcut ignores extra modifiers.
        let pressed = |s: &KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(s));
        if pressed(&SHORTCUT_SAVE_AS) {
            self.save(ctx, true);
        } else if pressed(&SHORTCUT_SAVE) {
            self.save(ctx, false);
        }
        if pressed(&SHORTCUT_NEW) {
            self.new_file(ctx);
        }
        if pressed(&SHORTCUT_OPEN) {
            self.open(ctx);
        }
        if pressed(&SHORTCUT_EXPORT) {
            self.export_png();
        }
        if pressed(&SHORTCUT_RENDER) {
            self.start_render(ctx);
        }
        if pressed(&SHORTCUT_GUIDE) {
            self.guide_open = !self.guide_open;
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_confirmed {
            if self.confirm_discard() {
                self.close_confirmed = true;
            } else {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| "untitled".into(), |n| n.to_string_lossy().into_owned());
        let title = format!("{}{name} — rasterCAD", if self.dirty() { "• " } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui
                    .add(egui::Button::new("New").shortcut_text(ctx.format_shortcut(&SHORTCUT_NEW)))
                    .clicked()
                {
                    self.new_file(&ctx);
                }
                if ui
                    .add(
                        egui::Button::new("Open…")
                            .shortcut_text(ctx.format_shortcut(&SHORTCUT_OPEN)),
                    )
                    .clicked()
                {
                    self.open(&ctx);
                }
                if ui
                    .add(
                        egui::Button::new("Save")
                            .shortcut_text(ctx.format_shortcut(&SHORTCUT_SAVE)),
                    )
                    .clicked()
                {
                    self.save(&ctx, false);
                }
                if ui
                    .add(
                        egui::Button::new("Save As…")
                            .shortcut_text(ctx.format_shortcut(&SHORTCUT_SAVE_AS)),
                    )
                    .clicked()
                {
                    self.save(&ctx, true);
                }
                ui.separator();
                if ui
                    .add(
                        egui::Button::new("Export PNG…")
                            .shortcut_text(ctx.format_shortcut(&SHORTCUT_EXPORT)),
                    )
                    .clicked()
                {
                    self.export_png();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            });
            ui.menu_button("View", |ui| {
                if ui
                    .add(
                        egui::Button::new("Render")
                            .shortcut_text(ctx.format_shortcut(&SHORTCUT_RENDER)),
                    )
                    .clicked()
                {
                    self.start_render(&ctx);
                }
                if ui.button("Fit to window").clicked() {
                    self.view.fit_requested = true;
                }
                if ui.button("Actual size (100%)").clicked() {
                    self.view.zoom = 1.0;
                    self.view.pan = Vec2::ZERO;
                }
            });
            ui.menu_button("Insert", |ui| {
                if ui.button("Image…").clicked() {
                    self.insert_image(&ctx);
                }
                ui.menu_button("Font", |ui| {
                    let fonts = rastercad::render::text::user_fonts();
                    if fonts.is_empty() {
                        let dir = rastercad::render::text::user_font_dir().map_or_else(
                            || "the user font folder".into(),
                            |d| d.display().to_string(),
                        );
                        ui.weak(format!("No fonts in {dir}"));
                    }
                    for spec in fonts {
                        if ui.button(&spec).clicked() {
                            self.insert_at_cursor(&ctx, format!("font={}", string_literal(&spec)));
                        }
                    }
                });
            });
            ui.menu_button("Help", |ui| {
                if ui
                    .add(
                        egui::Button::selectable(self.guide_open, "Language guide")
                            .shortcut_text(ctx.format_shortcut(&SHORTCUT_GUIDE)),
                    )
                    .clicked()
                {
                    self.guide_open = !self.guide_open;
                }
            });
        });
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        let editor_id = Id::new(EDITOR_ID);
        if let Some(idx) = self.jump_to.take() {
            let mut state = TextEditState::load(ui.ctx(), editor_id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(idx))));
            state.store(ui.ctx(), editor_id);
            ui.ctx().memory_mut(|m| m.request_focus(editor_id));
        }
        let font = FontId::monospace(EDITOR_FONT_SIZE);
        let dark = ui.visuals().dark_mode;
        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap_width: f32| {
            let job = highlight::highlight(buf.as_str(), dark, font.clone());
            ui.fonts_mut(|f| f.layout_job(job))
        };
        ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let lines = self.code.split('\n').count();
                let numbers = (1..=lines)
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.add_space(2.0); // matches the text edit's inner margin
                    ui.label(
                        RichText::new(numbers)
                            .font(FontId::monospace(EDITOR_FONT_SIZE))
                            .weak(),
                    );
                });
                TextEdit::multiline(&mut self.code)
                    .id(editor_id)
                    .code_editor()
                    .lock_focus(true)
                    .desired_width(f32::INFINITY)
                    .desired_rows(40)
                    .frame(egui::Frame::NONE)
                    .layouter(&mut layouter)
                    .show(ui);
            });
        });
    }

    fn messages(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if let Some(diag) = &self.error {
                    let text = RichText::new(diag.to_string())
                        .color(ui.visuals().error_fg_color)
                        .monospace();
                    if diag.file == 0 {
                        let jump = ui
                            .add(egui::Label::new(text).sense(Sense::click()))
                            .on_hover_text("Click to jump to the error");
                        if jump.clicked() {
                            self.jump_to = Some(char_index(&self.code, diag.line, diag.col));
                        }
                    } else {
                        ui.label(text);
                    }
                }
                for line in &self.log {
                    ui.label(RichText::new(line).monospace());
                }
            });
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        let Some(preview) = &self.preview else {
            let msg = if self.in_flight.is_some() {
                "Rendering…"
            } else {
                "Nothing rendered yet"
            };
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                msg,
                FontId::proportional(16.0),
                ui.visuals().weak_text_color(),
            );
            return;
        };
        let img = Vec2::new(
            preview.pixmap.width() as f32,
            preview.pixmap.height() as f32,
        );
        let view = &mut self.view;
        if view.fit_requested {
            let margin = 16.0;
            view.zoom = ((rect.width() - margin) / img.x)
                .min((rect.height() - margin) / img.y)
                .clamp(0.01, 1.0);
            view.pan = Vec2::ZERO;
            view.fit_requested = false;
        }

        // Zoom around the cursor with the mouse wheel (or pinch); pan by dragging.
        if let Some(hover) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta().y, i.zoom_delta()));
            let factor = (scroll * 0.002).exp() * pinch;
            if factor != 1.0 {
                let new_zoom = (view.zoom * factor).clamp(0.01, 64.0);
                let anchor = hover - rect.center() - view.pan;
                view.pan -= anchor * (new_zoom / view.zoom - 1.0);
                view.zoom = new_zoom;
            }
        }
        view.pan += response.drag_delta();

        let origin = rect.center() + view.pan - img * view.zoom / 2.0;
        let img_rect = Rect::from_min_size(origin, img * view.zoom);
        paint_checkerboard(&painter, img_rect.intersect(rect));
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(preview.texture.id(), img_rect, uv, Color32::WHITE);

        // Canvas coordinates and color under the cursor.
        let mut info = format!("{:.0}%", view.zoom * 100.0);
        if let Some(hover) = response.hover_pos() {
            let p = (hover - origin) / view.zoom;
            if p.x >= 0.0 && p.y >= 0.0 && p.x < img.x && p.y < img.y {
                let (x, y) = (p.x.floor() as u32, p.y.floor() as u32);
                if let Some(c) = preview.pixmap.pixel(x, y) {
                    let c = c.demultiply();
                    info = format!(
                        "x {x}, y {y}   #{:02x}{:02x}{:02x}{:02x}   {info}",
                        c.red(),
                        c.green(),
                        c.blue(),
                        c.alpha()
                    );
                }
            }
        }
        let font = FontId::monospace(12.0);
        let pos = rect.right_bottom() - Vec2::new(8.0, 6.0);
        let bg = painter
            .layout_no_wrap(info.clone(), font.clone(), Color32::WHITE)
            .rect;
        painter.rect_filled(
            Rect::from_min_size(
                pos - bg.size() - Vec2::splat(4.0),
                bg.size() + Vec2::splat(8.0),
            ),
            4.0,
            Color32::from_black_alpha(160),
        );
        painter.text(pos, Align2::RIGHT_BOTTOM, info, font, Color32::WHITE);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_render(&ctx);
        self.handle_shortcuts(&ctx);
        self.handle_close(&ctx);
        self.update_title(&ctx);

        Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.in_flight.is_some() {
                    ui.spinner();
                }
                ui.label(&self.status);
            });
        });
        Panel::left("editor")
            .resizable(true)
            .default_size(560.0)
            .show(ui, |ui| self.editor(ui));
        if self.guide_open {
            Panel::right("guide")
                .resizable(true)
                .default_size(420.0)
                .show(ui, |ui| guide::show(ui, &mut self.guide_query));
        }
        Panel::bottom("messages")
            .resizable(true)
            .default_size(120.0)
            .show(ui, |ui| self.messages(ui));
        CentralPanel::no_frame().show(ui, |ui| self.preview(ui));
    }
}

fn paint_checkerboard(painter: &egui::Painter, area: Rect) {
    const CELL: f32 = 10.0;
    painter.rect_filled(area, 0.0, Color32::from_gray(204));
    let (cols, rows) = (
        (area.width() / CELL).ceil() as i32,
        (area.height() / CELL).ceil() as i32,
    );
    for row in 0..rows {
        for col in (row % 2..cols).step_by(2) {
            let min = area.min + Vec2::new(col as f32, row as f32) * CELL;
            let cell = Rect::from_min_size(min, Vec2::splat(CELL)).intersect(area);
            painter.rect_filled(cell, 0.0, Color32::from_gray(255));
        }
    }
}

/// Converts a 1-based line and column (in characters) into a character index into `text`.
fn char_index(text: &str, line: usize, col: usize) -> usize {
    let mut idx = 0;
    for (i, l) in text.split('\n').enumerate() {
        let len = l.chars().count();
        if i + 1 == line {
            return idx + (col - 1).min(len);
        }
        idx += len + 1;
    }
    text.chars().count()
}

/// `s` as a script string literal.
fn string_literal(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Converts a character index into a byte index into `text`, clamped to its end.
fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(b, _)| b)
}

#[cfg(test)]
mod tests {
    use super::{byte_index, char_index};

    #[test]
    fn byte_index_of_char() {
        assert_eq!(byte_index("aé b", 2), 3);
        assert_eq!(byte_index("ab", 9), 2);
    }

    #[test]
    fn char_index_of_line_col() {
        assert_eq!(char_index("ab\ncdé\nf", 2, 3), 5);
        assert_eq!(char_index("ab", 9, 1), 2);
        assert_eq!(char_index("ab", 1, 99), 2);
    }
}
