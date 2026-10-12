//! Template syntax: turns source text into a tree of nodes.

use crate::limits::MAX_NESTING;

pub(crate) enum Tok {
    Text(String),
    Out(String, usize),
    Tag(String, usize),
}

// Nodes carry the template line they came from, for error messages.
#[derive(Clone)]
pub(crate) enum Node {
    Text(String),
    Out { expr: String, line: usize },
    If(Vec<(String, Vec<Node>, usize)>),
    For { var: String, list: String, body: Vec<Node>, line: usize },
    Include { name: String, args: Vec<(String, String)>, line: usize },
    Extends(String, usize),
    Block { name: String, body: Vec<Node>, line: usize },
    Assign { name: String, expr: String, line: usize },
    Capture { name: String, body: Vec<Node>, line: usize },
    Case { subject: String, whens: Vec<(Vec<String>, Vec<Node>)>, otherwise: Option<Vec<Node>> },
    Break(usize),
    Continue(usize),
}

/// Appends the template line to an error message.
pub(crate) fn at(msg: impl std::fmt::Display, line: usize) -> String {
    format!("{msg} (line {line})")
}

pub(crate) fn newlines(s: &str) -> usize {
    s.bytes().filter(|&b| b == b'\n').count()
}

/// Position of the next `{{`, `{%` or `{#`, in one pass (searching for each separately is quadratic).
pub(crate) fn next_open(s: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(p) = s[from..].find('{').map(|p| p + from) {
        if matches!(s.as_bytes().get(p + 1), Some(b'{' | b'%' | b'#')) {
            return Some(p);
        }
        from = p + 1;
    }
    None
}

/// Splits the `-` whitespace-control markers off a tag body: `{%- x -%}` gives (true, "x", true).
fn dashes(body: &str) -> (bool, &str, bool) {
    let left = body.starts_with('-');
    let body = if left { &body[1..] } else { body };
    let right = body.ends_with('-');
    (left, body[..body.len() - right as usize].trim(), right)
}

/// Finds the tag `{% name %}` in `s`: returns the offset of the tag, its length and its dash markers.
/// Tags are split the same way `tokenize` does, so this is linear too.
fn find_end_tag(s: &str, name: &str) -> Option<(usize, usize, bool, bool)> {
    let mut from = 0;
    while let Some(p) = s[from..].find("{%").map(|p| p + from) {
        let e = s[p + 2..].find("%}")?;
        let (left, body, right) = dashes(&s[p + 2..p + 2 + e]);
        if body == name {
            return Some((p, e + 4, left, right));
        }
        from = p + 2 + e + 2;
    }
    None
}

pub(crate) fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let (mut rest, mut line) = (src, 1);
    let mut trim_next = false; // the previous tag ended with `-%}`: strip the whitespace after it
    while let Some(pos) = next_open(rest) {
        let close = match &rest[pos..pos + 2] {
            "{{" => "}}",
            "{%" => "%}",
            _ => "#}",
        };
        let inner = &rest[pos + 2..];
        let end = inner.find(close).ok_or_else(|| at(format!("Unclosed delimiter: missing '{close}'"), line + newlines(&rest[..pos])))?;
        let (left, body, right) = dashes(&inner[..end]);
        let mut text = &rest[..pos];
        if trim_next {
            text = text.trim_start();
        }
        if left {
            text = text.trim_end();
        }
        if !text.is_empty() {
            toks.push(Tok::Text(text.into()));
        }
        line += newlines(&rest[..pos]);
        let mut consumed = pos + 2 + end + 2;
        trim_next = right;
        match (close, body) {
            ("}}", _) => toks.push(Tok::Out(body.into(), line)),
            ("#}", _) => {} // inline comment
            // `raw` keeps its content as text and `comment` drops it; neither is parsed.
            ("%}", "raw" | "comment") => {
                let end_tag = if body == "raw" { "endraw" } else { "endcomment" };
                let after = &rest[consumed..];
                let (len, tag, eleft, eright) = find_end_tag(after, end_tag).ok_or_else(|| at(format!("Unclosed block: missing '{end_tag}'"), line))?;
                let mut content = &after[..len];
                if right {
                    content = content.trim_start();
                }
                if eleft {
                    content = content.trim_end();
                }
                if body == "raw" && !content.is_empty() {
                    toks.push(Tok::Text(content.into()));
                }
                consumed += len + tag;
                trim_next = eright;
            }
            _ => toks.push(Tok::Tag(body.into(), line)),
        }
        line += newlines(&rest[pos..consumed]);
        rest = &rest[consumed..];
    }
    let text = if trim_next { rest.trim_start() } else { rest };
    if !text.is_empty() {
        toks.push(Tok::Text(text.into()));
    }
    Ok(toks)
}

