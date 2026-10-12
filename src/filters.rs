//! Filters: `{{ value | name: arg, ... }}`.
//!
//! Filters that can make their result much bigger than their input (`replace`, `join`,
//! `split`, `append`, ...) check the size first, so a hostile template can't make them
//! allocate without bound.

use crate::Value;
use crate::limits::MAX_VALUE_SIZE;
use std::cmp::Ordering;
use std::collections::HashSet;

fn check(bytes: usize) -> Result<(), String> {
    if bytes > MAX_VALUE_SIZE {
        return Err(format!("Value too large (max {MAX_VALUE_SIZE} bytes)"));
    }
    Ok(())
}

fn text(args: &[Value], i: usize, filter: &str) -> Result<String, String> {
    args.get(i).map(Value::show).ok_or_else(|| format!("{filter} needs an argument"))
}

/// Numbers, numeric strings, and `nil` (as 0).
fn number(v: &Value, filter: &str) -> Result<f64, String> {
    let n = match v {
        Value::Num(n) => Some(*n),
        Value::Nil => Some(0.0),
        Value::Str(s) => s.trim().parse().ok(),
        _ => None,
    };
    n.filter(|n| n.is_finite()).ok_or_else(|| format!("{filter} expects a number"))
}

fn num_arg(args: &[Value], i: usize, filter: &str) -> Result<f64, String> {
    number(args.get(i).ok_or_else(|| format!("{filter} needs an argument"))?, filter)
}

fn finite(n: f64) -> Result<Value, String> {
    if n.is_finite() { Ok(Value::Num(n)) } else { Err("Number out of range".into()) }
}

/// A non-negative whole number argument, as a count.
fn count(args: &[Value], i: usize, filter: &str) -> Result<usize, String> {
    let n = num_arg(args, i, filter)?;
    if n < 0.0 || n > 1e9 {
        return Err(format!("{filter} expects a non-negative number"));
    }
    Ok(n as usize)
}

/// Filters supplied by the host program, tried before the built-in ones (so they can also replace them).
///
/// `call` returns `None` for a name it doesn't know. The host is trusted code that runs
/// synchronously inside the render: it must not block for long, and its result is subject to
/// the same size limits as any other value. A `HashMap<String, F>` of closures is a `FilterSet`.
pub trait FilterSet {
    fn call(&self, name: &str, value: &Value, args: &[Value]) -> Option<Result<Value, String>>;

    /// Functions, called as `{{ name(arg1, arg2) }}`. Same rules as `call`; there are no built-in functions.
    fn function(&self, _name: &str, _args: &[Value]) -> Option<Result<Value, String>> {
        None
    }
}

impl<F: Fn(&Value, &[Value]) -> Result<Value, String>> FilterSet for std::collections::HashMap<String, F> {
    fn call(&self, name: &str, value: &Value, args: &[Value]) -> Option<Result<Value, String>> {
        self.get(name).map(|f| f(value, args))
    }
}

/// No host filters.
pub(crate) struct NoFilters;

impl FilterSet for NoFilters {
    fn call(&self, _: &str, _: &Value, _: &[Value]) -> Option<Result<Value, String>> {
        None
    }
}

