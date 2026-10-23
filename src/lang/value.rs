//! Runtime values.

use std::fmt;
use std::rc::Rc;

use super::ast::{BinOp, UnOp};

/// Upper bound on elements produced by expanding a range, to catch runaway loops.
pub const MAX_RANGE_LEN: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Undef,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    List(Rc<[Value]>),
    Range { start: f64, step: f64, end: f64 },
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Undef => "undef",
            Value::Bool(_) => "bool",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::List(_) => "list",
            Value::Range { .. } => "range",
        }
    }

    /// OpenSCAD truthiness: undef, false, 0, "" and [] are false.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Undef => false,
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Range { .. } => true,
        }
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List(items.into())
    }
}

/// Expands a range into its values, end inclusive.
pub fn range_values(start: f64, step: f64, end: f64) -> Result<Vec<f64>, String> {
    if !(start.is_finite() && step.is_finite() && end.is_finite()) {
        return Err("range bounds must be finite".into());
    }
    if step == 0.0 {
        return Err("range step must not be 0".into());
    }
    let count = ((end - start) / step + 1e-9).floor();
    if count < 0.0 {
        return Ok(Vec::new());
    }
    if count >= MAX_RANGE_LEN as f64 {
        return Err(format!("range has more than {MAX_RANGE_LEN} elements"));
    }
    Ok((0..=count as usize)
        .map(|i| start + step * i as f64)
        .collect())
}

/// Applies a binary operator (except the short-circuiting `&&`/`||`, handled by the evaluator).
pub fn binary(op: BinOp, a: &Value, b: &Value) -> Result<Value, String> {
    use Value::{Bool, List, Num, Str};
    Ok(match (op, a, b) {
        (BinOp::Eq, a, b) => Bool(a == b),
        (BinOp::Ne, a, b) => Bool(a != b),
        (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, a, b) => {
            let ord = match (a, b) {
                (Num(x), Num(y)) => x.partial_cmp(y),
                (Str(x), Str(y)) => Some(x.cmp(y)),
                _ => None,
            };
            let Some(ord) = ord else {
                return Err(mismatch(op, a, b));
            };
            Bool(match op {
                BinOp::Lt => ord.is_lt(),
                BinOp::Le => ord.is_le(),
                BinOp::Gt => ord.is_gt(),
                _ => ord.is_ge(),
            })
        }
        (_, Num(x), Num(y)) => Num(match op {
            BinOp::Add => x + y,
            BinOp::Sub => x - y,
            BinOp::Mul => x * y,
            BinOp::Div | BinOp::Mod if *y == 0.0 => return Err("division by zero".into()),
            BinOp::Div => x / y,
            BinOp::Mod => x % y,
            BinOp::Pow => x.powf(*y),
            _ => return Err(mismatch(op, a, b)),
        }),
        (BinOp::Add | BinOp::Sub, List(x), List(y)) if x.len() == y.len() => Value::list(
            x.iter()
                .zip(y.iter())
                .map(|(x, y)| binary(op, x, y))
                .collect::<Result<_, _>>()?,
        ),
        (BinOp::Mul, List(x), List(y)) if x.len() == y.len() => {
            let mut dot = 0.0;
            for (x, y) in x.iter().zip(y.iter()) {
                match (x, y) {
                    (Num(x), Num(y)) => dot += x * y,
                    _ => return Err("dot product needs two lists of numbers".into()),
                }
            }
            Num(dot)
        }
        (BinOp::Mul, Num(_), List(items)) => Value::list(
            items
                .iter()
                .map(|v| binary(op, a, v))
                .collect::<Result<_, _>>()?,
        ),
        (BinOp::Mul | BinOp::Div, List(items), Num(_)) => Value::list(
            items
                .iter()
                .map(|v| binary(op, v, b))
                .collect::<Result<_, _>>()?,
        ),
        (BinOp::Add, Str(_), Str(_)) => {
            return Err("cannot add strings; use str(a, b) to join them".into());
        }
        _ => return Err(mismatch(op, a, b)),
    })
}

