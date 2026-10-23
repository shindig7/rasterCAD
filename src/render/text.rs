//! Text to vector outlines. Glyphs become paths, so text stays sharp under any transform and
//! works with `stroke()`.

use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use cosmic_text::fontdb::{ID, Source};
use cosmic_text::{
    Attrs, Buffer, CacheKey, CacheKeyFlags, Command, Family, FontSystem, Metrics, Shaping, Style,
    SwashCache, Weight,
};
use tiny_skia::{Path, PathBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Center,
    Baseline,
    Bottom,
}

pub struct TextStyle<'a> {
    pub size: f32,
    /// `"Family"`, `"Family:style=Bold Italic"`, or a path to a .ttf/.otf file. The generic
    /// families `sans-serif`, `serif` and `monospace` are also accepted.
    pub font: &'a str,
    pub halign: HAlign,
    pub valign: VAlign,
}

struct Fonts {
    system: FontSystem,
    cache: SwashCache,
    /// Font files loaded so far (by path), so each is added to the database only once.
    files: HashMap<PathBuf, Vec<ID>>,
}

impl Fonts {
    fn load_file(&mut self, path: &FsPath) -> Result<&[ID], String> {
        if !self.files.contains_key(path) {
            let data = std::fs::read(path)
                .map_err(|e| format!("cannot read font file '{}': {e}", path.display()))?;
            let ids = self
                .system
                .db_mut()
                .load_font_source(Source::Binary(Arc::new(data)));
            self.files.insert(path.to_path_buf(), ids.to_vec());
        }
        Ok(&self.files[path])
    }

    /// Loads files added to the user font folder since the last call, and returns the IDs of
    /// every face in it. Unreadable or invalid files are skipped.
    fn load_user_fonts(&mut self) -> Vec<ID> {
        let Some(entries) = user_font_dir().and_then(|d| std::fs::read_dir(d).ok()) else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        for path in entries.flatten().map(|e| e.path()) {
            if is_font_file(&path)
                && let Ok(loaded) = self.load_file(&path)
            {
                ids.extend_from_slice(loaded);
            }
        }
        ids
    }
}

/// Loading system fonts is slow, so it happens once per process.
static FONTS: LazyLock<Mutex<Fonts>> = LazyLock::new(|| {
    Mutex::new(Fonts {
        system: FontSystem::new(),
        cache: SwashCache::new(),
        files: HashMap::new(),
    })
});

/// `$XDG_CONFIG_HOME/rasterCAD/fonts`, by default `~/.config/rasterCAD/fonts`. Fonts there can
/// be used by family name or by file name from any script.
pub fn user_font_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("rasterCAD/fonts"))
}

fn is_font_file(path: &FsPath) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["ttf", "otf", "ttc", "otc"].contains(&e.to_ascii_lowercase().as_str()))
}

/// The faces in the user font folder as sorted `font=` specs, e.g. `"Inter:style=Bold Italic"`.
pub fn user_fonts() -> Vec<String> {
    let mut guard = FONTS.lock().unwrap_or_else(|e| e.into_inner());
    let ids = guard.load_user_fonts();
    let db = guard.system.db();
    let mut specs: Vec<String> = ids
        .into_iter()
        .filter_map(|id| db.face(id))
        .filter_map(|face| {
            let (family, _) = face.families.first()?;
            let mut words = Vec::new();
            if let Some(w) = weight_word(face.weight) {
                words.push(w);
            }
            match face.style {
                Style::Normal => {}
                Style::Italic => words.push("Italic"),
                Style::Oblique => words.push("Oblique"),
            }
            Some(if words.is_empty() {
                family.clone()
            } else {
                format!("{family}:style={}", words.join(" "))
            })
        })
        .collect();
    specs.sort();
    specs.dedup();
    specs
}

/// The style word for a weight (`None` for regular), matching what `resolve_font` accepts.
fn weight_word(weight: Weight) -> Option<&'static str> {
    Some(match weight.0 {
        0..=150 => "Thin",
        151..=350 => "Light",
        351..=450 => return None,
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        _ => "Black",
    })
}

