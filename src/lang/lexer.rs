//! Tokenizer. Comments are kept as tokens so the editor can highlight them; the parser skips them.

use crate::diag::{Error, FileId, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    /// The `<path>` after `use` / `include`.
    Path(String),
    Comment,
    // Keywords
    Module,
    Function,
    If,
    Else,
    For,
    Let,
    Each,
    Use,
    Include,
    True,
    False,
    Undef,
    // Punctuation
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Colon,
    Assign,
    Question,
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Not,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    AndAnd,
    OrOr,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// Tokenizes `src`. On a lexing error, returns the tokens read so far alongside the error,
/// which lets the editor highlight a partially valid file.
pub fn lex(src: &str, file: FileId) -> (Vec<Token>, Option<Error>) {
    let mut lx = Lexer {
        src,
        bytes: src.as_bytes(),
        pos: 0,
        file,
        out: Vec::new(),
    };
    let err = lx.run().err();
    (lx.out, err)
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    file: FileId,
    out: Vec<Token>,
}

impl Lexer<'_> {
    fn peek(&self, ahead: usize) -> u8 {
        self.bytes.get(self.pos + ahead).copied().unwrap_or(0)
    }

    fn push(&mut self, tok: Tok, start: usize) {
        self.out.push(Token {
            tok,
            span: Span::new(self.file, start, self.pos),
        });
    }

    fn err(&self, start: usize, msg: impl Into<String>) -> Error {
        Error::new(Span::new(self.file, start, self.pos.max(start + 1)), msg)
    }

    fn skip_whitespace(&mut self) {
        while self.peek(0).is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn run(&mut self) -> Result<(), Error> {
        loop {
            self.skip_whitespace();
            let start = self.pos;
            let c = self.peek(0);
            if c == 0 && self.pos >= self.bytes.len() {
                self.push(Tok::Eof, start);
                return Ok(());
            }
            match c {
                b'/' if self.peek(1) == b'/' => {
                    while self.pos < self.bytes.len() && self.peek(0) != b'\n' {
                        self.pos += 1;
                    }
                    self.push(Tok::Comment, start);
                }
                b'/' if self.peek(1) == b'*' => {
                    match self.src[start + 2..].find("*/") {
                        Some(i) => self.pos = start + 2 + i + 2,
                        None => {
                            self.pos = self.bytes.len();
                            return Err(self.err(start, "unterminated block comment"));
                        }
                    }
                    self.push(Tok::Comment, start);
                }
                b'0'..=b'9' => self.number(start)?,
                b'.' if self.peek(1).is_ascii_digit() => self.number(start)?,
                b'"' => self.string(start)?,
                c if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => self.word(start)?,
                _ => self.punct(start)?,
            }
        }
    }

    fn number(&mut self, start: usize) -> Result<(), Error> {
        while self.peek(0).is_ascii_digit() {
            self.pos += 1;
        }
        if self.peek(0) == b'.' && self.peek(1).is_ascii_digit() {
            self.pos += 1;
            while self.peek(0).is_ascii_digit() {
                self.pos += 1;
            }
        }
        if matches!(self.peek(0), b'e' | b'E') {
            let sign = usize::from(matches!(self.peek(1), b'+' | b'-'));
            if self.peek(1 + sign).is_ascii_digit() {
                self.pos += 1 + sign;
                while self.peek(0).is_ascii_digit() {
                    self.pos += 1;
                }
            }
        }
        let text = &self.src[start..self.pos];
        let n = text
            .parse::<f64>()
            .map_err(|_| self.err(start, format!("invalid number '{text}'")))?;
        self.push(Tok::Num(n), start);
        Ok(())
    }

    fn string(&mut self, start: usize) -> Result<(), Error> {
        self.pos += 1; // opening quote
        let mut s = String::new();
        loop {
            let rest = &self.src[self.pos..];
            let Some(c) = rest.chars().next() else {
                return Err(self.err(start, "unterminated string"));
            };
            self.pos += c.len_utf8();
            match c {
                '"' => break,
                '\\' => {
                    let esc_start = self.pos - 1;
                    let Some(e) = self.src[self.pos..].chars().next() else {
                        return Err(self.err(start, "unterminated string"));
                    };
                    self.pos += e.len_utf8();
                    s.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' => '\\',
                        '"' => '"',
                        '\'' => '\'',
                        'u' => {
                            let hex = self.src.get(self.pos..self.pos + 4).unwrap_or("");
                            let ch = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
                            self.pos = (self.pos + 4).min(self.bytes.len());
                            ch.ok_or_else(|| {
                                self.err(esc_start, "expected \\uXXXX with 4 hex digits")
                            })?
                        }
                        other => {
                            return Err(self.err(esc_start, format!("unknown escape '\\{other}'")));
                        }
                    });
                }
                c => s.push(c),
            }
        }
        self.push(Tok::Str(s), start);
        Ok(())
    }

    fn word(&mut self, start: usize) -> Result<(), Error> {
        self.pos += 1;
        while self.peek(0).is_ascii_alphanumeric() || self.peek(0) == b'_' {
            self.pos += 1;
        }
        let tok = match &self.src[start..self.pos] {
            "module" => Tok::Module,
            "function" => Tok::Function,
            "if" => Tok::If,
            "else" => Tok::Else,
            "for" => Tok::For,
            "let" => Tok::Let,
            "each" => Tok::Each,
            "use" => Tok::Use,
            "include" => Tok::Include,
            "true" => Tok::True,
            "false" => Tok::False,
            "undef" => Tok::Undef,
            w => Tok::Ident(w.to_string()),
        };
        let is_import = matches!(tok, Tok::Use | Tok::Include);
        self.push(tok, start);
        if is_import {
            self.import_path()?;
        }
        Ok(())
    }

    /// Reads the `<path>` after `use`/`include`; `<` would otherwise lex as less-than.
    fn import_path(&mut self) -> Result<(), Error> {
        self.skip_whitespace();
        if self.peek(0) != b'<' {
            return Ok(()); // the parser reports the missing path
        }
        let start = self.pos;
        match self.src[start..].find(['>', '\n']) {
            Some(i) if self.bytes[start + i] == b'>' => {
                self.pos = start + i + 1;
                let path = self.src[start + 1..start + i].trim().to_string();
                self.push(Tok::Path(path), start);
                Ok(())
            }
            _ => Err(self.err(start, "expected '>' to close the library path")),
        }
    }

    fn punct(&mut self, start: usize) -> Result<(), Error> {
        let two = [self.peek(0), self.peek(1)];
        let (tok, len) = match &two {
            b"<=" => (Tok::Le, 2),
            b">=" => (Tok::Ge, 2),
            b"==" => (Tok::EqEq, 2),
            b"!=" => (Tok::Ne, 2),
            b"&&" => (Tok::AndAnd, 2),
            b"||" => (Tok::OrOr, 2),
            _ => (
                match two[0] {
                    b'(' => Tok::LParen,
                    b')' => Tok::RParen,
                    b'[' => Tok::LBracket,
                    b']' => Tok::RBracket,
                    b'{' => Tok::LBrace,
                    b'}' => Tok::RBrace,
                    b',' => Tok::Comma,
                    b';' => Tok::Semi,
                    b':' => Tok::Colon,
                    b'=' => Tok::Assign,
                    b'?' => Tok::Question,
                    b'.' => Tok::Dot,
                    b'+' => Tok::Plus,
                    b'-' => Tok::Minus,
                    b'*' => Tok::Star,
                    b'/' => Tok::Slash,
                    b'%' => Tok::Percent,
                    b'^' => Tok::Caret,
                    b'!' => Tok::Not,
                    b'<' => Tok::Lt,
                    b'>' => Tok::Gt,
                    _ => {
                        let ch = self.src[start..].chars().next().unwrap_or('?');
                        self.pos += ch.len_utf8();
                        return Err(self.err(start, format!("unexpected character '{ch}'")));
                    }
                },
                1,
            ),
        };
        self.pos += len;
        self.push(tok, start);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        let (t, err) = lex(src, 0);
        assert!(err.is_none(), "{err:?}");
        t.into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn basics() {
        assert_eq!(
            toks("x = 1.5e2; // hi\nuse <a/b.rcad>"),
            vec![
                Tok::Ident("x".into()),
                Tok::Assign,
                Tok::Num(150.0),
                Tok::Semi,
                Tok::Comment,
                Tok::Use,
                Tok::Path("a/b.rcad".into()),
                Tok::Eof
            ]
        );
        assert_eq!(
            toks(r#"a<=b "q\"\n" $c"#),
            vec![
                Tok::Ident("a".into()),
                Tok::Le,
                Tok::Ident("b".into()),
                Tok::Str("q\"\n".into()),
                Tok::Ident("$c".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn errors_keep_partial_tokens() {
        let (t, err) = lex("a = \"oops", 0);
        assert_eq!(t.len(), 2);
        assert_eq!(err.unwrap().msg, "unterminated string");
    }
}
