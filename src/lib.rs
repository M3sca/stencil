//! Minimal template engine:
//! `{{ var | filter: arg }}`, `{% if/elsif/else %}`, `{% for x in xs %}`.

#[cfg(feature = "node")]
mod node;

use std::borrow::Cow;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Value>),
    Map(HashMap<String, Value>),
}

impl From<&str> for Value { fn from(s: &str) -> Self { Value::Str(s.into()) } }
impl From<i64> for Value { fn from(n: i64) -> Self { Value::Num(n as f64) } }
impl From<bool> for Value { fn from(b: bool) -> Self { Value::Bool(b) } }
impl From<Vec<Value>> for Value { fn from(v: Vec<Value>) -> Self { Value::List(v) } }

pub type Context = HashMap<String, Value>;

impl Value {
    fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    fn show(&self) -> String {
        match self {
            Value::Nil => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => n.to_string(),
            Value::Str(s) => s.clone(),
            Value::List(l) => l.iter().map(Value::show).collect(),
            Value::Map(_) => String::new(),
        }
    }

    fn len(&self) -> Option<usize> {
        match self {
            Value::Str(s) => Some(s.chars().count()),
            Value::List(l) => Some(l.len()),
            Value::Map(m) => Some(m.len()),
            _ => None,
        }
    }
}

enum Tok {
    Text(String),
    Out(String),
    Tag(String),
}

enum Node {
    Text(String),
    Out(String),
    If(Vec<(String, Vec<Node>)>),
    For { var: String, list: String, body: Vec<Node> },
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let mut rest = src;
    while let Some(pos) = [rest.find("{{"), rest.find("{%")].into_iter().flatten().min() {
        if pos > 0 {
            toks.push(Tok::Text(rest[..pos].into()));
        }
        let (close, is_out) = if rest[pos..].starts_with("{{") { ("}}", true) } else { ("%}", false) };
        let inner = &rest[pos + 2..];
        let end = inner.find(close).ok_or(format!("Unclosed delimiter: missing '{close}'"))?;
        let body = inner[..end].trim().to_string();
        toks.push(if is_out { Tok::Out(body) } else { Tok::Tag(body) });
        rest = &inner[end + 2..];
    }
    if !rest.is_empty() {
        toks.push(Tok::Text(rest.into()));
    }
    Ok(toks)
}

/// Reads nodes until one of the tags in `stop`; returns the tag that ended the block.
fn block(toks: &[Tok], i: &mut usize, stop: &[&str]) -> Result<(Vec<Node>, Option<(String, String)>), String> {
    let mut nodes = Vec::new();
    while *i < toks.len() {
        let tok = &toks[*i];
        *i += 1;
        match tok {
            Tok::Text(t) => nodes.push(Node::Text(t.clone())),
            Tok::Out(e) => {
                if e.is_empty() {
                    return Err("Variable name cannot be empty".into());
                }
                nodes.push(Node::Out(e.clone()))
            }
            Tok::Tag(tag) => {
                let (name, arg) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
                let arg = arg.trim();
                if stop.contains(&name) {
                    return Ok((nodes, Some((name.into(), arg.into()))));
                }
                match name {
                    "if" => {
                        let mut branches = Vec::new();
                        let mut cond = arg.to_string();
                        loop {
                            let (body, end) = block(toks, i, &["elsif", "else", "endif"])?;
                            branches.push((cond, body));
                            let (end, a) = end.ok_or("Unclosed block: missing 'endif'")?;
                            match end.as_str() {
                                "endif" => break,
                                "else" => cond = "true".into(),
                                _ => cond = a,
                            }
                        }
                        nodes.push(Node::If(branches));
                    }
                    "for" => {
                        let (var, list) = arg.split_once(" in ").ok_or(format!("Invalid for tag: {tag}"))?;
                        let (body, end) = block(toks, i, &["endfor"])?;
                        end.ok_or("Unclosed block: missing 'endfor'")?;
                        nodes.push(Node::For { var: var.trim().into(), list: list.trim().into(), body });
                    }
                    _ => return Err(format!("Unknown tag: {name}")),
                }
            }
        }
    }
    Ok((nodes, None))
}

/// Splits on `sep`, ignoring occurrences inside quotes.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let (mut parts, mut start, mut quote) = (Vec::new(), 0, None);
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == sep => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            None => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Caller context plus loop-local variables (a stack: no clone per iteration).
struct Env<'a> {
    globals: &'a Context,
    locals: Vec<(String, Value)>,
}

impl Env<'_> {
    fn get(&self, k: &str) -> Option<&Value> {
        self.locals.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v).or_else(|| self.globals.get(k))
    }
}

fn value<'a>(expr: &str, env: &'a Env) -> Cow<'a, Value> {
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

fn apply_filter(name: &str, v: &Value, args: &[Value]) -> Result<Value, String> {
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

fn output<'a>(expr: &str, env: &'a Env) -> Result<Cow<'a, Value>, String> {
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

fn compare(a: &Value, op: &str, b: &Value) -> bool {
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
fn cond_tokens(s: &str) -> Result<Vec<String>, String> {
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

/// Grammatica: or := and ("or" and)* ; and := cmp ("and" cmp)* ; cmp := atom [op atom] ; atom := "(" or ")" | value.
struct Cond<'a> {
    toks: Vec<String>,
    i: usize,
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
            let v = self.or()?;
            if self.peek() != Some(")") {
                return Err("Invalid condition: missing ')'".into());
            }
            self.i += 1;
            return Ok(v);
        }
        Ok(value(&t, self.env))
    }
}

