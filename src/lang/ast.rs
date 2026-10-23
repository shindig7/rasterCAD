//! Syntax tree produced by the parser.

use std::rc::Rc;

use crate::diag::Span;

/// A statement list shared between the definition and every instantiation (cheap to clone).
pub type Body = Rc<[Stmt]>;

#[derive(Debug, Clone)]
pub enum Stmt {
    Assign {
        name: String,
        expr: Expr,
        span: Span,
    },
    ModuleDef(Rc<ModuleDef>),
    FunctionDef(Rc<FunctionDef>),
    Use {
        path: String,
        span: Span,
    },
    Include {
        path: String,
        span: Span,
    },
    /// `name(args) children`, for both builtin and user modules.
    Call {
        name: String,
        args: Vec<Arg>,
        children: Body,
        span: Span,
    },
    If {
        cond: Expr,
        then: Body,
        otherwise: Body,
    },
    For {
        bindings: Vec<Binding>,
        body: Body,
    },
    Let {
        bindings: Vec<Binding>,
        body: Body,
    },
    Block(Body),
}

impl Stmt {
    /// Whether the statement produces geometry (as opposed to defining something).
    pub fn is_instantiation(&self) -> bool {
        matches!(
            self,
            Stmt::Call { .. }
                | Stmt::If { .. }
                | Stmt::For { .. }
                | Stmt::Let { .. }
                | Stmt::Block(_)
        )
    }
}

#[derive(Debug)]
pub struct ModuleDef {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Body,
    pub span: Span,
}

#[derive(Debug)]
pub struct FunctionDef {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub name: String,
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Num(f64),
    Str(Rc<str>),
    Bool(bool),
    Undef,
    Var(String),
    List(Vec<ListElem>),
    Range {
        start: Box<Expr>,
        step: Option<Box<Expr>>,
        end: Box<Expr>,
    },
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Arg>),
    Index(Box<Expr>, Box<Expr>),
    Member(Box<Expr>, String),
    Let(Vec<Binding>, Box<Expr>),
}

/// An element of a list literal, including list-comprehension forms.
#[derive(Debug, Clone)]
pub enum ListElem {
    Expr(Expr),
    For(Vec<Binding>, Box<ListElem>),
    If(Expr, Box<ListElem>, Option<Box<ListElem>>),
    Let(Vec<Binding>, Box<ListElem>),
    Each(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}
