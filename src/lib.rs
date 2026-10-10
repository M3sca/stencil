//! Minimal template engine:
//! `{{ var | filter: arg }}`, `{% if/elsif/else %}`, `{% for x in xs %}`.

#[cfg(feature = "node")]
mod node;

use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;

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

/// Named templates available to `{% include "name" %}`. Names are plain map keys:
/// they are never interpreted as paths and nothing here touches the filesystem.
pub type Partials = HashMap<String, String>;

const MAX_INCLUDE_DEPTH: usize = 32;
// Bounds output growth from templates that include each other many times over.
const MAX_INCLUDES: usize = 100_000;
/// Total loop iterations in one render (all loops, all includes).
const MAX_ITERATIONS: usize = 1_000_000;
/// Nodes executed in one render: bounds CPU however the work is spread over loops and includes.
const MAX_STEPS: usize = 2_000_000;
/// Nesting of blocks (`if`/`for`/`block`) and of parentheses in conditions; bounds recursion depth.
const MAX_NESTING: usize = 50;
/// Total nested `run` calls across blocks, includes and loops: bounds stack use for any template.
const MAX_RUN_DEPTH: usize = 100;
/// Maximum size of a rendered output, in bytes.
const MAX_OUTPUT: usize = 10 * 1024 * 1024;

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

#[derive(Clone)]
enum Node {
    Text(String),
    Out(String),
    If(Vec<(String, Vec<Node>)>),
    For { var: String, list: String, body: Vec<Node> },
    Include { name: String, args: Vec<(String, String)> },
    Extends(String),
    Block { name: String, body: Vec<Node> },
}

/// Position of the next `{{` or `{%`, in one pass (searching for each separately is quadratic).
fn next_open(s: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(p) = s[from..].find('{').map(|p| p + from) {
        if matches!(s.as_bytes().get(p + 1), Some(b'{' | b'%')) {
            return Some(p);
        }
        from = p + 1;
    }
    None
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let mut rest = src;
    while let Some(pos) = next_open(rest) {
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
fn block(toks: &[Tok], i: &mut usize, stop: &[&str], depth: usize) -> Result<(Vec<Node>, Option<(String, String)>), String> {
    if depth > MAX_NESTING {
        return Err(format!("Template nested too deeply (max {MAX_NESTING} levels)"));
    }
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
                            let (body, end) = block(toks, i, &["elsif", "else", "endif"], depth + 1)?;
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
                        let (body, end) = block(toks, i, &["endfor"], depth + 1)?;
                        end.ok_or("Unclosed block: missing 'endfor'")?;
                        nodes.push(Node::For { var: var.trim().into(), list: list.trim().into(), body });
                    }
                    "include" => nodes.push(parse_include("include", arg)?),
                    "extends" => match parse_include("extends", arg)? {
                        Node::Include { name, args } if args.is_empty() => nodes.push(Node::Extends(name)),
                        _ => return Err("extends takes only a quoted template name".into()),
                    },
                    "block" => {
                        if arg.is_empty() || !arg.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                            return Err(format!("Invalid block name: '{arg}'"));
                        }
                        let (body, end) = block(toks, i, &["endblock"], depth + 1)?;
                        end.ok_or("Unclosed block: missing 'endblock'")?;
                        nodes.push(Node::Block { name: arg.into(), body });
                    }
                    _ => return Err(format!("Unknown tag: {name}")),
                }
            }
        }
    }
    Ok((nodes, None))
}

