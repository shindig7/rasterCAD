//! Tree-walking evaluator: turns statements into a scene tree.
//!
//! Scoping follows OpenSCAD: within a block, module/function definitions and `use` are hoisted,
//! then assignments run in order, then geometry is instantiated. Lookup is lexical.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use tiny_skia::{Color, IntSize, LineCap, PathBuilder, Pixmap, Rect, Stroke, Transform};

use super::ast::*;
use super::functions;
use super::parser::parse;
use super::value::{self, Value, range_values};
use crate::diag::{Error, Result, SourceMap, Span};
use crate::render::filters;
use crate::render::text::{self, HAlign, TextStyle, VAlign};
use crate::scene::{Canvas, Node};

/// Maximum nesting of module/function calls. The evaluation thread's stack is sized for it.
pub const MAX_DEPTH: usize = 5000;
const MAX_CANVAS: u32 = 16384;

/// Names of the builtin modules (filters are listed separately in `render::filters::FILTERS`).
pub const BUILTIN_MODULES: &[&str] = &[
    "canvas",
    "rect",
    "circle",
    "ellipse",
    "polygon",
    "line",
    "text",
    "image",
    "translate",
    "rotate",
    "scale",
    "mirror",
    "color",
    "stroke",
    "opacity",
    "children",
    "echo",
    "assert",
];

// ------------------------------------------------------------------------- environments

struct Children {
    body: Body,
    /// The caller's scope, where the children are evaluated.
    env: Rc<Env>,
}

/// Definitions by name.
type Table<T> = RefCell<HashMap<String, Rc<T>>>;

#[derive(Default)]
struct Env {
    parent: Option<Rc<Env>>,
    vars: RefCell<HashMap<String, Value>>,
    functions: Table<FunctionDef>,
    modules: Table<ModuleDef>,
    /// Libraries imported with `use`; their own definitions are visible here (not transitively).
    uses: RefCell<Vec<Rc<Env>>>,
    /// Set on the frame of a user module call.
    children: Option<Rc<Children>>,
}

impl Env {
    fn root() -> Rc<Env> {
        let env = Env::default();
        env.set("PI", Value::Num(std::f64::consts::PI));
        Rc::new(env)
    }

    fn new(parent: &Rc<Env>, children: Option<Rc<Children>>) -> Rc<Env> {
        Rc::new(Env {
            parent: Some(parent.clone()),
            children,
            ..Env::default()
        })
    }

    fn child(self: &Rc<Self>) -> Rc<Env> {
        Env::new(self, None)
    }

    fn set(&self, name: &str, v: Value) {
        self.vars.borrow_mut().insert(name.to_string(), v);
    }

    fn var(&self, name: &str) -> Option<Value> {
        let mut env = self;
        loop {
            if let Some(v) = env.vars.borrow().get(name) {
                return Some(v.clone());
            }
            env = env.parent.as_deref()?;
        }
    }

    /// Finds a definition and the environment it was defined in (its closure scope).
    fn lookup<T>(
        self: &Rc<Self>,
        name: &str,
        table: fn(&Env) -> &Table<T>,
    ) -> Option<(Rc<T>, Rc<Env>)> {
        let mut env = self.clone();
        loop {
            if let Some(def) = table(&env).borrow().get(name) {
                return Some((def.clone(), env.clone()));
            }
            for lib in env.uses.borrow().iter() {
                if let Some(def) = table(lib).borrow().get(name) {
                    return Some((def.clone(), lib.clone()));
                }
            }
            env = env.parent.clone()?;
        }
    }

    fn children(&self) -> Option<Rc<Children>> {
        let mut env = self;
        loop {
            if let Some(c) = &env.children {
                return Some(c.clone());
            }
            env = env.parent.as_deref()?;
        }
    }
}

// ------------------------------------------------------------------------- arguments

struct ArgVal {
    name: Option<String>,
    value: Value,
    span: Span,
}

/// Matches arguments to parameter names: positional first-come, named by name.
fn bind_args(what: &str, params: &[&str], args: Vec<ArgVal>) -> Result<Vec<Option<(Value, Span)>>> {
    let mut out: Vec<Option<(Value, Span)>> = vec![None; params.len()];
    let mut next = 0;
    for a in args {
        let idx = match &a.name {
            Some(n) => params
                .iter()
                .position(|p| p == n)
                .ok_or_else(|| Error::new(a.span, format!("{what} has no parameter '{n}'")))?,
            None => {
                next += 1;
                if next > params.len() {
                    let msg = format!("too many arguments to {what} (takes {})", params.len());
                    return Err(Error::new(a.span, msg));
                }
                next - 1
            }
        };
        if out[idx].is_some() {
            let msg = format!("parameter '{}' of {what} given twice", params[idx]);
            return Err(Error::new(a.span, msg));
        }
        out[idx] = Some((a.value, a.span));
    }
    Ok(out)
}

/// Typed access to the bound arguments of a builtin module.
struct Args {
    what: String,
    names: &'static [&'static str],
    vals: Vec<Option<(Value, Span)>>,
    call: Span,
}

impl Args {
    /// The argument at `i`, treating an explicit `undef` as absent.
    fn get(&self, i: usize) -> Option<&(Value, Span)> {
        self.vals[i].as_ref().filter(|(v, _)| *v != Value::Undef)
    }

