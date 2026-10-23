//! Source locations and user-facing error reporting.

use std::path::{Path, PathBuf};

/// Index into a [`SourceMap`]. File 0 is always the main script.
pub type FileId = u32;

/// A byte range inside one source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: usize, end: usize) -> Self {
        Self {
            file,
            start: start as u32,
            end: end as u32,
        }
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// An internal error: a message attached to a location.
#[derive(Debug, Clone)]
pub struct Error {
    pub span: Span,
    pub msg: String,
}

impl Error {
    pub fn new(span: Span, msg: impl Into<String>) -> Self {
        Self {
            span,
            msg: msg.into(),
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// All source files loaded during one evaluation, so spans can be resolved to line/column.
#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<(Option<PathBuf>, String)>,
}

impl SourceMap {
    pub fn add(&mut self, path: Option<PathBuf>, text: String) -> FileId {
        self.files.push((path, text));
        (self.files.len() - 1) as FileId
    }

    pub fn path(&self, file: FileId) -> Option<&Path> {
        self.files.get(file as usize)?.0.as_deref()
    }

    pub fn text(&self, file: FileId) -> &str {
        self.files.get(file as usize).map_or("", |f| f.1.as_str())
    }

    pub fn diagnostic(&self, err: &Error) -> Diagnostic {
        let text = self.text(err.span.file);
        let offset = (err.span.start as usize).min(text.len());
        let before = &text[..offset];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        Diagnostic {
            file: err.span.file,
            path: self.path(err.span.file).map(Path::to_path_buf),
            line,
            col,
            msg: err.msg.clone(),
        }
    }
}

/// A resolved, displayable error. `file == 0` means the main script.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}:{line}:{col}: {msg}", path.as_deref().map_or("<script>".into(), |p| p.display().to_string()))]
pub struct Diagnostic {
    pub file: FileId,
    pub path: Option<PathBuf>,
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_and_column() {
        let mut sm = SourceMap::default();
        let f = sm.add(None, "ab\ncd\nef".into());
        let d = sm.diagnostic(&Error::new(Span::new(f, 4, 5), "x"));
        assert_eq!((d.line, d.col), (2, 2));
        assert_eq!(d.to_string(), "<script>:2:2: x");
    }
}
