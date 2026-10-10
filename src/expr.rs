//! Expressions: variable lookup, filters and conditions.

use crate::Value;
use crate::env::Env;
use crate::limits::MAX_NESTING;
use crate::parse::split_top;
use std::borrow::Cow;

pub(crate) fn value<'a>(expr: &str, env: &'a Env) -> Cow<'a, Value> {
    let e = expr.trim();
    if e.len() >= 2 && (e.starts_with('"') && e.ends_with('"') || e.starts_with('\'') && e.ends_with('\'')) {
        return Cow::Owned(Value::Str(e[1..e.len() - 1].into()));
    }
    match e {
        "true" => return Cow::Owned(Value::Bool(true)),
        "false" => return Cow::Owned(Value::Bool(false)),
        "nil" | "" => return Cow::Owned(Value::Nil),
        _ => {}
    }
    if let Ok(n) = e.parse::<f64>() {
        return Cow::Owned(Value::Num(n));
    }
    let mut parts = e.split('.');
    let mut cur = env.get(parts.next().unwrap()).map_or(Cow::Owned(Value::Nil), Cow::Borrowed);
    for key in parts {
        // An owned value here can only be Num or Nil: it has no sub-keys.
        let Cow::Borrowed(v) = cur else { return Cow::Owned(Value::Nil) };
        cur = match (v, key) {
            (Value::Map(m), k) if m.contains_key(k) => Cow::Borrowed(&m[k]),
            (v, "size") if v.len().is_some() => Cow::Owned(Value::Num(v.len().unwrap() as f64)),
            (Value::List(l), k) => k.parse::<usize>().ok().and_then(|n| l.get(n)).map_or(Cow::Owned(Value::Nil), Cow::Borrowed),
            _ => Cow::Owned(Value::Nil),
        };
    }
    cur
}

pub(crate) fn apply_filter(name: &str, v: &Value, args: &[Value]) -> Result<Value, String> {
    let s = || v.show();
    Ok(match name {
        "uppercase" => Value::Str(s().to_uppercase()),
        "lowercase" => Value::Str(s().to_lowercase()),
        "capitalize" => {
            let s = s().to_lowercase();
            let mut c = s.chars();
            Value::Str(c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default())
        }
        "size" => Value::Num(v.len().unwrap_or(0) as f64),
        "join" => {
            let sep = args.first().map(Value::show).unwrap_or(" ".into());
            match v {
                Value::List(l) => Value::Str(l.iter().map(Value::show).collect::<Vec<_>>().join(&sep)),
                _ => v.clone(),
            }
        }
        "default" => {
            if v.truthy() && *v != Value::Str(String::new()) { v.clone() } else { args.first().cloned().unwrap_or(Value::Nil) }
        }
        "escape" => Value::Str(
            s().chars()
                .map(|c| match c {
                    '&' | '<' | '>' | '"' | '\'' => format!("&#{};", c as u32),
                    c => c.to_string(),
                })
                .collect(),
        ),
        _ => return Err(format!("Unknown filter: {name}")),
    })
}

pub(crate) fn output<'a>(expr: &str, env: &'a Env) -> Result<Cow<'a, Value>, String> {
    let mut pipes = split_top(expr, '|').into_iter();
    let mut v = value(pipes.next().unwrap(), env);
    for p in pipes {
        let (name, args) = p.split_once(':').unwrap_or((p, ""));
        let args: Vec<Value> = if args.trim().is_empty() {
            vec![]
        } else {
            split_top(args, ',').iter().map(|a| value(a, env).into_owned()).collect()
        };
        v = Cow::Owned(apply_filter(name.trim(), &v, &args)?);
    }
    Ok(v)
}