    fn type_err(&self, i: usize, expected: &str) -> Error {
        let (v, span) = self.vals[i]
            .as_ref()
            .map_or((&Value::Undef, self.call), |(v, s)| (v, *s));
        Error::new(
            span,
            format!(
                "{}: '{}' must be {expected}, got {}",
                self.what,
                self.names[i],
                v.type_name()
            ),
        )
    }

    fn missing(&self, i: usize) -> Error {
        Error::new(
            self.call,
            format!("{}: missing argument '{}'", self.what, self.names[i]),
        )
    }

    fn num(&self, i: usize) -> Result<Option<f64>> {
        match self.get(i) {
            None => Ok(None),
            Some((Value::Num(n), _)) if n.is_finite() => Ok(Some(*n)),
            Some(_) => Err(self.type_err(i, "a finite number")),
        }
    }

    fn req_num(&self, i: usize) -> Result<f64> {
        self.num(i)?.ok_or_else(|| self.missing(i))
    }

    fn bool(&self, i: usize) -> Result<Option<bool>> {
        match self.get(i) {
            None => Ok(None),
            Some((Value::Bool(b), _)) => Ok(Some(*b)),
            Some(_) => Err(self.type_err(i, "true or false")),
        }
    }

    fn str(&self, i: usize) -> Result<Option<Rc<str>>> {
        match self.get(i) {
            None => Ok(None),
            Some((Value::Str(s), _)) => Ok(Some(s.clone())),
            Some(_) => Err(self.type_err(i, "a string")),
        }
    }

    /// `[x, y]`, where entries may be `undef` (returned as `None`).
    fn vec2_opt(&self, i: usize) -> Result<Option<[Option<f64>; 2]>> {
        let Some((v, _)) = self.get(i) else {
            return Ok(None);
        };
        let entry = |v: &Value| match v {
            Value::Num(n) if n.is_finite() => Ok(Some(*n)),
            Value::Undef => Ok(None),
            _ => Err(self.type_err(i, "a list of two numbers")),
        };
        match v {
            Value::List(items) if items.len() == 2 => {
                Ok(Some([entry(&items[0])?, entry(&items[1])?]))
            }
            _ => Err(self.type_err(i, "a list of two numbers")),
        }
    }

    fn vec2(&self, i: usize) -> Result<Option<[f64; 2]>> {
        match self.vec2_opt(i)? {
            None => Ok(None),
            Some([Some(x), Some(y)]) => Ok(Some([x, y])),
            Some(_) => Err(self.type_err(i, "a list of two numbers")),
        }
    }

    /// A number `n` (meaning `[n, n]`) or `[x, y]`.
    fn num_or_vec2(&self, i: usize) -> Result<Option<[f64; 2]>> {
        match self.get(i) {
            Some((Value::Num(_), _)) => Ok(self.num(i)?.map(|n| [n, n])),
            _ => self.vec2(i),
        }
    }

    fn span(&self, i: usize) -> Span {
        self.vals[i].as_ref().map_or(self.call, |(_, s)| *s)
    }
}

// ------------------------------------------------------------------------- evaluator

pub struct Evaluator {
    pub sources: SourceMap,
    /// Loaded `use` libraries by canonical path; `None` while a library is being loaded.
    libs: HashMap<PathBuf, Option<Rc<Env>>>,
    include_stack: Vec<PathBuf>,
    depth: usize,
    canvas: Option<Canvas>,
    /// Output of `echo()`.
    pub log: Vec<String>,
}

impl Default for Evaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl Evaluator {
    pub fn new() -> Self {
        Self {
            sources: SourceMap::default(),
            libs: HashMap::new(),
            include_stack: Vec::new(),
            depth: 0,
            canvas: None,
            log: Vec::new(),
        }
    }

    /// Evaluates a whole script. `path` locates relative libraries, images and fonts.
    pub fn run(&mut self, source: &str, path: Option<&Path>) -> Result<(Canvas, Vec<Node>)> {
        let file = self
            .sources
            .add(path.map(Path::to_path_buf), source.to_string());
        let stmts = parse(source, file)?;
        let nodes = self.eval_block(&stmts, &Env::root(), None)?;
        let canvas = self.canvas.take().ok_or_else(|| {
            Error::new(
                Span::new(file, 0, 0),
                "the script never calls canvas(width, height)",
            )
        })?;
        Ok((canvas, nodes))
    }

