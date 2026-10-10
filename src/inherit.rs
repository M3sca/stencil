//! Template inheritance: `{% extends %}` and `{% block %}`.

use crate::Partials;
use crate::limits::MAX_INCLUDE_DEPTH;
use crate::parse::{Node, at, block, tokenize};
use std::collections::HashMap;

/// Collects every `block` in the tree by name; a name defined twice is an error.
pub(crate) fn collect_blocks(nodes: &[Node], map: &mut HashMap<String, Vec<Node>>) -> Result<(), String> {
    for n in nodes {
        match n {
            Node::Block { name, body, line } => {
                if map.insert(name.clone(), body.clone()).is_some() {
                    return Err(at(format!("Duplicate block '{name}'"), *line));
                }
                collect_blocks(body, map)?;
            }
            Node::If(branches) => branches.iter().try_for_each(|(_, b, _)| collect_blocks(b, map))?,
            Node::For { body, .. } | Node::Capture { body, .. } => collect_blocks(body, map)?,
            Node::Case { whens, otherwise, .. } => {
                for (_, b) in whens {
                    collect_blocks(b, map)?;
                }
                if let Some(b) = otherwise {
                    collect_blocks(b, map)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Replaces the body of every block that `ov` overrides. Blocks nested inside an overriding body are left as written.
pub(crate) fn substitute(nodes: Vec<Node>, ov: &HashMap<String, Vec<Node>>) -> Vec<Node> {
    nodes
        .into_iter()
        .map(|n| match n {
            Node::Block { name, body, line } => {
                let body = ov.get(&name).cloned().unwrap_or_else(|| substitute(body, ov));
                Node::Block { name, body, line }
            }
            Node::If(branches) => Node::If(branches.into_iter().map(|(c, b, l)| (c, substitute(b, ov), l)).collect()),
            Node::For { var, list, body, line } => Node::For { var, list, body: substitute(body, ov), line },
            Node::Capture { name, body, line } => Node::Capture { name, body: substitute(body, ov), line },
            Node::Case { subject, whens, otherwise } => Node::Case {
                subject,
                whens: whens.into_iter().map(|(v, b)| (v, substitute(b, ov))).collect(),
                otherwise: otherwise.map(|b| substitute(b, ov)),
            },
            n => n,
        })
        .collect()
}

/// Parses a template and resolves `{% extends %}`: the parent is loaded (recursively) and this template's blocks replace the parent's.
// ponytail: block bodies are cloned while resolving; fine for page-sized templates.
pub(crate) fn load(src: &str, partials: &Partials, depth: usize) -> Result<Vec<Node>, String> {
    let toks = tokenize(src)?;
    let (nodes, _) = block(&toks, &mut 0, &[], 0)?;
    let mut own = HashMap::new();
    collect_blocks(&nodes, &mut own)?;
    let Some(i) = nodes.iter().position(|n| matches!(n, Node::Extends(..))) else {
        return Ok(nodes);
    };
    let Node::Extends(parent, line) = &nodes[i] else { unreachable!() };
    let first = nodes[..i].iter().all(|n| matches!(n, Node::Text(t) if t.trim().is_empty()));
    if !first || nodes[i + 1..].iter().any(|n| matches!(n, Node::Extends(..))) {
        return Err(at("extends must be the first tag of a template, and used once", *line));
    }
    if depth >= MAX_INCLUDE_DEPTH {
        return Err(at(format!("Extends nesting too deep (max {MAX_INCLUDE_DEPTH}) at '{parent}'"), *line));
    }
    let psrc = partials.get(parent).ok_or_else(|| at(format!("Unknown template: {parent}"), *line))?;
    let base = load(psrc, partials, depth + 1).map_err(|e| format!("{e} (in '{parent}')"))?;
    Ok(substitute(base, &own))
}