pub(crate) fn compare(a: &Value, op: &str, b: &Value) -> bool {
    match op {
        "==" => a == b,
        "!=" => a != b,
        "contains" => match (a, b) {
            (Value::Str(x), Value::Str(y)) => x.contains(y.as_str()),
            (Value::List(l), b) => l.contains(b),
            _ => false,
        },
        _ => {
            use std::cmp::Ordering::*;
            let ord = match (a, b) {
                (Value::Num(x), Value::Num(y)) => x.partial_cmp(y),
                (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
                _ => None,
            };
            matches!((op, ord), ("<", Some(Less)) | (">", Some(Greater)) | ("<=", Some(Less | Equal)) | (">=", Some(Greater | Equal)))
        }
    }
}

/// Splits a condition into tokens: quoted strings, parentheses, operators and words.
pub(crate) fn cond_tokens(s: &str) -> Result<Vec<String>, String> {
    let (mut toks, mut it) = (Vec::new(), s.char_indices().peekable());
    while let Some(&(i, c)) = it.peek() {
        let mut end = i + c.len_utf8();
        it.next();
        match c {
            c if c.is_whitespace() => continue,
            '(' | ')' => {}
            '"' | '\'' => loop {
                let (j, d) = it.next().ok_or("Unclosed string in condition")?;
                end = j + d.len_utf8();
                if d == c { break; }
            },
            '=' | '!' | '<' | '>' => {
                if let Some(&(j, '=')) = it.peek() {
                    end = j + 1;
                    it.next();
                }
            }
            _ => {
                while let Some(&(j, d)) = it.peek() {
                    if d.is_whitespace() || "()=!<>\"'".contains(d) { break; }
                    end = j + d.len_utf8();
                    it.next();
                }
            }
        }
        toks.push(s[i..end].to_string());
    }
    Ok(toks)
}

/// Grammar: or := and ("or" and)* ; and := cmp ("and" cmp)* ; cmp := atom [op atom] ; atom := "(" or ")" | value.
pub(crate) struct Cond<'a> {
    toks: Vec<String>,
    i: usize,
    depth: usize,
    env: &'a Env<'a>,
}

impl<'a> Cond<'a> {
    fn peek(&self) -> Option<&str> {
        self.toks.get(self.i).map(String::as_str)
    }

    fn or(&mut self) -> Result<Cow<'a, Value>, String> {
        let mut v = self.and()?;
        while self.peek() == Some("or") {
            self.i += 1;
            let r = self.and()?;
            v = Cow::Owned(Value::Bool(v.truthy() || r.truthy()));
        }
        Ok(v)
    }

    fn and(&mut self) -> Result<Cow<'a, Value>, String> {
        let mut v = self.cmp()?;
        while self.peek() == Some("and") {
            self.i += 1;
            let r = self.cmp()?;
            v = Cow::Owned(Value::Bool(v.truthy() && r.truthy()));
        }
        Ok(v)
    }

    fn cmp(&mut self) -> Result<Cow<'a, Value>, String> {
        let a = self.atom()?;
        match self.peek() {
            Some(op @ ("==" | "!=" | "<" | ">" | "<=" | ">=" | "contains")) => {
                let op = op.to_string();
                self.i += 1;
                let b = self.atom()?;
                Ok(Cow::Owned(Value::Bool(compare(&a, &op, &b))))
            }
            _ => Ok(a),
        }
    }

    fn atom(&mut self) -> Result<Cow<'a, Value>, String> {
        let t = self.toks.get(self.i).ok_or("Incomplete condition")?.clone();
        self.i += 1;
        if t == "(" {
            self.depth += 1;
            if self.depth > MAX_NESTING {
                return Err(format!("Condition nested too deeply (max {MAX_NESTING} levels)"));
            }
            let v = self.or()?;
            self.depth -= 1;
            if self.peek() != Some(")") {
                return Err("Invalid condition: missing ')'".into());
            }
            self.i += 1;
            return Ok(v);
        }
        Ok(value(&t, self.env))
    }
}

pub(crate) fn condition(expr: &str, env: &Env) -> Result<bool, String> {
    let mut c = Cond { toks: cond_tokens(expr)?, i: 0, depth: 0, env };
    let v = c.or()?;
    if c.i < c.toks.len() {
        return Err(format!("Invalid condition: '{expr}'"));
    }
    Ok(v.truthy())
}