    /// Runs `f` one call level deeper, failing cleanly instead of overflowing the stack.
    fn nested<T>(&mut self, span: Span, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.depth >= MAX_DEPTH {
            let msg = format!("recursion too deep (more than {MAX_DEPTH} nested calls)");
            return Err(Error::new(span, msg));
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }

    fn base_dir(&self, span: Span) -> PathBuf {
        self.sources
            .path(span.file)
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    // --------------------------------------------------------------------- libraries

    fn resolve_library(&self, rel: &str, span: Span) -> Result<PathBuf> {
        let mut dirs = vec![self.base_dir(span)];
        if let Some(paths) = std::env::var_os("RASTERCAD_PATH") {
            dirs.extend(std::env::split_paths(&paths));
        }
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".local/share/rastercad/libraries"));
        }
        dirs.iter()
            .map(|d| d.join(rel))
            .find(|p| p.is_file())
            .and_then(|p| p.canonicalize().ok())
            .ok_or_else(|| {
                Error::new(
                    span,
                    format!(
                        "library <{rel}> not found (searched the script's directory, \
                         $RASTERCAD_PATH and ~/.local/share/rastercad/libraries)"
                    ),
                )
            })
    }

    fn load_source(&mut self, path: &Path, span: Span) -> Result<Vec<Stmt>> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::new(span, format!("cannot read {}: {e}", path.display())))?;
        let file = self.sources.add(Some(path.to_path_buf()), text);
        parse(self.sources.text(file), file)
    }

    /// Loads a `use`d library once; later `use`s share the same definitions.
    fn load_library(&mut self, rel: &str, span: Span) -> Result<Rc<Env>> {
        let path = self.resolve_library(rel, span)?;
        match self.libs.get(&path) {
            Some(Some(env)) => return Ok(env.clone()),
            Some(None) => {
                let msg = format!("circular use of {}", path.display());
                return Err(Error::new(span, msg));
            }
            None => {}
        }
        self.libs.insert(path.clone(), None);
        let env = Env::root();
        let loaded = self
            .load_source(&path, span)
            .and_then(|stmts| self.declare(&stmts, &env).map(drop));
        match loaded {
            Ok(()) => {
                self.libs.insert(path, Some(env.clone()));
                Ok(env)
            }
            Err(e) => {
                self.libs.remove(&path);
                Err(e)
            }
        }
    }

    /// Replaces `include` statements with the included files' statements.
    fn expand_includes<'s>(&mut self, stmts: &'s [Stmt]) -> Result<Cow<'s, [Stmt]>> {
        if !stmts.iter().any(|s| matches!(s, Stmt::Include { .. })) {
            return Ok(Cow::Borrowed(stmts));
        }
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            let Stmt::Include { path, span } = stmt else {
                out.push(stmt.clone());
                continue;
            };
            let path = self.resolve_library(path, *span)?;
            if self.include_stack.contains(&path) {
                let msg = format!("circular include of {}", path.display());
                return Err(Error::new(*span, msg));
            }
            let inner = self.load_source(&path, *span)?;
            self.include_stack.push(path);
            let expanded = self.expand_includes(&inner).map(Cow::into_owned);
            self.include_stack.pop();
            out.extend(expanded?);
        }
        Ok(Cow::Owned(out))
    }

    // --------------------------------------------------------------------- statements

    /// Hoists definitions and `use`, then runs assignments. Returns the expanded statements.
    fn declare<'s>(&mut self, stmts: &'s [Stmt], env: &Rc<Env>) -> Result<Cow<'s, [Stmt]>> {
        let stmts = self.expand_includes(stmts)?;
        for stmt in stmts.iter() {
            match stmt {
                Stmt::ModuleDef(def) => {
                    env.modules
                        .borrow_mut()
                        .insert(def.name.clone(), def.clone());
                }
                Stmt::FunctionDef(def) => {
                    env.functions
                        .borrow_mut()
                        .insert(def.name.clone(), def.clone());
                }
                Stmt::Use { path, span } => {
                    let lib = self.load_library(path, *span)?;
                    env.uses.borrow_mut().push(lib);
                }
                _ => {}
            }
        }
        for stmt in stmts.iter() {
            if let Stmt::Assign { name, expr, .. } = stmt {
                let v = self.eval_expr(expr, env)?;
                env.set(name, v);
            }
        }
        Ok(stmts)
    }

    /// Evaluates a block in `env`. `select` keeps only the instantiations at those indices.
    fn eval_block(
        &mut self,
        stmts: &[Stmt],
        env: &Rc<Env>,
        select: Option<&[usize]>,
    ) -> Result<Vec<Node>> {
        let stmts = self.declare(stmts, env)?;
        let mut nodes = Vec::new();
        for (i, stmt) in stmts.iter().filter(|s| s.is_instantiation()).enumerate() {
            if select.is_none_or(|sel| sel.contains(&i)) {
                nodes.extend(self.eval_stmt(stmt, env)?);
            }
        }
        Ok(nodes)
    }

    fn eval_stmt(&mut self, stmt: &Stmt, env: &Rc<Env>) -> Result<Vec<Node>> {
        match stmt {
            Stmt::Block(body) => self.eval_block(body, &env.child(), None),
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                let branch = if self.eval_expr(cond, env)?.truthy() {
                    then
                } else {
                    otherwise
                };
                self.eval_block(branch, &env.child(), None)
            }
            Stmt::Let { bindings, body } => {
                let scope = self.bind_sequential(bindings, env)?;
                self.eval_block(body, &scope, None)
            }
            Stmt::For { bindings, body } => {
                let mut nodes = Vec::new();
                self.for_each(bindings, env, &mut |ev, scope| {
                    nodes.extend(ev.eval_block(body, &scope, None)?);
                    Ok(())
                })?;
                Ok(nodes)
            }
            Stmt::Call {
                name,
                args,
                children,
                span,
            } => self.call_module(name, args, children, env, *span),
            Stmt::Assign { .. }
            | Stmt::ModuleDef(_)
            | Stmt::FunctionDef(_)
            | Stmt::Use { .. }
            | Stmt::Include { .. } => Ok(Vec::new()),
        }
    }

    fn bind_sequential(&mut self, bindings: &[Binding], env: &Rc<Env>) -> Result<Rc<Env>> {
        let scope = env.child();
        for b in bindings {
            let v = self.eval_expr(&b.expr, &scope)?;
            scope.set(&b.name, v);
        }
        Ok(scope)
    }

    /// Runs `f` for every combination of the (nested) loop bindings.
    fn for_each(
        &mut self,
        bindings: &[Binding],
        env: &Rc<Env>,
        f: &mut dyn FnMut(&mut Self, Rc<Env>) -> Result<()>,
    ) -> Result<()> {
        let Some((first, rest)) = bindings.split_first() else {
            return f(self, env.clone());
        };
        let items = self.eval_expr(&first.expr, env)?;
        for item in iterate(&items).map_err(|m| Error::new(first.expr.span, m))? {
            let scope = env.child();
            scope.set(&first.name, item);
            self.for_each(rest, &scope, f)?;
        }
        Ok(())
    }

    fn eval_args(&mut self, args: &[Arg], env: &Rc<Env>) -> Result<Vec<ArgVal>> {
        args.iter()
            .map(|a| {
                Ok(ArgVal {
                    name: a.name.clone(),
                    value: self.eval_expr(&a.value, env)?,
                    span: a.span,
                })
            })
            .collect()
    }

    /// Binds a user module's or function's parameters into `frame`, evaluating defaults there.
    fn bind_params(
        &mut self,
        what: &str,
        params: &[Param],
        args: Vec<ArgVal>,
        frame: &Rc<Env>,
    ) -> Result<()> {
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        let bound = bind_args(what, &names, args)?;
        for (param, arg) in params.iter().zip(bound) {
            let v = match (arg, &param.default) {
                (Some((v, _)), _) => v,
                (None, Some(default)) => self.eval_expr(default, frame)?,
                (None, None) => Value::Undef,
            };
            frame.set(&param.name, v);
        }
        Ok(())
    }

    // --------------------------------------------------------------------- modules

    fn call_module(
        &mut self,
        name: &str,
        args: &[Arg],
        children: &Body,
        env: &Rc<Env>,
        span: Span,
    ) -> Result<Vec<Node>> {
        let args = self.eval_args(args, env)?;
        if let Some((def, def_env)) = env.lookup(name, |e| &e.modules) {
            return self.nested(span, |ev| {
                let ctx = Children {
                    body: children.clone(),
                    env: env.clone(),
                };
                let frame = Env::new(&def_env, Some(Rc::new(ctx)));
                ev.bind_params(&format!("{name}()"), &def.params, args, &frame)?;
                let count = children.iter().filter(|s| s.is_instantiation()).count();
                frame.set("$children", Value::Num(count as f64));
                ev.eval_block(&def.body, &frame, None)
            });
        }
        self.builtin_module(name, args, children, env, span)
    }

    fn builtin_module(
        &mut self,
        name: &str,
        args: Vec<ArgVal>,
        children: &Body,
        env: &Rc<Env>,
        span: Span,
    ) -> Result<Vec<Node>> {
        let params: &'static [&'static str] = match name {
            "canvas" => &["width", "height", "bg"],
            "rect" => &["size", "center", "r"],
            "circle" => &["r", "d"],
            "ellipse" => &["r"],
            "polygon" => &["points"],
            "line" => &["from", "to", "width"],
            "text" => &[
                "text",
                "size",
                "font",
                "halign",
                "valign",
                "color",
                "outline_thickness",
                "outline_color",
            ],
            "image" => &["file", "size", "center"],
            "translate" => &["v"],
            "rotate" => &["a", "center"],
            "scale" => &["v"],
            "mirror" => &["v"],
            "color" => &["c", "alpha"],
            "stroke" => &["width"],
            "opacity" => &["a"],
            "children" => &["index"],
            "assert" => &["condition", "message"],
            "echo" => return self.echo(args, children, env),
            _ => {
                if let Some(def) = filters::find(name) {
                    return self.filter(def, args, children, env, span);
                }
                let msg = if env.lookup(name, |e| &e.functions).is_some()
                    || functions::call(name, &[]).is_some()
                {
                    format!("'{name}' is a function, not a module")
                } else {
                    format!("unknown module '{name}'")
                };
                return Err(Error::new(span, msg));
            }
        };
        let what = format!("{name}()");
        let a = Args {
            vals: bind_args(&what, params, args)?,
            what,
            names: params,
            call: span,
        };
        let is_leaf = matches!(
            name,
            "canvas"
                | "rect"
                | "circle"
                | "ellipse"
                | "polygon"
                | "line"
                | "text"
                | "image"
                | "children"
                | "assert"
        );
        if is_leaf && children.iter().any(Stmt::is_instantiation) {
            return Err(Error::new(span, format!("{name}() does not take children")));
        }
        let shape = |pb: Option<tiny_skia::Path>| Ok(pb.map(Node::Shape).into_iter().collect());

        match name {
            "canvas" => {
                self.canvas(&a)?;
                Ok(Vec::new())
            }
            "rect" => {
                let [w, h] = a.num_or_vec2(0)?.ok_or_else(|| a.missing(0))?;
                if w < 0.0 || h < 0.0 {
                    return Err(Error::new(a.span(0), "rect(): size must not be negative"));
                }
                let (x, y) = if a.bool(1)?.unwrap_or(false) {
                    (-w / 2.0, -h / 2.0)
                } else {
                    (0.0, 0.0)
                };
                let r = a.num(2)?.unwrap_or(0.0).clamp(0.0, w.min(h) / 2.0);
                shape(rounded_rect(
                    x as f32, y as f32, w as f32, h as f32, r as f32,
                ))
            }
            "circle" => {
                let r = match (a.num(0)?, a.num(1)?) {
                    (Some(r), None) => r,
                    (None, Some(d)) => d / 2.0,
                    (None, None) => return Err(a.missing(0)),
                    (Some(_), Some(_)) => {
                        return Err(Error::new(span, "circle(): give either r or d, not both"));
                    }
                };
                if r < 0.0 {
                    return Err(Error::new(span, "circle(): radius must not be negative"));
                }
                shape(PathBuilder::from_circle(0.0, 0.0, r as f32))
            }
            "ellipse" => {
                let [rx, ry] = a.num_or_vec2(0)?.ok_or_else(|| a.missing(0))?;
                if rx < 0.0 || ry < 0.0 {
                    return Err(Error::new(
                        a.span(0),
                        "ellipse(): radii must not be negative",
                    ));
                }
                let rect =
                    Rect::from_xywh(-rx as f32, -ry as f32, 2.0 * rx as f32, 2.0 * ry as f32);
                shape(rect.and_then(PathBuilder::from_oval))
            }
            "polygon" => {
                let points = self.points(&a, 0)?;
                if points.len() < 3 {
                    return Err(Error::new(a.span(0), "polygon(): needs at least 3 points"));
                }
                let mut pb = PathBuilder::new();
                pb.move_to(points[0][0], points[0][1]);
                for p in &points[1..] {
                    pb.line_to(p[0], p[1]);
                }
                pb.close();
                shape(pb.finish())
            }
            "line" => {
                let from = a.vec2(0)?.ok_or_else(|| a.missing(0))?;
                let to = a.vec2(1)?.ok_or_else(|| a.missing(1))?;
                let width = a.num(2)?.unwrap_or(1.0);
                if width <= 0.0 {
                    return Err(Error::new(a.span(2), "line(): width must be positive"));
                }
                let mut pb = PathBuilder::new();
                pb.move_to(from[0] as f32, from[1] as f32);
                pb.line_to(to[0] as f32, to[1] as f32);
                let stroke = Stroke {
                    width: width as f32,
                    line_cap: LineCap::Butt,
                    ..Stroke::default()
                };
                shape(pb.finish().and_then(|p| p.stroke(&stroke, 1.0)))
            }
            "text" => {
                let content = match a.get(0) {
                    Some((v @ (Value::Str(_) | Value::Num(_)), _)) => v.to_string(),
                    Some(_) => return Err(a.type_err(0, "a string")),
                    None => return Err(a.missing(0)),
                };
                let halign = match a.str(3)?.as_deref() {
                    None | Some("left") => HAlign::Left,
                    Some("center") => HAlign::Center,
                    Some("right") => HAlign::Right,
                    Some(_) => return Err(a.type_err(3, "\"left\", \"center\" or \"right\"")),
                };
                let valign = match a.str(4)?.as_deref() {
                    None | Some("baseline") => VAlign::Baseline,
                    Some("top") => VAlign::Top,
                    Some("center") => VAlign::Center,
                    Some("bottom") => VAlign::Bottom,
                    Some(_) => {
                        return Err(
                            a.type_err(4, "\"top\", \"center\", \"baseline\" or \"bottom\"")
                        );
                    }
                };
                let font = a.str(2)?;
                let style = TextStyle {
                    size: a.num(1)?.unwrap_or(16.0) as f32,
                    font: font.as_deref().unwrap_or("sans-serif"),
                    halign,
                    valign,
                };
                let Some(path) = text::text_path(&content, &style, &self.base_dir(span))
                    .map_err(|m| Error::new(span, format!("text(): {m}")))?
                else {
                    return Ok(Vec::new());
                };
                let color = |i: usize| {
                    a.get(i)
                        .map(|(v, s)| {
                            color_from_value(v, None)
                                .map_err(|m| Error::new(*s, format!("text(): {m}")))
                        })
                        .transpose()
                };
                let mut fill = Node::Shape(path.clone());
                if let Some(c) = color(5)? {
                    fill = Node::Color(c, vec![fill]);
                }
                let thickness = a.num(6)?.unwrap_or(0.0);
                if thickness < 0.0 {
                    return Err(Error::new(
                        a.span(6),
                        "text(): outline_thickness must not be negative",
                    ));
                }
                if thickness == 0.0 {
                    return Ok(vec![fill]);
                }
                // The stroke is centered on the glyph edge and drawn behind the fill, so
                // doubling it leaves `thickness` pixels visible outside the letters.
                let outline = Node::Stroke(2.0 * thickness as f32, vec![Node::Shape(path)]);
                let outline_color = color(7)?.unwrap_or(Color::BLACK);
                Ok(vec![Node::Color(outline_color, vec![outline]), fill])
            }
            "image" => self.image(&a),
            "translate" => {
                let [x, y] = a.vec2(0)?.ok_or_else(|| a.missing(0))?;
                self.wrap_transform(Transform::from_translate(x as f32, y as f32), children, env)
            }
            "rotate" => {
                let angle = a.req_num(0)? as f32;
                let [cx, cy] = a.vec2(1)?.unwrap_or([0.0, 0.0]);
                self.wrap_transform(
                    Transform::from_rotate_at(angle, cx as f32, cy as f32),
                    children,
                    env,
                )
            }
            "scale" => {
                let [sx, sy] = a.num_or_vec2(0)?.ok_or_else(|| a.missing(0))?;
                self.wrap_transform(Transform::from_scale(sx as f32, sy as f32), children, env)
            }
            "mirror" => {
                // Reflect across the line through the origin perpendicular to v.
                let [nx, ny] = a.vec2(0)?.ok_or_else(|| a.missing(0))?;
                let d = nx * nx + ny * ny;
                if d == 0.0 {
                    return Err(Error::new(a.span(0), "mirror(): vector must not be [0, 0]"));
                }
                let (xx, xy, yy) = (
                    1.0 - 2.0 * nx * nx / d,
                    -2.0 * nx * ny / d,
                    1.0 - 2.0 * ny * ny / d,
                );
                let t = Transform::from_row(xx as f32, xy as f32, xy as f32, yy as f32, 0.0, 0.0);
                self.wrap_transform(t, children, env)
            }
            "color" => {
                let color = parse_color(&a)?;
                Ok(vec![Node::Color(
                    color,
                    self.eval_block(children, &env.child(), None)?,
                )])
            }
            "stroke" => {
                let width = a.req_num(0)?;
                if width <= 0.0 {
                    return Err(Error::new(a.span(0), "stroke(): width must be positive"));
                }
                Ok(vec![Node::Stroke(
                    width as f32,
                    self.eval_block(children, &env.child(), None)?,
                )])
            }
            "opacity" => {
                let alpha = a.req_num(0)?;
                if !(0.0..=1.0).contains(&alpha) {
                    return Err(Error::new(a.span(0), "opacity(): must be between 0 and 1"));
                }
                Ok(vec![Node::Layer {
                    opacity: alpha as f32,
                    filter: None,
                    span,
                    children: self.eval_block(children, &env.child(), None)?,
                }])
            }
            "children" => self.children(&a, env, span),
            "assert" => {
                let ok = a.get(0).is_some_and(|(v, _)| v.truthy());
                if ok {
                    Ok(Vec::new())
                } else {
                    let msg = a.get(1).map_or_else(
                        || "assertion failed".into(),
                        |(v, _)| format!("assertion failed: {v}"),
                    );
                    Err(Error::new(span, msg))
                }
            }
            _ => unreachable!("every name in the params table is handled"),
        }
    }

    fn wrap_transform(
        &mut self,
        t: Transform,
        children: &Body,
        env: &Rc<Env>,
    ) -> Result<Vec<Node>> {
        Ok(vec![Node::Transform(
            t,
            self.eval_block(children, &env.child(), None)?,
        )])
    }

    fn canvas(&mut self, a: &Args) -> Result<()> {
        if self.canvas.is_some() {
            return Err(Error::new(a.call, "canvas() can only be called once"));
        }
        let dim = |i: usize| -> Result<u32> {
            let n = a.req_num(i)?.round();
            if !(1.0..=f64::from(MAX_CANVAS)).contains(&n) {
                let msg = format!(
                    "canvas(): {} must be between 1 and {MAX_CANVAS}",
                    a.names[i]
                );
                return Err(Error::new(a.span(i), msg));
            }
            Ok(n as u32)
        };
        let background = match a.get(2) {
            None => Color::TRANSPARENT,
            Some((v, span)) => color_from_value(v, None)
                .map_err(|m| Error::new(*span, format!("canvas(): {m}")))?,
        };
        self.canvas = Some(Canvas {
            width: dim(0)?,
            height: dim(1)?,
            background,
            span: a.call,
        });
        Ok(())
    }

    fn points(&self, a: &Args, i: usize) -> Result<Vec<[f32; 2]>> {
        let err = || a.type_err(i, "a list of [x, y] points");
        let Some((Value::List(items), _)) = a.get(i) else {
            return Err(if a.get(i).is_some() {
                err()
            } else {
                a.missing(i)
            });
        };
        items
            .iter()
            .map(|p| match p {
                Value::List(xy) => match xy.as_ref() {
                    [Value::Num(x), Value::Num(y)] if x.is_finite() && y.is_finite() => {
                        Ok([*x as f32, *y as f32])
                    }
                    _ => Err(err()),
                },
                _ => Err(err()),
            })
            .collect()
    }

    fn image(&mut self, a: &Args) -> Result<Vec<Node>> {
        let file = a.str(0)?.ok_or_else(|| a.missing(0))?;
        let path = self.base_dir(a.call).join(&*file);
        let pixmap =
            load_image(&path).map_err(|m| Error::new(a.span(0), format!("image(): {m}")))?;
        let (iw, ih) = (f64::from(pixmap.width()), f64::from(pixmap.height()));
        // A single given dimension scales the other to keep the aspect ratio.
        let (w, h) = match a.vec2_opt(1)? {
            None | Some([None, None]) => (iw, ih),
            Some([Some(w), None]) => (w, w * ih / iw),
            Some([None, Some(h)]) => (h * iw / ih, h),
            Some([Some(w), Some(h)]) => (w, h),
        };
        let mut t = Transform::from_scale((w / iw) as f32, (h / ih) as f32);
        if a.bool(2)?.unwrap_or(false) {
            t = t.post_translate((-w / 2.0) as f32, (-h / 2.0) as f32);
        }
        Ok(vec![Node::Transform(t, vec![Node::Image(Rc::new(pixmap))])])
    }

    fn children(&mut self, a: &Args, env: &Rc<Env>, span: Span) -> Result<Vec<Node>> {
        let ctx = env
            .children()
            .ok_or_else(|| Error::new(span, "children() can only be used inside a module"))?;
        let select =
            match a.get(0) {
                None => None,
                Some((v, s)) => Some(
                    iterate(v)
                        .and_then(|items| {
                            items
                                .iter()
                                .map(|v| match v {
                                    Value::Num(n) if *n >= 0.0 => Ok(*n as usize),
                                    _ => Err("children(): index must be a non-negative number"
                                        .to_string()),
                                })
                                .collect::<std::result::Result<Vec<_>, _>>()
                        })
                        .map_err(|m| Error::new(*s, m))?,
                ),
            };
        // Children were written in the caller's scope, so they evaluate there.
        self.nested(span, |ev| {
            ev.eval_block(&ctx.body, &ctx.env.child(), select.as_deref())
        })
    }

    fn echo(&mut self, args: Vec<ArgVal>, children: &Body, env: &Rc<Env>) -> Result<Vec<Node>> {
        let parts: Vec<String> = args
            .iter()
            .map(|a| {
                let v = match &a.value {
                    Value::Str(s) => format!("{s:?}"),
                    v => v.to_string(),
                };
                match &a.name {
                    Some(n) => format!("{n} = {v}"),
                    None => v,
                }
            })
            .collect();
        self.log.push(format!("ECHO: {}", parts.join(", ")));
        self.eval_block(children, &env.child(), None)
    }

    fn filter(
        &mut self,
        def: &'static filters::FilterDef,
        args: Vec<ArgVal>,
        children: &Body,
        env: &Rc<Env>,
        span: Span,
    ) -> Result<Vec<Node>> {
        let what = format!("{}()", def.name);
        let names: Vec<&str> = def.params.iter().map(|(n, _)| *n).collect();
        let bound = bind_args(&what, &names, args)?;
        let mut nums = Vec::with_capacity(def.params.len());
        for ((pname, default), arg) in def.params.iter().zip(bound) {
            let n = match (arg, default) {
                (Some((Value::Num(n), _)), _) if n.is_finite() => n,
                (Some((v, s)), _) => {
                    let msg = format!(
                        "{what}: '{pname}' must be a finite number, got {}",
                        v.type_name()
                    );
                    return Err(Error::new(s, msg));
                }
                (None, Some(d)) => *d,
                (None, None) => {
                    return Err(Error::new(
                        span,
                        format!("{what}: missing argument '{pname}'"),
                    ));
                }
            };
            nums.push(n);
        }
        Ok(vec![Node::Layer {
            opacity: 1.0,
            filter: Some((def, nums)),
            span,
            children: self.eval_block(children, &env.child(), None)?,
        }])
    }

    // --------------------------------------------------------------------- expressions

    fn eval_expr(&mut self, e: &Expr, env: &Rc<Env>) -> Result<Value> {
        let at = |m: String| Error::new(e.span, m);
        Ok(match &e.kind {
            ExprKind::Num(n) => Value::Num(*n),
            ExprKind::Str(s) => Value::Str(s.clone()),
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Undef => Value::Undef,
            ExprKind::Var(name) => env
                .var(name)
                .ok_or_else(|| at(format!("unknown variable '{name}'")))?,
            ExprKind::List(elems) => {
                let mut out = Vec::new();
                for el in elems {
                    self.list_elem(el, env, &mut out)?;
                }
                Value::list(out)
            }
            ExprKind::Range { start, step, end } => {
                let mut num = |e: &Expr| match self.eval_expr(e, env)? {
                    Value::Num(n) => Ok(n),
                    other => Err(Error::new(
                        e.span,
                        format!("range bounds must be numbers, got {}", other.type_name()),
                    )),
                };
                let start = num(start)?;
                let step = match step {
                    Some(s) => num(s)?,
                    None => 1.0,
                };
                Value::Range {
                    start,
                    step,
                    end: num(end)?,
                }
            }
            ExprKind::Unary(op, x) => value::unary(*op, &self.eval_expr(x, env)?).map_err(at)?,
            ExprKind::Binary(BinOp::And, l, r) => {
                Value::Bool(self.eval_expr(l, env)?.truthy() && self.eval_expr(r, env)?.truthy())
            }
            ExprKind::Binary(BinOp::Or, l, r) => {
                Value::Bool(self.eval_expr(l, env)?.truthy() || self.eval_expr(r, env)?.truthy())
            }
            ExprKind::Binary(op, l, r) => {
                let (l, r) = (self.eval_expr(l, env)?, self.eval_expr(r, env)?);
                value::binary(*op, &l, &r).map_err(at)?
            }
            ExprKind::Ternary(c, t, f) => {
                let branch = if self.eval_expr(c, env)?.truthy() {
                    t
                } else {
                    f
                };
                self.eval_expr(branch, env)?
            }
            ExprKind::Call(name, args) => self.call_function(name, args, env, e.span)?,
            ExprKind::Index(base, i) => {
                let (base, i) = (self.eval_expr(base, env)?, self.eval_expr(i, env)?);
                value::index(&base, &i).map_err(at)?
            }
            ExprKind::Member(base, field) => {
                let i = match field.as_str() {
                    "x" => 0.0,
                    "y" => 1.0,
                    "z" => 2.0,
                    _ => return Err(at(format!("unknown member '.{field}' (use .x, .y or .z)"))),
                };
                value::index(&self.eval_expr(base, env)?, &Value::Num(i)).map_err(at)?
            }
            ExprKind::Let(bindings, body) => {
                let scope = self.bind_sequential(bindings, env)?;
                self.eval_expr(body, &scope)?
            }
        })
    }

    fn list_elem(&mut self, el: &ListElem, env: &Rc<Env>, out: &mut Vec<Value>) -> Result<()> {
        match el {
            ListElem::Expr(e) => out.push(self.eval_expr(e, env)?),
            ListElem::For(bindings, body) => {
                self.for_each(bindings, env, &mut |ev, scope| {
                    ev.list_elem(body, &scope, out)
                })?;
            }
            ListElem::If(cond, then, otherwise) => {
                if self.eval_expr(cond, env)?.truthy() {
                    self.list_elem(then, env, out)?;
                } else if let Some(otherwise) = otherwise {
                    self.list_elem(otherwise, env, out)?;
                }
            }
            ListElem::Let(bindings, body) => {
                let scope = self.bind_sequential(bindings, env)?;
                self.list_elem(body, &scope, out)?;
            }
            ListElem::Each(e) => {
                let v = self.eval_expr(e, env)?;
                out.extend(iterate(&v).map_err(|m| Error::new(e.span, m))?);
            }
        }
        Ok(())
    }

    fn call_function(
        &mut self,
        name: &str,
        args: &[Arg],
        env: &Rc<Env>,
        span: Span,
    ) -> Result<Value> {
        let args = self.eval_args(args, env)?;
        if let Some((def, def_env)) = env.lookup(name, |e| &e.functions) {
            return self.nested(span, |ev| {
                let frame = def_env.child();
                ev.bind_params(&format!("{name}()"), &def.params, args, &frame)?;
                ev.eval_expr(&def.body, &frame)
            });
        }
        let positional = args
            .into_iter()
            .map(|a| match a.name {
                Some(_) => Err(Error::new(
                    a.span,
                    format!("builtin {name}() takes no named arguments"),
                )),
                None => Ok(a.value),
            })
            .collect::<Result<Vec<_>>>()?;
        match functions::call(name, &positional) {
            Some(r) => r.map_err(|m| Error::new(span, format!("{name}(): {m}"))),
            None if env.lookup(name, |e| &e.modules).is_some() => Err(Error::new(
                span,
                format!("'{name}' is a module, not a function"),
            )),
            None => Err(Error::new(span, format!("unknown function '{name}'"))),
        }
    }
}

