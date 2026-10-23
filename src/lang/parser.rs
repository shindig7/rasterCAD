//! Recursive-descent parser. Expressions use precedence climbing.

use std::rc::Rc;

use super::ast::*;
use super::lexer::{Tok, Token, lex};
use crate::diag::{Error, FileId, Result, Span};

pub fn parse(src: &str, file: FileId) -> Result<Vec<Stmt>> {
    let (tokens, err) = lex(src, file);
    if let Some(err) = err {
        return Err(err);
    }
    let mut p = Parser {
        toks: tokens
            .into_iter()
            .filter(|t| t.tok != Tok::Comment)
            .collect(),
        pos: 0,
    };
    let mut stmts = Vec::new();
    while !p.at(&Tok::Eof) {
        p.statement(&mut stmts)?;
    }
    Ok(stmts)
}

struct Parser {
    /// Always ends with `Tok::Eof`.
    toks: Vec<Token>,
    pos: usize,
}

/// Binary operators from loosest to tightest. `^` and unary operators are handled separately.
const LEVELS: &[&[(Tok, BinOp)]] = &[
    &[(Tok::OrOr, BinOp::Or)],
    &[(Tok::AndAnd, BinOp::And)],
    &[(Tok::EqEq, BinOp::Eq), (Tok::Ne, BinOp::Ne)],
    &[
        (Tok::Lt, BinOp::Lt),
        (Tok::Le, BinOp::Le),
        (Tok::Gt, BinOp::Gt),
        (Tok::Ge, BinOp::Ge),
    ],
    &[(Tok::Plus, BinOp::Add), (Tok::Minus, BinOp::Sub)],
    &[
        (Tok::Star, BinOp::Mul),
        (Tok::Slash, BinOp::Div),
        (Tok::Percent, BinOp::Mod),
    ],
];