/// `"name"` or `"name" with a = expr, b = expr`. The name must be a quoted literal.
fn parse_include(tag: &str, arg: &str) -> Result<Node, String> {
    let bad = || format!("{tag} needs a quoted template name");
    let q = arg.chars().next().filter(|c| matches!(c, '"' | '\'')).ok_or_else(bad)?;
    let rest = &arg[1..];
    let end = rest.find(q).filter(|&e| e > 0).ok_or_else(bad)?;
    let tail = rest[end + 1..].trim();
    let mut args = Vec::new();
    if !tail.is_empty() {
        let list = tail.strip_prefix("with").filter(|l| l.starts_with(char::is_whitespace));
        for part in split_top(list.ok_or(format!("Invalid {tag}: {arg}"))?, ',') {
            let (k, v) = part.split_once('=').ok_or(format!("Invalid include argument: '{}'", part.trim()))?;
            let k = k.trim();
            if k.is_empty() || !k.chars().all(|c| c.is_alphanumeric() || c == '_') || v.trim().is_empty() {
                return Err(format!("Invalid include argument: '{}'", part.trim()));
            }
            args.push((k.to_string(), v.trim().to_string()));
        }
    }
    Ok(Node::Include { name: rest[..end].to_string(), args })
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

/// Collects every `block` in the tree by name; a name defined twice is an error.
fn collect_blocks(nodes: &[Node], map: &mut HashMap<String, Vec<Node>>) -> Result<(), String> {
    for n in nodes {
        match n {
            Node::Block { name, body } => {
                if map.insert(name.clone(), body.clone()).is_some() {
                    return Err(format!("Duplicate block '{name}'"));
                }
                collect_blocks(body, map)?;
            }
            Node::If(branches) => branches.iter().try_for_each(|(_, b)| collect_blocks(b, map))?,
            Node::For { body, .. } => collect_blocks(body, map)?,
            _ => {}
        }
    }
    Ok(())
}

/// Replaces the body of every block that `ov` overrides. Blocks nested inside an overriding body are left as written.
fn substitute(nodes: Vec<Node>, ov: &HashMap<String, Vec<Node>>) -> Vec<Node> {
    nodes
        .into_iter()
        .map(|n| match n {
            Node::Block { name, body } => {
                let body = ov.get(&name).cloned().unwrap_or_else(|| substitute(body, ov));
                Node::Block { name, body }
            }
            Node::If(branches) => Node::If(branches.into_iter().map(|(c, b)| (c, substitute(b, ov))).collect()),
            Node::For { var, list, body } => Node::For { var, list, body: substitute(body, ov) },
            n => n,
        })
        .collect()
}

/// Parses a template and resolves `{% extends %}`: the parent is loaded (recursively) and this template's blocks replace the parent's.
// ponytail: block bodies are cloned while resolving; fine for page-sized templates.
fn load(src: &str, partials: &Partials, depth: usize) -> Result<Vec<Node>, String> {
    let toks = tokenize(src)?;
    let (nodes, _) = block(&toks, &mut 0, &[], 0)?;
    let mut own = HashMap::new();
    collect_blocks(&nodes, &mut own)?;
    let Some(i) = nodes.iter().position(|n| matches!(n, Node::Extends(_))) else {
        return Ok(nodes);
    };
    let first = nodes[..i].iter().all(|n| matches!(n, Node::Text(t) if t.trim().is_empty()));
    if !first || nodes[i + 1..].iter().any(|n| matches!(n, Node::Extends(_))) {
        return Err("extends must be the first tag of a template, and used once".into());
    }
    let Node::Extends(parent) = &nodes[i] else { unreachable!() };
    if depth >= MAX_INCLUDE_DEPTH {
        return Err(format!("Extends nesting too deep (max {MAX_INCLUDE_DEPTH}) at '{parent}'"));
    }
    let psrc = partials.get(parent).ok_or(format!("Unknown template: {parent}"))?;
    let base = load(psrc, partials, depth + 1).map_err(|e| format!("{e} (in '{parent}')"))?;
    Ok(substitute(base, &own))
}

/// Caller context plus loop-local variables (a stack: no clone per iteration).
struct Env<'a> {
    globals: &'a Context,
    locals: Vec<(String, Value)>,
    partials: &'a Partials,
    depth: usize,
    includes: usize,
    iterations: usize,
    steps: usize,
    level: usize,
    /// Parsed partials by name: each is parsed once per render, however often it is included.
    cache: HashMap<String, Rc<Vec<Node>>>,
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

fn condition(expr: &str, env: &Env) -> Result<bool, String> {
    let mut c = Cond { toks: cond_tokens(expr)?, i: 0, depth: 0, env };
    let v = c.or()?;
    if c.i < c.toks.len() {
        return Err(format!("Invalid condition: '{expr}'"));
    }
    Ok(v.truthy())
}

fn emit(out: &mut String, s: &str) -> Result<(), String> {
    if out.len() + s.len() > MAX_OUTPUT {
        return Err(format!("Output too large (max {MAX_OUTPUT} bytes)"));
    }
    out.push_str(s);
    Ok(())
}

fn run(nodes: &[Node], env: &mut Env, out: &mut String) -> Result<(), String> {
    env.level += 1;
    let r = if env.level > MAX_RUN_DEPTH {
        Err(format!("Template nested too deeply (max {MAX_RUN_DEPTH} levels, includes included)"))
    } else {
        run_nodes(nodes, env, out)
    };
    env.level -= 1;
    r
}

fn run_nodes(nodes: &[Node], env: &mut Env, out: &mut String) -> Result<(), String> {
    for n in nodes {
        env.steps += 1;
        if env.steps > MAX_STEPS {
            return Err(format!("Template too complex (max {MAX_STEPS} steps)"));
        }
        match n {
            Node::Text(t) => emit(out, t)?,
            Node::Out(e) => emit(out, &output(e, env)?.show())?,
            Node::If(branches) => {
                for (c, body) in branches {
                    if condition(c, env)? {
                        run(body, env, out)?;
                        break;
                    }
                }
            }
            Node::Include { name, args } => {
                let src = env.partials.get(name).ok_or(format!("Unknown template: {name}"))?;
                if env.depth >= MAX_INCLUDE_DEPTH {
                    return Err(format!("Include nesting too deep (max {MAX_INCLUDE_DEPTH}) at '{name}'"));
                }
                env.includes += 1;
                if env.includes > MAX_INCLUDES {
                    return Err(format!("Too many includes (max {MAX_INCLUDES})"));
                }
                let nodes = match env.cache.get(name) {
                    Some(n) => n.clone(),
                    None => {
                        let n = Rc::new(load(src, env.partials, env.depth + 1).map_err(|e| format!("{e} (in '{name}')"))?);
                        env.cache.insert(name.clone(), n.clone());
                        n
                    }
                };
                // `with` values are evaluated in the caller's scope, then visible only inside the include.
                let extra: Vec<_> = args.iter().map(|(k, e)| (k.clone(), value(e, env).into_owned())).collect();
                let base = env.locals.len();
                env.locals.extend(extra);
                env.depth += 1;
                let r = run(&nodes, env, out).map_err(|e| format!("{e} (in '{name}')"));
                env.depth -= 1;
                env.locals.truncate(base);
                r?;
            }
            Node::Block { body, .. } => run(body, env, out)?,
            Node::Extends(_) => return Err("extends must be the first tag of a template".into()),
            Node::For { var, list, body } => {
                // ponytail: the list is cloned once per loop (so env isn't kept borrowed); use Rc if it gets costly.
                let items = match value(list, env).into_owned() {
                    Value::List(l) => l,
                    Value::Nil => vec![],
                    v => vec![v],
                };
                let (base, len) = (env.locals.len(), items.len());
                // The enclosing loop's metadata (if any) becomes `forloop.parentloop`.
                let parent = env.locals.iter().rev().find(|(n, _)| n == "forloop").map(|(_, v)| v.clone());
                for (k, item) in items.into_iter().enumerate() {
                    env.iterations += 1;
                    if env.iterations > MAX_ITERATIONS {
                        env.locals.truncate(base);
                        return Err(format!("Too many loop iterations (max {MAX_ITERATIONS})"));
                    }
                    let mut forloop = HashMap::from([
                        ("index".into(), Value::Num((k + 1) as f64)),
                        ("index0".into(), Value::Num(k as f64)),
                        ("first".into(), Value::Bool(k == 0)),
                        ("last".into(), Value::Bool(k + 1 == len)),
                        ("length".into(), Value::Num(len as f64)),
                    ]);
                    if let Some(p) = &parent {
                        forloop.insert("parentloop".into(), p.clone());
                    }
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
    render_with(template, ctx, &Partials::new())
}

/// Like [`render`], with named templates that the template can `{% include "name" %}`.
pub fn render_with(template: &str, ctx: &Context, partials: &Partials) -> Result<String, String> {
    let nodes = load(template, partials, 0)?;
    let mut out = String::new();
    let mut env = Env { globals: ctx, locals: Vec::new(), partials, depth: 0, includes: 0, iterations: 0, steps: 0, level: 0, cache: HashMap::new() };
    run(&nodes, &mut env, &mut out)?;
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
        let meta = "{% for x in xs %}{% if forloop.first %}[{% endif %}{{ forloop.index0 }}/{{ forloop.length }}{% if forloop.last %}]{% endif %}{% endfor %}";
        assert_eq!(render(meta, &c).unwrap(), "[0/21/2]");
        // `forloop` doesn't outlive its loop, and in nested loops it is the innermost one's.
        assert_eq!(render("{% for x in xs %}{% for y in xs %}{{ forloop.index }}{% endfor %}{{ forloop.index }}{% endfor %}x{{ forloop.index }}", &c).unwrap(), "121122x");
        // `forloop.parentloop` reaches the enclosing loops, chained for deeper nesting; the outermost has none.
        let t = "{% for a in xs %}{% for b in xs %}{% for c in xs %}{{ forloop.parentloop.parentloop.index }}{{ forloop.parentloop.index }}{{ forloop.index }}{% if forloop.last %};{% endif %}{% endfor %}{% endfor %}{% endfor %}";
        assert_eq!(render(t, &c).unwrap(), "111112;121122;211212;221222;");
        assert_eq!(render("{% for a in xs %}[{{ forloop.parentloop.index }}]{% endfor %}", &c).unwrap(), "[][]");
        let rows = vec![vec![1.into(), 2.into()].into(), vec![3.into()].into()];
        let c = ctx(vec![("rows", rows.into())]);
        let t = "{% for r in rows %}{% for c in r %}{{ c }}{% endfor %}|{% endfor %}";
        assert_eq!(render(t, &c).unwrap(), "12|3|");
        // Loop variables don't leak out of the loop.
        assert_eq!(render("{% for x in xs %}{% endfor %}[{{ x }}]", &c).unwrap(), "[]");
    }

    #[test]
    fn includes() {
        let partials = Partials::from([
            ("row".to_string(), "<{{ x }}>".to_string()),
            ("list".to_string(), "{% for x in xs %}{% include 'row' %}{% endfor %}".to_string()),
            ("loop".to_string(), "{% include \"loop\" %}".to_string()),
            ("bad".to_string(), "{% if %}".to_string()),
            ("../../etc/passwd".to_string(), "should never be reached by path".to_string()),
        ]);
        let c = ctx(vec![("xs", vec![1.into(), 2.into()].into()), ("x", "g".into())]);
        let r = |t: &str| render_with(t, &c, &partials);
        assert_eq!(r("{% include \"row\" %}").unwrap(), "<g>"); // sees caller variables
        assert_eq!(r("{% include \"list\" %}").unwrap(), "<1><2>"); // nested, loop variable visible
        assert!(r("{% include \"nope\" %}").unwrap_err().contains("Unknown template"));
        assert!(r("{% include \"loop\" %}").unwrap_err().contains("too deep"));
        assert!(r("{% include row %}").unwrap_err().contains("quoted"));
        assert!(r("{% include \"\" %}").is_err());
        assert!(r("{% include \"bad\" %}").unwrap_err().contains("in 'bad'"));
        // Names are only map keys: a traversal-looking name matches only an identical key.
        assert!(r("{% include \"../Cargo.toml\" %}").unwrap_err().contains("Unknown template"));
        // Templates that include each other many times over are cut off instead of exploding.
        let mut chain: Partials = (0..25).map(|i| (format!("p{i}"), format!("{{% include 'p{}' %}}{{% include 'p{}' %}}", i + 1, i + 1))).collect();
        chain.insert("p25".into(), "x".into());
        assert!(render_with("{% include 'p0' %}", &c, &chain).unwrap_err().contains("Too many includes"));
        // `with` passes extra variables, evaluated in the caller's scope, visible only in the include.
        let p2 = Partials::from([("card".to_string(), "[{{ title }}|{{ n }}]".to_string())]);
        let w = |t: &str| render_with(t, &c, &p2);
        assert_eq!(w("{% include 'card' with title = 'A, b', n = 3 %}").unwrap(), "[A, b|3]");
        assert_eq!(w("{% for x in xs %}{% include 'card' with title = x, n = xs.size %}{% endfor %}").unwrap(), "[1|2][2|2]");
        assert_eq!(w("{% include 'card' with title = 'T' %}{{ title }}.").unwrap(), "[T|].");
        for bad in ["with", "with a", "with = 1", "with a =", "with 1x y = 2", "x = 1"] {
            assert!(w(&format!("{{% include 'card' {bad} %}}")).is_err(), "{bad}");
        }
        // Includes need registered partials: plain render() has none.
        assert!(render("{% include \"row\" %}", &c).is_err());
    }

    #[test]
    fn iterations_are_capped() {
        let big: Vec<Value> = (0..1000).map(|i| (i as i64).into()).collect();
        let small: Vec<Value> = (0..500).map(|i| (i as i64).into()).collect();
        let c = ctx(vec![("big", big.into()), ("small", small.into())]);
        let t = "{% for a in big %}{% for b in big %}{% for c in big %}{% endfor %}{% endfor %}{% endfor %}";
        assert!(render(t, &c).unwrap_err().contains("Too many loop iterations"));
        assert_eq!(render("{% for a in small %}{% for b in small %}{% endfor %}{% endfor %}", &c).unwrap(), "");
    }

    #[test]
    fn extends_and_blocks() {
        let partials = Partials::from([
            ("layout".to_string(), "<h1>{% block title %}Site{% endblock %}</h1>|{% block body %}{% endblock %}".to_string()),
            ("section".to_string(), "{% extends 'layout' %}{% block title %}{{ title }}!{% endblock %}{% block body %}S:{% block inner %}in{% endblock %}{% endblock %}".to_string()),
            ("rows".to_string(), "{% for x in xs %}{% block row %}-{% endblock %}{% endfor %}".to_string()),
            ("self".to_string(), "{% extends 'self' %}".to_string()),
            ("dup".to_string(), "{% block a %}{% endblock %}{% block a %}{% endblock %}".to_string()),
            ("late".to_string(), "x{% extends 'layout' %}".to_string()),
            ("with".to_string(), "{% extends 'layout' with a = 1 %}".to_string()),
            ("page".to_string(), "{% extends 'layout' %}\n{% block body %}Hi {{ name }}{% endblock %}".to_string()),
        ]);
        let c = ctx(vec![("name", "Ann".into()), ("title", "T".into()), ("xs", vec![1.into(), 2.into()].into())]);
        let r = |t: &str| render_with(t, &c, &partials);
        // A child overrides some blocks and inherits the rest; text outside blocks is ignored.
        assert_eq!(r("{% include 'page' %}").unwrap(), "<h1>Site</h1>|Hi Ann");
        assert_eq!(r("{% extends 'layout' %}ignored{% block body %}B{% endblock %}").unwrap(), "<h1>Site</h1>|B");
        // Several levels, and overriding a block nested in a parent's block.
        assert_eq!(r("{% extends 'section' %}{% block inner %}IN2{% endblock %}").unwrap(), "<h1>T!</h1>|S:IN2");
        assert_eq!(r("{% extends 'section' %}").unwrap(), "<h1>T!</h1>|S:in");
        // Blocks inside loops see the loop variable.
        assert_eq!(r("{% extends 'rows' %}{% block row %}{{ x }}{% endblock %}").unwrap(), "12");
        // Without extends a block just renders its body; unknown blocks in a child are ignored.
        assert_eq!(r("{% block a %}d{% endblock %}").unwrap(), "d");
        assert_eq!(r("{% extends 'layout' %}{% block nope %}x{% endblock %}").unwrap(), "<h1>Site</h1>|");
        // Errors.
        assert!(r("{% extends 'nope' %}").unwrap_err().contains("Unknown template"));
        assert!(r("{% extends 'self' %}").unwrap_err().contains("too deep"));
        assert!(r("{% include 'dup' %}").unwrap_err().contains("Duplicate block"));
        assert!(r("{% include 'late' %}").unwrap_err().contains("first tag"));
        assert!(r("{% include 'with' %}").unwrap_err().contains("only a quoted"));
        assert!(r("{% extends layout %}").unwrap_err().contains("quoted"));
        assert!(r("{% extends 'layout' %}{% extends 'layout' %}").is_err());
        assert!(r("{% if true %}{% extends 'layout' %}{% endif %}").is_err());
        assert!(r("{% block %}{% endblock %}").is_err());
        assert!(r("{% block a b %}{% endblock %}").is_err());
        assert!(r("{% block a %}").is_err());
    }

    #[test]
    fn hostile_templates_are_bounded() {
        let c = Context::new();
        // Many tags in a row: parsing must be linear (it used to be quadratic).
        let flat = "{% for a in xs %}{% endfor %}".repeat(30_000);
        assert_eq!(render(&flat, &c).unwrap(), "");
        // Deep nesting is refused instead of overflowing the stack.
        let deep = |open: &str, close: &str| format!("{}x{}", open.repeat(100_000), close.repeat(100_000));
        for (o, cl) in [("{% if true %}", "{% endif %}"), ("{% for a in xs %}", "{% endfor %}"), ("{% block a %}", "{% endblock %}")] {
            assert!(render(&deep(o, cl), &c).unwrap_err().contains("nested too deeply"), "{o}");
        }
        let parens = format!("{{% if {}true{} %}}x{{% endif %}}", "(".repeat(100_000), ")".repeat(100_000));
        assert!(render(&parens, &c).unwrap_err().contains("nested too deeply"));
        // Deep nesting spread over a long include chain is cut by the total depth budget, not by a stack overflow.
        let nest = |inner: &str| format!("{}{}{}", "{% if (((true))) %}".repeat(MAX_NESTING - 1), inner, "{% endif %}".repeat(MAX_NESTING - 1));
        let mut partials: Partials = (0..MAX_INCLUDE_DEPTH - 1).map(|i| (format!("p{i}"), nest(&format!("{{% include 'p{}' %}}", i + 1)))).collect();
        partials.insert(format!("p{}", MAX_INCLUDE_DEPTH - 1), "x".into());
        assert!(render_with("{% include 'p0' %}", &c, &partials).unwrap_err().contains("nested too deeply"));
        // Ordinary nesting through ordinary includes works.
        let mut ok: Partials = (0..8).map(|i| (format!("p{i}"), format!("{{% if true %}}{{% if true %}}{{% include 'p{}' %}}{{% endif %}}{{% endif %}}", i + 1))).collect();
        ok.insert("p8".into(), "x".into());
        assert_eq!(render_with("{% include 'p0' %}", &c, &ok).unwrap(), "x");
    }

    #[test]
    fn work_is_bounded_and_partials_are_parsed_once() {
        let big: Vec<Value> = (0..1000).map(|i| (i as i64).into()).collect();
        let c = ctx(vec![("big", big.into())]);
        // Lots of cheap nodes inside one big partial included in a loop: capped by total steps.
        let partials = Partials::from([("p".to_string(), "{% if false %}{% endif %}".repeat(3000))]);
        let err = render_with("{% for a in big %}{% include 'p' %}{% endfor %}", &c, &partials).unwrap_err();
        assert!(err.contains("too complex"), "{err}");
    }

    #[test]
    fn output_is_capped() {
        let big: Vec<Value> = (0..1000).map(|i| (i as i64).into()).collect();
        let c = ctx(vec![("big", big.into()), ("s", "x".repeat(100).as_str().into())]);
        let t = "{% for a in big %}{% for b in big %}{{ s }}{% endfor %}{% endfor %}";
        assert!(render(t, &c).unwrap_err().contains("Output too large"));
        assert!(render("{% for a in big %}{{ s }}{% endfor %}", &c).unwrap().len() == 100_000);
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