pub(crate) fn apply_filter(name: &str, v: &Value, args: &[Value]) -> Result<Value, String> {
    let s = || v.show();
    let out = match name {
        // Text
        "uppercase" => Value::Str(s().to_uppercase()),
        "lowercase" => Value::Str(s().to_lowercase()),
        "capitalize" => {
            let s = s().to_lowercase();
            let mut c = s.chars();
            Value::Str(c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default())
        }
        "strip" => Value::Str(s().trim().to_string()),
        "append" | "prepend" => {
            let (base, extra) = (s(), text(args, 0, name)?);
            check(base.len() + extra.len())?;
            Value::Str(if name == "append" { base + &extra } else { extra + &base })
        }
        "truncate" => {
            let n = count(args, 0, name)?;
            let ellipsis = args.get(1).map(Value::show).unwrap_or_else(|| "...".into());
            let t = s();
            if t.chars().count() <= n {
                Value::Str(t)
            } else {
                let keep = n.saturating_sub(ellipsis.chars().count());
                Value::Str(t.chars().take(keep).chain(ellipsis.chars()).collect())
            }
        }
        "replace" => {
            let (old, new) = (text(args, 0, name)?, text(args, 1, name)?);
            if old.is_empty() {
                return Err("replace needs a non-empty text to look for".into());
            }
            let t = s();
            let found = t.matches(&old).count();
            check(t.len() - found * old.len() + found * new.len())?;
            Value::Str(t.replace(&old, &new))
        }
        "split" => {
            let sep = text(args, 0, name)?;
            let t = s();
            let items = if sep.is_empty() { t.chars().count() } else { t.matches(&sep).count() + 1 };
            check(24 + items * 24 + t.len())?;
            Value::List(if sep.is_empty() {
                t.chars().map(|c| Value::Str(c.to_string())).collect()
            } else {
                t.split(&sep).map(|p| Value::Str(p.into())).collect()
            })
        }
        "url_encode" => {
            let t = s();
            check(t.len() * 3)?;
            let mut out = String::with_capacity(t.len());
            for b in t.bytes() {
                match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
                    _ => out.push_str(&format!("%{b:02X}")),
                }
            }
            Value::Str(out)
        }
        "escape" => Value::Str(
            s().chars()
                .map(|c| match c {
                    '&' | '<' | '>' | '"' | '\'' => format!("&#{};", c as u32),
                    c => c.to_string(),
                })
                .collect(),
        ),
        "default" => {
            if v.truthy() && *v != Value::Str(String::new()) { v.clone() } else { args.first().cloned().unwrap_or(Value::Nil) }
        }
        "size" => Value::Num(v.len().unwrap_or(0) as f64),

        // Numbers
        "plus" | "minus" | "times" | "divided_by" => {
            let (a, b) = (number(v, name)?, num_arg(args, 0, name)?);
            match name {
                "plus" => finite(a + b)?,
                "minus" => finite(a - b)?,
                "times" => finite(a * b)?,
                _ if b == 0.0 => return Err("Division by zero".into()),
                _ => finite(a / b)?,
            }
        }
        "round" => {
            let digits = match args.first() {
                Some(d) => number(d, name)?,
                None => 0.0,
            };
            if !(0.0..=10.0).contains(&digits) {
                return Err("round expects 0 to 10 digits".into());
            }
            let f = 10f64.powi(digits as i32);
            finite((number(v, name)? * f).round() / f)?
        }
        "floor" => finite(number(v, name)?.floor())?,
        "ceil" => finite(number(v, name)?.ceil())?,

        // Lists
        "join" => {
            let sep = args.first().map(Value::show).unwrap_or_else(|| " ".into());
            match v {
                Value::List(l) => {
                    let parts: Vec<String> = l.iter().map(Value::show).collect();
                    check(parts.iter().map(String::len).sum::<usize>() + sep.len() * parts.len().saturating_sub(1))?;
                    Value::Str(parts.join(&sep))
                }
                _ => v.clone(),
            }
        }
        "first" | "last" => match v {
            Value::List(l) => (if name == "first" { l.first() } else { l.last() }).cloned().unwrap_or(Value::Nil),
            Value::Str(t) => (if name == "first" { t.chars().next() } else { t.chars().last() })
                .map_or(Value::Nil, |c| Value::Str(c.to_string())),
            _ => Value::Nil,
        },
        "reverse" => match v {
            Value::List(l) => Value::List(l.iter().rev().cloned().collect()),
            _ => v.clone(),
        },
        "sort" => match v {
            Value::List(l) => sort(l, args.first().map(Value::show).as_deref())?,
            _ => v.clone(),
        },
        "uniq" => match v {
            Value::List(l) => {
                let mut seen = HashSet::new();
                Value::List(l.iter().filter(|x| seen.insert(canon(x))).cloned().collect())
            }
            _ => v.clone(),
        },
        "map" => {
            let key = text(args, 0, name)?;
            match v {
                Value::List(l) => Value::List(
                    l.iter().map(|x| if let Value::Map(m) = x { m.get(&key).cloned().unwrap_or(Value::Nil) } else { Value::Nil }).collect(),
                ),
                Value::Nil => Value::List(vec![]),
                _ => return Err("map expects a list".into()),
            }
        }

        // Dates
        "date" => {
            let secs = number(v, name)?.floor();
            if !(-62_135_596_800.0..=253_402_300_799.0).contains(&secs) {
                return Err("date out of range (years 1 to 9999)".into());
            }
            let fmt = args.first().map(Value::show).unwrap_or_else(|| "%Y-%m-%d".into());
            Value::Str(format_date(secs as i64, &fmt))
        }
        _ => return Err(format!("Unknown filter: {name}")),
    };
    check(out.weight())?;
    Ok(out)
}