fn mismatch(op: BinOp, a: &Value, b: &Value) -> String {
    let sym = match op {
        BinOp::Or => "||",
        BinOp::And => "&&",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "^",
    };
    let detail = match (a, b) {
        (Value::List(x), Value::List(y)) if x.len() != y.len() => {
            format!(" of different lengths ({} and {})", x.len(), y.len())
        }
        _ => String::new(),
    };
    format!(
        "cannot apply '{sym}' to {} and {}{detail}",
        a.type_name(),
        b.type_name()
    )
}

pub fn unary(op: UnOp, v: &Value) -> Result<Value, String> {
    match (op, v) {
        (UnOp::Not, v) => Ok(Value::Bool(!v.truthy())),
        (UnOp::Plus, Value::Num(_)) => Ok(v.clone()),
        (UnOp::Neg, Value::Num(n)) => Ok(Value::Num(-n)),
        (UnOp::Neg, Value::List(items)) => Ok(Value::list(
            items
                .iter()
                .map(|v| unary(op, v))
                .collect::<Result<_, _>>()?,
        )),
        _ => Err(format!("cannot negate {}", v.type_name())),
    }
}

pub fn index(base: &Value, i: &Value) -> Result<Value, String> {
    let Value::Num(n) = i else {
        return Err(format!("index must be a number, got {}", i.type_name()));
    };
    let len = match base {
        Value::List(items) => items.len(),
        Value::Str(s) => s.chars().count(),
        other => return Err(format!("cannot index into {}", other.type_name())),
    };
    let idx = n.floor();
    if idx < 0.0 || idx >= len as f64 {
        return Err(format!(
            "index {} out of range for length {len}",
            fmt_num(*n)
        ));
    }
    let idx = idx as usize;
    Ok(match base {
        Value::List(items) => items[idx].clone(),
        Value::Str(s) => Value::Str(
            s.chars()
                .nth(idx)
                .map(String::from)
                .unwrap_or_default()
                .into(),
        ),
        _ => unreachable!("checked above"),
    })
}

pub fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Undef => f.write_str("undef"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Num(n) => f.write_str(&fmt_num(*n)),
            Value::Str(s) => f.write_str(s),
            Value::List(items) => {
                f.write_str("[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    match v {
                        Value::Str(s) => write!(f, "{s:?}")?,
                        v => write!(f, "{v}")?,
                    }
                }
                f.write_str("]")
            }
            Value::Range { start, step, end } => {
                write!(
                    f,
                    "[{}:{}:{}]",
                    fmt_num(*start),
                    fmt_num(*step),
                    fmt_num(*end)
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(
            range_values(0.0, 1.0, 3.0).unwrap(),
            vec![0.0, 1.0, 2.0, 3.0]
        );
        assert_eq!(range_values(0.0, 0.1, 0.3).unwrap().len(), 4);
        assert!(range_values(3.0, 1.0, 0.0).unwrap().is_empty());
        assert!(range_values(0.0, 0.0, 1.0).is_err());
    }

    #[test]
    fn arithmetic() {
        let v = |n: &[f64]| Value::list(n.iter().map(|&n| Value::Num(n)).collect());
        assert_eq!(
            binary(BinOp::Add, &v(&[1.0, 2.0]), &v(&[3.0, 4.0])),
            Ok(v(&[4.0, 6.0]))
        );
        assert_eq!(
            binary(BinOp::Mul, &Value::Num(2.0), &v(&[1.0, 2.0])),
            Ok(v(&[2.0, 4.0]))
        );
        assert_eq!(
            binary(BinOp::Mul, &v(&[1.0, 2.0]), &v(&[3.0, 4.0])),
            Ok(Value::Num(11.0))
        );
        assert!(binary(BinOp::Div, &Value::Num(1.0), &Value::Num(0.0)).is_err());
        assert_eq!(
            index(&v(&[5.0, 6.0]), &Value::Num(1.0)),
            Ok(Value::Num(6.0))
        );
        assert!(index(&v(&[5.0]), &Value::Num(1.0)).is_err());
    }

    #[test]
    fn display() {
        let v = Value::list(vec![
            Value::Num(1.0),
            Value::Num(0.5),
            Value::Str("a".into()),
        ]);
        assert_eq!(v.to_string(), r#"[1, 0.5, "a"]"#);
    }
}