fn condition(expr: &str, env: &Env) -> Result<bool, String> {
    let mut c = Cond { toks: cond_tokens(expr)?, i: 0, env };
    let v = c.or()?;
    if c.i < c.toks.len() {
        return Err(format!("Invalid condition: '{expr}'"));
    }
    Ok(v.truthy())
}

fn run(nodes: &[Node], env: &mut Env, out: &mut String) -> Result<(), String> {
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::Out(e) => out.push_str(&output(e, env)?.show()),
            Node::If(branches) => {
                for (c, body) in branches {
                    if condition(c, env)? {
                        run(body, env, out)?;
                        break;
                    }
                }
            }
            Node::For { var, list, body } => {
                // ponytail: the list is cloned once per loop (so env isn't kept borrowed); use Rc if it gets costly.
                let items = match value(list, env).into_owned() {
                    Value::List(l) => l,
                    Value::Nil => vec![],
                    v => vec![v],
                };
                let (base, len) = (env.locals.len(), items.len());
                for (k, item) in items.into_iter().enumerate() {
                    let forloop = HashMap::from([
                        ("index".into(), Value::Num((k + 1) as f64)),
                        ("index0".into(), Value::Num(k as f64)),
                        ("first".into(), Value::Bool(k == 0)),
                        ("last".into(), Value::Bool(k + 1 == len)),
                        ("length".into(), Value::Num(len as f64)),
                    ]);
                    env.locals.push((var.clone(), item));
                    env.locals.push(("forloop".into(), Value::Map(forloop)));
                    let r = run(body, env, out);
                    env.locals.truncate(base);
                    r?;
                }
            }
        }
    }
    Ok(())
}

pub fn render(template: &str, ctx: &Context) -> Result<String, String> {
    let toks = tokenize(template)?;
    let (nodes, _) = block(&toks, &mut 0, &[])?;
    let mut out = String::new();
    run(&nodes, &mut Env { globals: ctx, locals: Vec::new() }, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pairs: Vec<(&str, Value)>) -> Context {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    #[test]
    fn variables_paths_and_filters() {
        let user = Value::Map(HashMap::from([("name".into(), "Andrea".into())]));
        let c = ctx(vec![("user", user), ("xs", vec![1.into(), 2.into()].into())]);
        assert_eq!(render("Hello {{ user.name | uppercase }}!", &c).unwrap(), "Hello ANDREA!");
        assert_eq!(render("{{ x | default: 'n/a' }}", &c).unwrap(), "n/a");
        assert_eq!(render("{{ xs | join: ', ' }}", &c).unwrap(), "1, 2");
        assert_eq!(render("{{ xs.size }}", &c).unwrap(), "2");
        assert_eq!(render("{{ '<a>' | escape }}", &c).unwrap(), "&#60;a&#62;");
    }

    #[test]
    fn if_elsif_else() {
        let t = "{% if n > 5 %}big{% elsif n == 5 %}five{% else %}small{% endif %}";
        for (n, want) in [(9, "big"), (5, "five"), (1, "small")] {
            assert_eq!(render(t, &ctx(vec![("n", n.into())])).unwrap(), want);
        }
        let c = ctx(vec![("a", true.into())]);
        assert_eq!(render("{% if a and b %}y{% endif %}", &c).unwrap(), "");
        assert_eq!(render("{% if a or b %}y{% endif %}", &c).unwrap(), "y");
        // `and` binds tighter than `or`; parentheses override; connectives inside strings are plain text.
        assert_eq!(render("{% if a or b and c %}y{% endif %}", &c).unwrap(), "y");
        assert_eq!(render("{% if (a or b) and c %}y{% endif %}", &c).unwrap(), "");
        let c = ctx(vec![("s", "x and y".into())]);
        assert_eq!(render("{% if s == 'x and y' %}y{% endif %}", &c).unwrap(), "y");
        assert!(render("{% if (a %}y{% endif %}", &c).is_err());
    }

    #[test]
    fn for_loops() {
        let c = ctx(vec![("xs", vec!["a".into(), "b".into()].into())]);
        assert_eq!(render("{% for x in xs %}{{ forloop.index }}={{ x }};{% endfor %}", &c).unwrap(), "1=a;2=b;");
        let rows = vec![vec![1.into(), 2.into()].into(), vec![3.into()].into()];
        let c = ctx(vec![("rows", rows.into())]);
        let t = "{% for r in rows %}{% for c in r %}{{ c }}{% endfor %}|{% endfor %}";
        assert_eq!(render(t, &c).unwrap(), "12|3|");
        // Loop variables don't leak out of the loop.
        assert_eq!(render("{% for x in xs %}{% endfor %}[{{ x }}]", &c).unwrap(), "[]");
    }

    #[test]
    fn errors() {
        let c = Context::new();
        assert!(render("Hello {{ name", &c).is_err());
        assert!(render("{% if a %}x", &c).is_err());
        assert!(render("{% boh %}", &c).is_err());
        assert!(render("{{ a | nope }}", &c).is_err());
        assert_eq!(render("Hello {{ name }}!", &c).unwrap(), "Hello !");
    }
}