/// Lays out `text` (which may contain newlines, and `{-}` for an em dash) and returns its
/// outline, or `None` if it has no visible glyphs. `base_dir` resolves relative font file paths.
pub fn text_path(text: &str, style: &TextStyle, base_dir: &FsPath) -> Result<Option<Path>, String> {
    if !(style.size > 0.0 && style.size.is_finite()) {
        return Err(format!("text size must be positive, got {}", style.size));
    }
    let mut guard = FONTS.lock().unwrap_or_else(|e| e.into_inner());
    guard.load_user_fonts();
    let (family, weight, font_style) = resolve_font(&mut guard, style.font, base_dir)?;
    let Fonts {
        system: fonts,
        cache,
        ..
    } = &mut *guard;
    let family = match family.as_str() {
        "sans-serif" => Family::SansSerif,
        "serif" => Family::Serif,
        "monospace" => Family::Monospace,
        name => Family::Name(name),
    };

    let mut buffer = Buffer::new_empty(Metrics::new(style.size, style.size * 1.2));
    buffer.set_size(None, None);
    let attrs = Attrs::new()
        .family(family)
        .weight(weight)
        .style(font_style)
        .cache_key_flags(CacheKeyFlags::DISABLE_HINTING);
    buffer.set_text(&text.replace("{-}", "—"), &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);

    let runs: Vec<_> = buffer.layout_runs().collect();
    let (Some(first), Some(last)) = (runs.first(), runs.last()) else {
        return Ok(None);
    };
    let total_height = last.line_top + last.line_height;
    let dy = match style.valign {
        VAlign::Top => -first.line_top,
        VAlign::Center => -total_height / 2.0,
        VAlign::Baseline => -first.line_y,
        VAlign::Bottom => -total_height,
    };

    let mut pb = PathBuilder::new();
    for run in &runs {
        let dx = match style.halign {
            HAlign::Left => 0.0,
            HAlign::Center => -run.line_w / 2.0,
            HAlign::Right => -run.line_w,
        };
        for glyph in run.glyphs {
            let (key, _, _) = CacheKey::new(
                glyph.font_id,
                glyph.glyph_id,
                glyph.font_size,
                (0.0, 0.0),
                glyph.font_weight,
                glyph.cache_key_flags,
            );
            let Some(commands) = cache.get_outline_commands(fonts, key) else {
                continue;
            };
            let ox = glyph.x + glyph.font_size * glyph.x_offset + dx;
            let oy = run.line_y + glyph.y - glyph.font_size * glyph.y_offset + dy;
            // Font outlines are y-up; the canvas is y-down.
            for cmd in commands {
                match *cmd {
                    Command::MoveTo(p) => pb.move_to(ox + p.x, oy - p.y),
                    Command::LineTo(p) => pb.line_to(ox + p.x, oy - p.y),
                    Command::QuadTo(c, p) => pb.quad_to(ox + c.x, oy - c.y, ox + p.x, oy - p.y),
                    Command::CurveTo(a, b, p) => {
                        pb.cubic_to(ox + a.x, oy - a.y, ox + b.x, oy - b.y, ox + p.x, oy - p.y);
                    }
                    Command::Close => pb.close(),
                }
            }
        }
    }
    Ok(pb.finish())
}

/// Parses a font spec into a family name known to the font database (or a generic family name),
/// plus weight and style.
fn resolve_font(
    fonts: &mut Fonts,
    spec: &str,
    base_dir: &FsPath,
) -> Result<(String, Weight, Style), String> {
    let (name, styles) = spec.split_once(':').unwrap_or((spec, ""));
    let name = name.trim();

    let family = if matches!(name, "sans-serif" | "serif" | "monospace") {
        name.to_string()
    } else if is_font_file(FsPath::new(name)) {
        // Relative to the script first, then the user font folder.
        let path = std::iter::once(base_dir.join(name))
            .chain(user_font_dir().map(|d| d.join(name)))
            .find(|p| p.exists())
            .unwrap_or_else(|| base_dir.join(name));
        let id = fonts.load_file(&path)?.first().copied();
        let face = id.and_then(|id| fonts.system.db().face(id));
        face.and_then(|f| f.families.first())
            .map(|(family, _)| family.clone())
            .ok_or_else(|| format!("'{}' is not a usable font file", path.display()))?
    } else {
        fonts
            .system
            .db()
            .faces()
            .flat_map(|f| &f.families)
            .find(|(family, _)| family.eq_ignore_ascii_case(name))
            .map(|(family, _)| family.clone())
            .ok_or_else(|| {
                format!(
                    "font '{name}' not found (run `fc-list : family` to see installed fonts, \
                     or add it to ~/.config/rasterCAD/fonts)"
                )
            })?
    };

    let mut weight = Weight::NORMAL;
    let mut style = Style::Normal;
    let styles = styles.trim();
    let styles = styles.strip_prefix("style=").unwrap_or(styles);
    for word in styles.split_whitespace() {
        match word.to_ascii_lowercase().as_str() {
            "thin" => weight = Weight::THIN,
            "light" => weight = Weight::LIGHT,
            "regular" | "normal" | "book" => weight = Weight::NORMAL,
            "medium" => weight = Weight::MEDIUM,
            "semibold" | "demibold" => weight = Weight::SEMIBOLD,
            "bold" => weight = Weight::BOLD,
            "extrabold" => weight = Weight::EXTRA_BOLD,
            "black" | "heavy" => weight = Weight::BLACK,
            "italic" => style = Style::Italic,
            "oblique" => style = Style::Oblique,
            other => return Err(format!("unknown font style '{other}'")),
        }
    }
    Ok((family, weight, style))
}