// ------------------------------------------------------------------------- helpers

/// The values a `for` loop or `each` visits.
fn iterate(v: &Value) -> std::result::Result<Vec<Value>, String> {
    match v {
        Value::List(items) => Ok(items.to_vec()),
        Value::Range { start, step, end } => Ok(range_values(*start, *step, *end)?
            .into_iter()
            .map(Value::Num)
            .collect()),
        Value::Str(s) => Ok(s
            .chars()
            .map(|c| Value::Str(c.to_string().into()))
            .collect()),
        Value::Num(_) => Ok(vec![v.clone()]),
        other => Err(format!("cannot iterate over {}", other.type_name())),
    }
}

/// A rectangle with corner radius `r` (cubic Bézier corners).
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    if r <= 0.0 {
        return Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect);
    }
    const K: f32 = 0.552_284_8; // control-point distance for a quarter circle
    let c = r * (1.0 - K);
    let (x1, y1) = (x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x1 - r, y);
    pb.cubic_to(x1 - c, y, x1, y + c, x1, y + r);
    pb.line_to(x1, y1 - r);
    pb.cubic_to(x1, y1 - c, x1 - c, y1, x1 - r, y1);
    pb.line_to(x + r, y1);
    pb.cubic_to(x + c, y1, x, y1 - c, x, y1 - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + c, x + c, y, x + r, y);
    pb.close();
    pb.finish()
}