enum Key {
    Num(f64),
    Str(String),
}

/// Sorts numbers or strings (optionally by a key of each map). Everything must be of one kind,
/// so the comparison is a total order (sorting with an inconsistent order can panic).
fn sort(items: &[Value], key: Option<&str>) -> Result<Value, String> {
    let nil = Value::Nil;
    let keys = items
        .iter()
        .map(|x| match (key, x) {
            (None, x) => Ok(x),
            (Some(k), Value::Map(m)) => Ok(m.get(k).unwrap_or(&nil)),
            _ => Err("sort with a key expects a list of maps".to_string()),
        })
        .map(|k| match k? {
            Value::Num(n) if !n.is_nan() => Ok(Key::Num(*n)),
            Value::Str(s) => Ok(Key::Str(s.clone())),
            _ => Err("sort expects numbers or strings, all of the same kind".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !(keys.iter().all(|k| matches!(k, Key::Num(_))) || keys.iter().all(|k| matches!(k, Key::Str(_)))) {
        return Err("sort expects numbers or strings, all of the same kind".into());
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| match (&keys[a], &keys[b]) {
        (Key::Num(x), Key::Num(y)) => x.total_cmp(y),
        (Key::Str(x), Key::Str(y)) => x.cmp(y),
        _ => Ordering::Equal,
    });
    Ok(Value::List(order.into_iter().map(|i| items[i].clone()).collect()))
}

/// A string that is the same for equal values (maps are ordered by key), used by `uniq`.
fn canon(v: &Value) -> String {
    match v {
        Value::Nil => "n".into(),
        Value::Bool(b) => format!("b{b}"),
        Value::Num(n) => format!("#{:?}", if *n == 0.0 { 0.0 } else { *n }),
        Value::Str(s) => format!("s{}:{s}", s.len()),
        Value::List(l) => format!("l[{}]", l.iter().map(canon).collect::<Vec<_>>().join(",")),
        Value::Map(m) => {
            let mut entries: Vec<_> = m.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            format!("m{{{}}}", entries.iter().map(|(k, v)| format!("{}:{k}={}", k.len(), canon(v))).collect::<Vec<_>>().join(","))
        }
    }
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian calendar.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// Formats a Unix timestamp (UTC) with a subset of strftime:
/// `%Y %y %m %d %e %H %I %M %S %p %j %B %b %A %a %F %T %s %%`.
fn format_date(ts: i64, fmt: &str) -> String {
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let (days, secs) = (ts.div_euclid(86_400), ts.rem_euclid(86_400));
    let (y, m, d) = civil(days);
    let (hour, min, sec) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let weekday = (days + 4).rem_euclid(7) as usize;
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&format!("{y:04}")),
            Some('y') => out.push_str(&format!("{:02}", y.rem_euclid(100))),
            Some('m') => out.push_str(&format!("{m:02}")),
            Some('d') => out.push_str(&format!("{d:02}")),
            Some('e') => out.push_str(&format!("{d:>2}")),
            Some('H') => out.push_str(&format!("{hour:02}")),
            Some('I') => out.push_str(&format!("{:02}", if hour % 12 == 0 { 12 } else { hour % 12 })),
            Some('M') => out.push_str(&format!("{min:02}")),
            Some('S') => out.push_str(&format!("{sec:02}")),
            Some('p') => out.push_str(if hour < 12 { "AM" } else { "PM" }),
            Some('j') => out.push_str(&format!("{:03}", days - days_from_civil(y, 1, 1) + 1)),
            Some('B') => out.push_str(MONTHS[m as usize - 1]),
            Some('b') => out.push_str(&MONTHS[m as usize - 1][..3]),
            Some('A') => out.push_str(DAYS[weekday]),
            Some('a') => out.push_str(&DAYS[weekday][..3]),
            Some('F') => out.push_str(&format!("{y:04}-{m:02}-{d:02}")),
            Some('T') => out.push_str(&format!("{hour:02}:{min:02}:{sec:02}")),
            Some('s') => out.push_str(&ts.to_string()),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}
