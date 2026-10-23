//! The language guide (docs/GUIDE.md) in a searchable side panel.
//!
//! shortcut: renders only the Markdown subset the guide uses (## sections, ``` code, tables,
//! bullets, `inline code`); extend `markdown` if the guide starts using more.

use eframe::egui::{self, CollapsingHeader, FontId, RichText, ScrollArea, TextEdit};

use super::highlight;

const GUIDE: &str = include_str!("../../docs/GUIDE.md");

/// The guide's `## ` sections as (title, body).
fn sections() -> impl Iterator<Item = (&'static str, &'static str)> {
    GUIDE
        .split("\n## ")
        .skip(1)
        .map(|s| s.split_once('\n').unwrap_or((s, "")))
}

pub fn show(ui: &mut egui::Ui, query: &mut String) {
    ui.add_space(4.0);
    ui.add(
        TextEdit::singleline(query)
            .hint_text("Search the guide…")
            .desired_width(f32::INFINITY),
    );
    let q = query.trim().to_lowercase();
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let mut found = false;
        for (title, body) in sections() {
            let matches = q.is_empty()
                || title.to_lowercase().contains(&q)
                || body.to_lowercase().contains(&q);
            if !matches {
                continue;
            }
            found = true;
            CollapsingHeader::new(RichText::new(title).strong())
                .default_open(title == "Basics")
                .open((!q.is_empty()).then_some(true))
                .show(ui, |ui| markdown(ui, body));
        }
        if !found {
            ui.weak("No matches");
        }
    });
}

fn markdown(ui: &mut egui::Ui, text: &str) {
    let mut code: Option<Vec<&str>> = None;
    let mut in_table = false;
    for line in text.lines() {
        if line.starts_with("```") {
            match code.take() {
                Some(lines) => code_block(ui, &lines.join("\n")),
                None => code = Some(Vec::new()),
            }
            continue;
        }
        if let Some(lines) = &mut code {
            lines.push(line);
            continue;
        }
        let Some(row) = line.strip_prefix('|') else {
            in_table = false;
            match line.strip_prefix("- ") {
                _ if line.trim().is_empty() => ui.add_space(4.0),
                Some(item) => inline(ui, &format!("•  {item}")),
                None => inline(ui, line),
            }
            continue;
        };
        // The first row is the header and the second the |---| separator; both are skipped.
        if !in_table || row.starts_with("---") {
            in_table = true;
            continue;
        }
        let row = row.replace("\\|", "\0");
        let cells: Vec<String> = row
            .trim_end_matches('|')
            .split('|')
            .map(|c| c.trim().replace('\0', "|"))
            .collect();
        ui.add_space(2.0);
        inline(ui, &cells.join("  —  "));
    }
}

/// A line of text where `backticked` spans are shown as code.
fn inline(ui: &mut egui::Ui, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, part) in text.split('`').enumerate() {
            if i % 2 == 1 {
                ui.label(RichText::new(part).code());
            } else if !part.is_empty() {
                ui.label(part);
            }
        }
    });
}

/// A highlighted, selectable (read-only) code block.
fn code_block(ui: &mut egui::Ui, mut code: &str) {
    let dark = ui.visuals().dark_mode;
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = highlight::highlight(buf.as_str(), dark, FontId::monospace(13.0));
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    ui.add(
        TextEdit::multiline(&mut code)
            .code_editor()
            .desired_rows(1)
            .desired_width(f32::INFINITY)
            .layouter(&mut layouter),
    );
}

#[cfg(test)]
mod tests {
    use super::{GUIDE, sections};

    #[test]
    fn guide_sections_and_examples_parse() {
        assert!(sections().count() > 5);
        // Odd pieces between ``` fences are code blocks.
        let blocks: Vec<&str> = GUIDE.split("```\n").skip(1).step_by(2).collect();
        assert!(
            GUIDE.matches("```").count().is_multiple_of(2),
            "unclosed code block"
        );
        assert!(blocks.len() > 5);
        for code in blocks {
            if let Err(e) = rastercad::lang::parser::parse(code, 0) {
                panic!("guide example does not parse ({}):\n{code}", e.msg);
            }
        }
    }
}
