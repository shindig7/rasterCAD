//! Builtin functions. Angles are in degrees, as in OpenSCAD.

use super::ast::BinOp;
use super::value::{self, Value, range_values};

/// Calls builtin `name`, or returns `None` if there is no such builtin.
pub fn call(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let math1 =
        |f: fn(f64) -> f64| -> Result<Value, String> { Ok(Value::Num(f(num_arg(args, 1)?[0]))) };
    Some(match name {
        "sin" => math1(|x| x.to_radians().sin()),
        "cos" => math1(|x| x.to_radians().cos()),
        "tan" => math1(|x| x.to_radians().tan()),
        "asin" => math1(|x| x.asin().to_degrees()),
        "acos" => math1(|x| x.acos().to_degrees()),
        "atan" => math1(|x| x.atan().to_degrees()),
        "abs" => math1(f64::abs),
        "sqrt" => math1(f64::sqrt),
        "exp" => math1(f64::exp),
        "ln" => math1(f64::ln),
        "log" => math1(f64::log10),
        "floor" => math1(f64::floor),
        "ceil" => math1(f64::ceil),
        "round" => math1(f64::round),
        "sign" => math1(|x| if x == 0.0 { 0.0 } else { x.signum() }),
        "atan2" => num_arg(args, 2).map(|a| Value::Num(a[0].atan2(a[1]).to_degrees())),
        "pow" => num_arg(args, 2).map(|a| Value::Num(a[0].powf(a[1]))),
        "min" | "max" => min_max(name == "min", args),
        "norm" => norm(args),
        "in_to_px" => inches(BinOp::Mul, args),
        "px_to_in" => inches(BinOp::Div, args),
        "len" => len(args),
        "concat" => Ok(concat(args)),
        "str" => Ok(Value::Str(
            args.iter()
                .map(ToString::to_string)
                .collect::<String>()
                .into(),
        )),
        "chr" => chr(args),
        "ord" => ord(args),
        "is_undef" => is(args, |v| matches!(v, Value::Undef)),
        "is_num" => is(args, |v| matches!(v, Value::Num(n) if !n.is_nan())),
        "is_string" => is(args, |v| matches!(v, Value::Str(_))),
        "is_list" => is(args, |v| matches!(v, Value::List(_))),
        "is_bool" => is(args, |v| matches!(v, Value::Bool(_))),
        _ => return None,
    })
}

fn arity(args: &[Value], n: usize) -> Result<(), String> {
    if args.len() == n {
        Ok(())
    } else {
        Err(format!("expected {n} argument(s), got {}", args.len()))
    }
}

fn num_arg(args: &[Value], n: usize) -> Result<Vec<f64>, String> {
    arity(args, n)?;
    args.iter()
        .map(|v| match v {
            Value::Num(x) => Ok(*x),
            other => Err(format!("expected a number, got {}", other.type_name())),
        })
        .collect()
}

fn is(args: &[Value], pred: fn(&Value) -> bool) -> Result<Value, String> {
    arity(args, 1)?;
    Ok(Value::Bool(pred(&args[0])))
}

fn min_max(is_min: bool, args: &[Value]) -> Result<Value, String> {
    let items: &[Value] = match args {
        [Value::List(items)] => items,
        _ => args,
    };
    let nums = num_arg(items, items.len())?;
    let pick = if is_min { f64::min } else { f64::max };
    nums.into_iter()
        .reduce(pick)
        .map(Value::Num)
        .ok_or_else(|| "needs at least one number".into())
}

fn norm(args: &[Value]) -> Result<Value, String> {
    arity(args, 1)?;
    let Value::List(items) = &args[0] else {
        return Err("expected a list of numbers".into());
    };
    let nums = num_arg(items, items.len())?;
    Ok(Value::Num(nums.iter().map(|x| x * x).sum::<f64>().sqrt()))
}

/// Inches to pixels (`Mul`) or back (`Div`) at a given PPI. Works on numbers and lists.
fn inches(op: BinOp, args: &[Value]) -> Result<Value, String> {
    arity(args, 2)?;
    match &args[1] {
        Value::Num(ppi) if ppi.is_finite() && *ppi > 0.0 => value::binary(op, &args[0], &args[1]),
        _ => Err("ppi must be a positive number".into()),
    }
}

fn len(args: &[Value]) -> Result<Value, String> {
    arity(args, 1)?;
    let n = match &args[0] {
        Value::List(items) => items.len(),
        Value::Str(s) => s.chars().count(),
        Value::Range { start, step, end } => range_values(*start, *step, *end)?.len(),
        other => return Err(format!("cannot take the length of {}", other.type_name())),
    };
    Ok(Value::Num(n as f64))
}

fn concat(args: &[Value]) -> Value {
    let mut out = Vec::new();
    for a in args {
        match a {
            Value::List(items) => out.extend(items.iter().cloned()),
            other => out.push(other.clone()),
        }
    }
    Value::list(out)
}

fn chr(args: &[Value]) -> Result<Value, String> {
    let codes: Vec<f64> = match args {
        [Value::List(items)] => num_arg(items, items.len())?,
        _ => num_arg(args, args.len())?,
    };
    codes
        .iter()
        .map(|&c| {
            char::from_u32(c as u32)
                .filter(|_| c >= 0.0 && c.fract() == 0.0)
                .ok_or_else(|| format!("{c} is not a valid character code"))
        })
        .collect::<Result<String, _>>()
        .map(|s| Value::Str(s.into()))
}

fn ord(args: &[Value]) -> Result<Value, String> {
    arity(args, 1)?;
    match &args[0] {
        Value::Str(s) if s.chars().count() == 1 => {
            Ok(Value::Num(f64::from(s.chars().next().map_or(0, u32::from))))
        }
        _ => Err("expected a single-character string".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(x: f64) -> Value {
        Value::Num(x)
    }

    #[test]
    fn builtins() {
        assert_eq!(call("max", &[n(1.0), n(3.0), n(2.0)]), Some(Ok(n(3.0))));
        assert_eq!(
            call("min", &[Value::list(vec![n(4.0), n(2.0)])]),
            Some(Ok(n(2.0)))
        );
        assert_eq!(
            call("norm", &[Value::list(vec![n(3.0), n(4.0)])]),
            Some(Ok(n(5.0)))
        );
        assert_eq!(
            call("str", &[Value::Str("a".into()), n(1.0)]),
            Some(Ok(Value::Str("a1".into())))
        );
        assert!((call("sin", &[n(90.0)]).unwrap().unwrap() == n(1.0)));
        assert_eq!(call("in_to_px", &[n(8.5), n(300.0)]), Some(Ok(n(2550.0))));
        assert_eq!(
            call(
                "px_to_in",
                &[Value::list(vec![n(600.0), n(150.0)]), n(300.0)]
            ),
            Some(Ok(Value::list(vec![n(2.0), n(0.5)])))
        );
        assert!(call("in_to_px", &[n(1.0), n(0.0)]).unwrap().is_err());
        assert!(call("nope", &[]).is_none());
        assert!(call("sqrt", &[]).unwrap().is_err());
    }
}