fn parse_color(a: &Args) -> Result<Color> {
    let (v, span) = a.get(0).ok_or_else(|| a.missing(0))?;
    let alpha = a.num(1)?;
    color_from_value(v, alpha).map_err(|m| Error::new(*span, format!("color(): {m}")))
}

/// A CSS color string (`"red"`, `"#ff000080"`, `"hsl(...)"`) or `[r, g, b(, a)]` in 0..1.
fn color_from_value(v: &Value, alpha: Option<f64>) -> std::result::Result<Color, String> {
    let mut rgba = match v {
        Value::Str(s) => {
            let c = color::parse_color(s)
                .map_err(|e| format!("invalid color \"{s}\": {e}"))?
                .to_alpha_color::<color::Srgb>()
                .components;
            [c[0], c[1], c[2], c[3]]
        }
        Value::List(items) if items.len() == 3 || items.len() == 4 => {
            let mut c = [1.0; 4];
            for (slot, item) in c.iter_mut().zip(items.iter()) {
                match item {
                    Value::Num(n) if n.is_finite() => *slot = *n as f32,
                    _ => return Err("color components must be numbers".into()),
                }
            }
            c
        }
        other => {
            return Err(format!(
                "expected a color name or [r, g, b], got {}",
                other.type_name()
            ));
        }
    };
    if let Some(a) = alpha {
        rgba[3] = a as f32;
    }
    let [r, g, b, a] = rgba.map(|c| c.clamp(0.0, 1.0));
    Color::from_rgba(r, g, b, a).ok_or_else(|| "invalid color".into())
}

fn load_image(path: &Path) -> std::result::Result<Pixmap, String> {
    let img = image::open(path)
        .map_err(|e| format!("cannot load '{}': {e}", path.display()))?
        .into_rgba8();
    let (w, h) = img.dimensions();
    let mut data = img.into_raw();
    for px in data.as_chunks_mut::<4>().0 {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8; // tiny-skia stores premultiplied alpha
        }
    }
    IntSize::from_wh(w, h)
        .and_then(|size| Pixmap::from_vec(data, size))
        .ok_or_else(|| format!("'{}' has invalid dimensions", path.display()))
}
