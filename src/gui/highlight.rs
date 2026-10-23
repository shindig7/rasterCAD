//! Syntax highlighting for the editor, driven by the language's own lexer.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};
use rastercad::lang::eval::BUILTIN_MODULES;
use rastercad::lang::lexer::{Tok, lex};
use rastercad::render::filters::FILTERS;

struct Palette {
    plain: Color32,
    comment: Color32,
    keyword: Color32,
    number: Color32,
    string: Color32,
    builtin: Color32,
}

impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                plain: Color32::from_rgb(0xd4, 0xd4, 0xd4),
                comment: Color32::from_rgb(0x6a, 0x99, 0x55),
                keyword: Color32::from_rgb(0xc5, 0x86, 0xc0),
                number: Color32::from_rgb(0xb5, 0xce, 0xa8),
                string: Color32::from_rgb(0xce, 0x91, 0x78),
                builtin: Color32::from_rgb(0x4e, 0xc9, 0xb0),
            }
        } else {
            Self {
                plain: Color32::from_rgb(0x1f, 0x1f, 0x1f),
                comment: Color32::from_rgb(0x00, 0x80, 0x00),
                keyword: Color32::from_rgb(0xaf, 0x00, 0xdb),
                number: Color32::from_rgb(0x09, 0x86, 0x58),
                string: Color32::from_rgb(0xa3, 0x15, 0x15),
                builtin: Color32::from_rgb(0x00, 0x70, 0xc1),
            }
        }
    }
}

pub fn highlight(src: &str, dark: bool, font: FontId) -> LayoutJob {
    let pal = Palette::new(dark);
    let mut job = LayoutJob::default();
    let mut append = |text: &str, color: Color32| {
        job.append(text, 0.0, TextFormat::simple(font.clone(), color));
    };
    let (tokens, _) = lex(src, 0); // on a lex error the rest is shown unhighlighted
    let mut pos = 0;
    for t in tokens {
        let (start, end) = (t.span.start as usize, t.span.end as usize);
        if start > pos {
            append(&src[pos..start], pal.plain);
        }
        let text = &src[start..end];
        let color = match &t.tok {
            Tok::Comment => pal.comment,
            Tok::Num(_) => pal.number,
            Tok::Str(_) | Tok::Path(_) => pal.string,
            Tok::Ident(name)
                if BUILTIN_MODULES.contains(&name.as_str())
                    || FILTERS.iter().any(|f| f.name == name) =>
            {
                pal.builtin
            }
            Tok::Module
            | Tok::Function
            | Tok::If
            | Tok::Else
            | Tok::For
            | Tok::Let
            | Tok::Each
            | Tok::Use
            | Tok::Include
            | Tok::True
            | Tok::False
            | Tok::Undef => pal.keyword,
            _ => pal.plain,
        };
        append(text, color);
        pos = end;
    }
    if pos < src.len() {
        append(&src[pos..], pal.plain);
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_all_text() {
        for src in ["x = 1; // c\nrect(5);", "a = \"unterminated", "", "  \n"] {
            let job = highlight(src, true, FontId::monospace(14.0));
            assert_eq!(job.text, src);
        }
    }
}