/// The tag that ended a block: its name, its argument and its line.
pub(crate) type Stop = Option<(String, String, usize)>;

/// Reads nodes until one of the tags in `stop`; returns the tag that ended the block.
pub(crate) fn block(toks: &[Tok], i: &mut usize, stop: &[&str], depth: usize) -> Result<(Vec<Node>, Stop), String> {
    if depth > MAX_NESTING {
        return Err(format!("Template nested too deeply (max {MAX_NESTING} levels)"));
    }
    let mut nodes = Vec::new();
    while *i < toks.len() {
        let tok = &toks[*i];
        *i += 1;
        match tok {
            Tok::Text(t) => nodes.push(Node::Text(t.clone())),
            Tok::Out(e, line) => {
                if e.is_empty() {
                    return Err(at("Variable name cannot be empty", *line));
                }
                nodes.push(Node::Out { expr: e.clone(), line: *line })
            }
            Tok::Tag(tag, line) => {
                let line = *line;
                let (name, arg) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
                let arg = arg.trim();
                if stop.contains(&name) {
                    return Ok((nodes, Some((name.into(), arg.into(), line))));
                }
                match name {
                    "if" => {
                        let mut branches = Vec::new();
                        let (mut cond, mut cond_line) = (arg.to_string(), line);
                        loop {
                            let (body, end) = block(toks, i, &["elsif", "else", "endif"], depth + 1)?;
                            branches.push((cond, body, cond_line));
                            let (end, a, l) = end.ok_or_else(|| at("Unclosed block: missing 'endif'", line))?;
                            cond_line = l;
                            match end.as_str() {
                                "endif" => break,
                                "else" => cond = "true".into(),
                                _ => cond = a,
                            }
                        }
                        nodes.push(Node::If(branches));
                    }
                    "for" => {
                        let (var, list) = arg.split_once(" in ").ok_or_else(|| at(format!("Invalid for tag: {tag}"), line))?;
                        let (body, end) = block(toks, i, &["endfor"], depth + 1)?;
                        end.ok_or_else(|| at("Unclosed block: missing 'endfor'", line))?;
                        nodes.push(Node::For { var: var.trim().into(), list: list.trim().into(), body, line });
                    }
                    "unless" => {
                        // `unless c` is `if not (c)`; it takes an optional `else`.
                        let (body, end) = block(toks, i, &["else", "endunless"], depth + 1)?;
                        let (end, _, l) = end.ok_or_else(|| at("Unclosed block: missing 'endunless'", line))?;
                        let mut branches = vec![(format!("not ({arg})"), body, line)];
                        if end == "else" {
                            let (body, end) = block(toks, i, &["endunless"], depth + 1)?;
                            end.ok_or_else(|| at("Unclosed block: missing 'endunless'", line))?;
                            branches.push(("true".into(), body, l));
                        }
                        nodes.push(Node::If(branches));
                    }
                    "case" => {
                        if arg.is_empty() {
                            return Err(at("case needs a value", line));
                        }
                        // Anything between `case` and the first `when` is ignored.
                        let (_, mut stop) = block(toks, i, &["when", "else", "endcase"], depth + 1)?;
                        let (mut whens, mut otherwise) = (Vec::new(), None);
                        loop {
                            let (end, a, l) = stop.ok_or_else(|| at("Unclosed block: missing 'endcase'", line))?;
                            match end.as_str() {
                                "when" => {
                                    if a.is_empty() {
                                        return Err(at("when needs at least one value", l));
                                    }
                                    let values = split_top(&a, ',').into_iter().map(|v| v.trim().to_string()).collect();
                                    let (body, next) = block(toks, i, &["when", "else", "endcase"], depth + 1)?;
                                    whens.push((values, body));
                                    stop = next;
                                }
                                "else" => {
                                    let (body, next) = block(toks, i, &["endcase"], depth + 1)?;
                                    otherwise = Some(body);
                                    stop = next;
                                }
                                _ => break,
                            }
                        }
                        nodes.push(Node::Case { subject: arg.into(), whens, otherwise });
                    }
                    "assign" => {
                        let (name, expr) = arg.split_once('=').ok_or_else(|| at("assign needs 'name = value'", line))?;
                        let name = name.trim();
                        if !valid_name(name) || expr.trim().is_empty() {
                            return Err(at(format!("Invalid assign: '{arg}'"), line));
                        }
                        nodes.push(Node::Assign { name: name.into(), expr: expr.trim().into(), line });
                    }
                    "capture" => {
                        if !valid_name(arg) {
                            return Err(at(format!("Invalid capture name: '{arg}'"), line));
                        }
                        let (body, end) = block(toks, i, &["endcapture"], depth + 1)?;
                        end.ok_or_else(|| at("Unclosed block: missing 'endcapture'", line))?;
                        nodes.push(Node::Capture { name: arg.into(), body, line });
                    }
                    "break" | "continue" => {
                        if !arg.is_empty() {
                            return Err(at(format!("{name} takes no arguments"), line));
                        }
                        nodes.push(if name == "break" { Node::Break(line) } else { Node::Continue(line) });
                    }
                    "include" => nodes.push(parse_include("include", arg, line).map_err(|e| at(e, line))?),
                    "extends" => match parse_include("extends", arg, line).map_err(|e| at(e, line))? {
                        Node::Include { name, args, .. } if args.is_empty() => nodes.push(Node::Extends(name, line)),
                        _ => return Err(at("extends takes only a quoted template name", line)),
                    },
                    "block" => {
                        if arg.is_empty() || !arg.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                            return Err(at(format!("Invalid block name: '{arg}'"), line));
                        }
                        let (body, end) = block(toks, i, &["endblock"], depth + 1)?;
                        end.ok_or_else(|| at("Unclosed block: missing 'endblock'", line))?;
                        nodes.push(Node::Block { name: arg.into(), body, line });
                    }
                    _ => return Err(at(format!("Unknown tag: {name}"), line)),
                }
            }
        }
    }
    Ok((nodes, None))
}

/// Variable names for `assign`/`capture`: letters, digits and `_`.
fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// `"name"` or `"name" with a = expr, b = expr`. The name must be a quoted literal.
pub(crate) fn parse_include(tag: &str, arg: &str, line: usize) -> Result<Node, String> {
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
    Ok(Node::Include { name: rest[..end].to_string(), args, line })
}

/// Splits on `sep`, ignoring occurrences inside quotes.
pub(crate) fn split_top(s: &str, sep: char) -> Vec<&str> {
    let (mut parts, mut start, mut quote, mut depth) = (Vec::new(), 0, None, 0usize);
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '(' => depth += 1,
            None if c == ')' => depth = depth.saturating_sub(1),
            None if c == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            None => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Index of the `)` that closes the `(` at byte `open`, ignoring parentheses inside quotes.
pub(crate) fn matching_paren(s: &str, open: usize) -> Option<usize> {
    let (mut depth, mut quote) = (0usize, None);
    for (i, c) in s[open..].char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '(' => depth += 1,
            None if c == ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            None => {}
        }
    }
    None
}