fn describe(tok: &Tok) -> String {
    match tok {
        Tok::Num(n) => format!("number {n}"),
        Tok::Str(_) => "string".into(),
        Tok::Ident(name) => format!("'{name}'"),
        Tok::Path(p) => format!("<{p}>"),
        Tok::Eof => "end of file".into(),
        Tok::Comment => "comment".into(),
        other => {
            let text = match other {
                Tok::LParen => "(",
                Tok::RParen => ")",
                Tok::LBracket => "[",
                Tok::RBracket => "]",
                Tok::LBrace => "{",
                Tok::RBrace => "}",
                Tok::Comma => ",",
                Tok::Semi => ";",
                Tok::Colon => ":",
                Tok::Assign => "=",
                Tok::Question => "?",
                Tok::Dot => ".",
                Tok::Plus => "+",
                Tok::Minus => "-",
                Tok::Star => "*",
                Tok::Slash => "/",
                Tok::Percent => "%",
                Tok::Caret => "^",
                Tok::Not => "!",
                Tok::Lt => "<",
                Tok::Le => "<=",
                Tok::Gt => ">",
                Tok::Ge => ">=",
                Tok::EqEq => "==",
                Tok::Ne => "!=",
                Tok::AndAnd => "&&",
                Tok::OrOr => "||",
                _ => return format!("keyword '{}'", format!("{other:?}").to_lowercase()),
            };
            format!("'{text}'")
        }
    }
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, ahead: usize) -> &Tok {
        let i = (self.pos + ahead).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn prev_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].span
    }

    fn at(&self, tok: &Tok) -> bool {
        self.peek() == tok
    }

    fn bump(&mut self) -> &Token {
        let t = &self.toks[self.pos];
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        let hit = self.at(tok);
        if hit {
            self.bump();
        }
        hit
    }

    fn expect(&mut self, tok: &Tok, what: &str) -> Result<Span> {
        if self.at(tok) {
            Ok(self.bump().span)
        } else {
            Err(self.unexpected(what))
        }
    }

    fn unexpected(&self, expected: &str) -> Error {
        Error::new(
            self.span(),
            format!("expected {expected}, found {}", describe(self.peek())),
        )
    }

    fn ident(&mut self, what: &str) -> Result<(String, Span)> {
        match self.peek().clone() {
            Tok::Ident(name) => Ok((name, self.bump().span)),
            _ => Err(self.unexpected(what)),
        }
    }

    // ---------------------------------------------------------------- statements

    /// Parses one statement, appending it to `out` (empty statements append nothing).
    fn statement(&mut self, out: &mut Vec<Stmt>) -> Result<()> {
        let start = self.span();
        match self.peek().clone() {
            Tok::Semi => {
                self.bump();
            }
            Tok::LBrace => out.push(Stmt::Block(self.block()?)),
            Tok::Star => {
                // `*stmt` disables a statement, as in OpenSCAD.
                self.bump();
                self.statement(&mut Vec::new())?;
            }
            Tok::Module => {
                self.bump();
                let (name, _) = self.ident("module name")?;
                let params = self.params()?;
                let body = self.child_body()?;
                out.push(Stmt::ModuleDef(Rc::new(ModuleDef {
                    name,
                    params,
                    body,
                    span: start.to(self.prev_span()),
                })));
            }
            Tok::Function => {
                self.bump();
                let (name, _) = self.ident("function name")?;
                let params = self.params()?;
                self.expect(&Tok::Assign, "'=' after function parameters")?;
                let body = self.expr()?;
                self.expect(&Tok::Semi, "';' after function body")?;
                out.push(Stmt::FunctionDef(Rc::new(FunctionDef {
                    name,
                    params,
                    body,
                    span: start.to(self.prev_span()),
                })));
            }
            Tok::Use | Tok::Include => {
                let is_use = self.at(&Tok::Use);
                self.bump();
                let Tok::Path(path) = self.peek().clone() else {
                    return Err(self.unexpected("a library path like <file.rcad>"));
                };
                let span = start.to(self.bump().span);
                self.eat(&Tok::Semi);
                out.push(if is_use {
                    Stmt::Use { path, span }
                } else {
                    Stmt::Include { path, span }
                });
            }
            Tok::If => {
                self.bump();
                self.expect(&Tok::LParen, "'(' after if")?;
                let cond = self.expr()?;
                self.expect(&Tok::RParen, "')' after condition")?;
                let then = self.child_body()?;
                let otherwise = if self.eat(&Tok::Else) {
                    self.child_body()?
                } else {
                    Rc::from([])
                };
                out.push(Stmt::If {
                    cond,
                    then,
                    otherwise,
                });
            }
            Tok::For | Tok::Let => {
                let is_for = self.at(&Tok::For);
                self.bump();
                let bindings = self.bindings()?;
                let body = self.child_body()?;
                out.push(if is_for {
                    Stmt::For { bindings, body }
                } else {
                    Stmt::Let { bindings, body }
                });
            }
            Tok::Ident(name) => {
                self.bump();
                if self.eat(&Tok::Assign) {
                    let expr = self.expr()?;
                    self.expect(&Tok::Semi, "';' after assignment")?;
                    out.push(Stmt::Assign {
                        name,
                        expr,
                        span: start.to(self.prev_span()),
                    });
                } else if self.at(&Tok::LParen) {
                    let args = self.args()?;
                    let span = start.to(self.prev_span());
                    let children = self.child_body()?;
                    out.push(Stmt::Call {
                        name,
                        args,
                        children,
                        span,
                    });
                } else {
                    return Err(self.unexpected("'=' or '(' after name"));
                }
            }
            _ => return Err(self.unexpected("a statement")),
        }
        Ok(())
    }

    fn block(&mut self) -> Result<Body> {
        self.expect(&Tok::LBrace, "'{'")?;
        let mut stmts = Vec::new();
        while !self.eat(&Tok::RBrace) {
            if self.at(&Tok::Eof) {
                return Err(self.unexpected("'}'"));
            }
            self.statement(&mut stmts)?;
        }
        Ok(stmts.into())
    }

    /// The children of a module call or control statement: `;`, `{ ... }`, or one statement.
    fn child_body(&mut self) -> Result<Body> {
        if self.at(&Tok::LBrace) {
            return self.block();
        }
        let mut stmts = Vec::new();
        self.statement(&mut stmts)?;
        Ok(stmts.into())
    }

    fn params(&mut self) -> Result<Vec<Param>> {
        self.expect(&Tok::LParen, "'('")?;
        let mut params = Vec::new();
        while !self.eat(&Tok::RParen) {
            let (name, span) = self.ident("parameter name")?;
            if params.iter().any(|p: &Param| p.name == name) {
                return Err(Error::new(span, format!("duplicate parameter '{name}'")));
            }
            let default = if self.eat(&Tok::Assign) {
                Some(self.expr()?)
            } else {
                None
            };
            params.push(Param { name, default });
            if !self.eat(&Tok::Comma) {
                self.expect(&Tok::RParen, "',' or ')'")?;
                break;
            }
        }
        Ok(params)
    }

    fn args(&mut self) -> Result<Vec<Arg>> {
        self.expect(&Tok::LParen, "'('")?;
        let mut args = Vec::new();
        while !self.eat(&Tok::RParen) {
            let start = self.span();
            let name = match (self.peek().clone(), self.peek_at(1)) {
                (Tok::Ident(n), Tok::Assign) => {
                    self.pos += 2;
                    Some(n)
                }
                _ => None,
            };
            let value = self.expr()?;
            args.push(Arg {
                name,
                span: start.to(value.span),
                value,
            });
            if !self.eat(&Tok::Comma) {
                self.expect(&Tok::RParen, "',' or ')'")?;
                break;
            }
        }
        Ok(args)
    }

    /// `(a = 1, b = 2)` as used by `for` and `let`.
    fn bindings(&mut self) -> Result<Vec<Binding>> {
        self.expect(&Tok::LParen, "'('")?;
        let mut bindings = Vec::new();
        while !self.eat(&Tok::RParen) {
            let (name, _) = self.ident("variable name")?;
            self.expect(&Tok::Assign, "'='")?;
            let expr = self.expr()?;
            bindings.push(Binding { name, expr });
            if !self.eat(&Tok::Comma) {
                self.expect(&Tok::RParen, "',' or ')'")?;
                break;
            }
        }
        Ok(bindings)
    }

    // ---------------------------------------------------------------- expressions

    fn expr(&mut self) -> Result<Expr> {
        if self.at(&Tok::Let) {
            let start = self.bump().span;
            let bindings = self.bindings()?;
            let body = self.expr()?;
            return Ok(Expr {
                span: start.to(body.span),
                kind: ExprKind::Let(bindings, Box::new(body)),
            });
        }
        let cond = self.binary(0)?;
        if !self.eat(&Tok::Question) {
            return Ok(cond);
        }
        let then = self.expr()?;
        self.expect(&Tok::Colon, "':' in conditional expression")?;
        let otherwise = self.expr()?;
        Ok(Expr {
            span: cond.span.to(otherwise.span),
            kind: ExprKind::Ternary(Box::new(cond), Box::new(then), Box::new(otherwise)),
        })
    }

    fn binary(&mut self, level: usize) -> Result<Expr> {
        let Some(ops) = LEVELS.get(level) else {
            return self.unary();
        };
        let mut lhs = self.binary(level + 1)?;
        while let Some(&(_, op)) = ops.iter().find(|(t, _)| self.at(t)) {
            self.bump();
            let rhs = self.binary(level + 1)?;
            lhs = Expr {
                span: lhs.span.to(rhs.span),
                kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)),
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr> {
        let op = match self.peek() {
            Tok::Minus => UnOp::Neg,
            Tok::Plus => UnOp::Plus,
            Tok::Not => UnOp::Not,
            _ => return self.power(),
        };
        let start = self.bump().span;
        let operand = self.unary()?;
        Ok(Expr {
            span: start.to(operand.span),
            kind: ExprKind::Unary(op, Box::new(operand)),
        })
    }

    /// `^` binds tighter than unary minus on its left and is right-associative.
    fn power(&mut self) -> Result<Expr> {
        let base = self.postfix()?;
        if !self.eat(&Tok::Caret) {
            return Ok(base);
        }
        let exp = self.unary()?;
        Ok(Expr {
            span: base.span.to(exp.span),
            kind: ExprKind::Binary(BinOp::Pow, Box::new(base), Box::new(exp)),
        })
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.eat(&Tok::LBracket) {
                let index = self.expr()?;
                let end = self.expect(&Tok::RBracket, "']'")?;
                e = Expr {
                    span: e.span.to(end),
                    kind: ExprKind::Index(Box::new(e), Box::new(index)),
                };
            } else if self.eat(&Tok::Dot) {
                let (field, end) = self.ident("member name after '.'")?;
                e = Expr {
                    span: e.span.to(end),
                    kind: ExprKind::Member(Box::new(e), field),
                };
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Num(n) => ExprKind::Num(n),
            Tok::Str(s) => ExprKind::Str(s.into()),
            Tok::True => ExprKind::Bool(true),
            Tok::False => ExprKind::Bool(false),
            Tok::Undef => ExprKind::Undef,
            Tok::Ident(name) => {
                self.bump();
                if self.at(&Tok::LParen) {
                    let args = self.args()?;
                    return Ok(Expr {
                        kind: ExprKind::Call(name, args),
                        span: span.to(self.prev_span()),
                    });
                }
                return Ok(Expr {
                    kind: ExprKind::Var(name),
                    span,
                });
            }
            Tok::LParen => {
                self.bump();
                let mut e = self.expr()?;
                let end = self.expect(&Tok::RParen, "')'")?;
                e.span = span.to(end);
                return Ok(e);
            }
            Tok::LBracket => return self.list(),
            _ => return Err(self.unexpected("an expression")),
        };
        self.bump();
        Ok(Expr { kind, span })
    }

    /// `[a, b]`, `[start:end]`, `[start:step:end]`, or a list comprehension.
    fn list(&mut self) -> Result<Expr> {
        let start = self.expect(&Tok::LBracket, "'['")?;
        let mut elems = Vec::new();
        if !self.at(&Tok::RBracket) {
            let first = self.list_elem()?;
            if let (ListElem::Expr(first), true) = (&first, self.at(&Tok::Colon)) {
                return self.range(start, first.clone());
            }
            elems.push(first);
            while self.eat(&Tok::Comma) {
                if self.at(&Tok::RBracket) {
                    break; // trailing comma
                }
                elems.push(self.list_elem()?);
            }
        }
        let end = self.expect(&Tok::RBracket, "',' or ']'")?;
        Ok(Expr {
            kind: ExprKind::List(elems),
            span: start.to(end),
        })
    }

    fn range(&mut self, start_span: Span, first: Expr) -> Result<Expr> {
        self.expect(&Tok::Colon, "':'")?;
        let second = self.expr()?;
        let (step, end) = if self.eat(&Tok::Colon) {
            (Some(Box::new(second)), self.expr()?)
        } else {
            (None, second)
        };
        let close = self.expect(&Tok::RBracket, "']' to close range")?;
        Ok(Expr {
            kind: ExprKind::Range {
                start: Box::new(first),
                step,
                end: Box::new(end),
            },
            span: start_span.to(close),
        })
    }

    fn list_elem(&mut self) -> Result<ListElem> {
        match self.peek() {
            Tok::For => {
                self.bump();
                let bindings = self.bindings()?;
                Ok(ListElem::For(bindings, Box::new(self.list_elem()?)))
            }
            Tok::Let => {
                self.bump();
                let bindings = self.bindings()?;
                Ok(ListElem::Let(bindings, Box::new(self.list_elem()?)))
            }
            Tok::Each => {
                self.bump();
                Ok(ListElem::Each(self.expr()?))
            }
            Tok::If => {
                self.bump();
                self.expect(&Tok::LParen, "'(' after if")?;
                let cond = self.expr()?;
                self.expect(&Tok::RParen, "')' after condition")?;
                let then = Box::new(self.list_elem()?);
                let otherwise = if self.eat(&Tok::Else) {
                    Some(Box::new(self.list_elem()?))
                } else {
                    None
                };
                Ok(ListElem::If(cond, then, otherwise))
            }
            _ => Ok(ListElem::Expr(self.expr()?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr_of(src: &str) -> Expr {
        match parse(&format!("x = {src};"), 0).unwrap().remove(0) {
            Stmt::Assign { expr, .. } => expr,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn precedence() {
        // -2^2 parses as -(2^2); 1 + 2 * 3 as 1 + (2 * 3)
        assert!(matches!(
            expr_of("-2^2").kind,
            ExprKind::Unary(UnOp::Neg, ref e) if matches!(e.kind, ExprKind::Binary(BinOp::Pow, ..))
        ));
        assert!(matches!(
            expr_of("1 + 2 * 3").kind,
            ExprKind::Binary(BinOp::Add, _, ref r) if matches!(r.kind, ExprKind::Binary(BinOp::Mul, ..))
        ));
    }

    #[test]
    fn statements() {
        let src = "module m(a, b=2) { children(); }
                   function f(x) = x * 2;
                   translate([1,2]) rotate(45) m(1) circle(3);
                   for (i = [0:2]) if (i > 0) rect(i); else ;
                   *circle(1);";
        let stmts = parse(src, 0).unwrap();
        assert_eq!(stmts.len(), 4);
        let Stmt::Call { name, children, .. } = &stmts[2] else {
            panic!()
        };
        assert_eq!(name, "translate");
        assert_eq!(children.len(), 1);
    }

    #[test]
    fn comprehension_and_range() {
        assert!(matches!(
            expr_of("[for (i = [0:2:10]) if (i % 4 == 0) i]").kind,
            ExprKind::List(ref e) if matches!(e[0], ListElem::For(..))
        ));
        assert!(matches!(
            expr_of("[0:5]").kind,
            ExprKind::Range { step: None, .. }
        ));
    }

    #[test]
    fn error_location() {
        let err = parse("circle(5)\nrect([1,2]", 0).unwrap_err();
        assert_eq!(err.msg, "expected ',' or ')', found end of file");
    }
}
