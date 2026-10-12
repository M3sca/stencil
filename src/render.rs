//! Executes a parsed template.

use crate::env::Env;
use crate::expr::{condition, output, value};
use crate::filters::NoFilters;
use crate::inherit::load;
use crate::limits::*;
use crate::parse::{Node, at};
use crate::{Context, FilterSet, Partials, Value};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

/// How a run of nodes ended: normally, or by `break`/`continue` on its way up to the loop.
#[derive(Clone, Copy, PartialEq)]
enum Flow {
    Normal,
    Break,
    Continue,
}

fn emit(out: &mut String, s: &str) -> Result<(), String> {
    if out.len() + s.len() > MAX_OUTPUT {
        return Err(format!("Output too large (max {MAX_OUTPUT} bytes)"));
    }
    out.push_str(s);
    Ok(())
}

fn run(nodes: &[Node], env: &mut Env, out: &mut String) -> Result<Flow, String> {
    env.level += 1;
    let r = if env.level > MAX_RUN_DEPTH {
        Err(format!("Template nested too deeply (max {MAX_RUN_DEPTH} levels, includes included)"))
    } else {
        run_nodes(nodes, env, out)
    };
    env.level -= 1;
    r
}

fn run_nodes(nodes: &[Node], env: &mut Env, out: &mut String) -> Result<Flow, String> {
    for n in nodes {
        env.charge(1)?;
        // A nested run that ended in `break`/`continue` ends this one too, up to the loop.
        macro_rules! nested {
            ($body:expr) => {
                match run($body, env, out)? {
                    Flow::Normal => {}
                    flow => return Ok(flow),
                }
            };
        }
        match n {
            Node::Text(t) => emit(out, t)?,
            Node::Out { expr, line } => {
                let shown = output(expr, env).map_err(|e| at(e, *line))?.show();
                emit(out, &shown).map_err(|e| at(e, *line))?
            }
            Node::If(branches) => {
                for (c, body, line) in branches {
                    if condition(c, env).map_err(|e| at(e, *line))? {
                        nested!(body);
                        break;
                    }
                }
            }
            Node::Case { subject, whens, otherwise } => {
                let subject = value(subject, env)?.into_owned();
                let mut hit = None;
                'search: for w in whens {
                    for v in &w.0 {
                        if *value(v, env)? == subject {
                            hit = Some(w);
                            break 'search;
                        }
                    }
                }
                match (hit, otherwise) {
                    (Some((_, body)), _) | (None, Some(body)) => nested!(body),
                    (None, None) => {}
                }
            }
            Node::Assign { name, expr, line } => {
                let v = output(expr, env).map_err(|e| at(e, *line))?.into_owned();
                env.assign(name, v).map_err(|e| at(e, *line))?;
            }
            Node::Capture { name, body, line } => {
                let mut buf = String::new();
                let flow = run(body, env, &mut buf)?;
                env.assign(name, Value::Str(buf)).map_err(|e| at(e, *line))?;
                if flow != Flow::Normal {
                    return Ok(flow);
                }
            }
            Node::Break(line) | Node::Continue(line) => {
                if env.loops == 0 {
                    let what = if matches!(n, Node::Break(_)) { "break" } else { "continue" };
                    return Err(at(format!("{what} outside a loop"), *line));
                }
                return Ok(if matches!(n, Node::Break(_)) { Flow::Break } else { Flow::Continue });
            }
            Node::Include { name, args, line } => {
                let src = env.partials.get(name).ok_or_else(|| at(format!("Unknown template: {name}"), *line))?;
                if env.depth >= MAX_INCLUDE_DEPTH {
                    return Err(at(format!("Include nesting too deep (max {MAX_INCLUDE_DEPTH}) at '{name}'"), *line));
                }
                env.includes += 1;
                if env.includes > MAX_INCLUDES {
                    return Err(at(format!("Too many includes (max {MAX_INCLUDES})"), *line));
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
                let extra = args.iter().map(|(k, e)| Ok((k.clone(), value(e, env)?.into_owned()))).collect::<Result<Vec<_>, String>>()?;
                let base = env.locals.len();
                env.locals.extend(extra);
                env.depth += 1;
                // `break`/`continue` can't cross the include boundary.
                let outer_loops = std::mem::replace(&mut env.loops, 0);
                let r = run(&nodes, env, out).map_err(|e| format!("{e} (in '{name}')"));
                env.loops = outer_loops;
                env.depth -= 1;
                env.locals.truncate(base);
                r?;
            }
            Node::Block { body, .. } => nested!(body),
            Node::Extends(_, line) => return Err(at("extends must be the first tag of a template", *line)),
            Node::For { var, list, body, line } => {
                // ponytail: the list is cloned once per loop (so env isn't kept borrowed); use Rc if it gets costly.
                let items = match output(list, env).map_err(|e| at(e, *line))?.into_owned() {
                    Value::List(l) => l,
                    Value::Nil => vec![],
                    v => vec![v],
                };
                env.charge(items.iter().map(Value::weight).sum::<usize>() / 256)?;
                let (base, len) = (env.locals.len(), items.len());
                // The enclosing loop's metadata (if any) becomes `forloop.parentloop`.
                let parent = env.locals.iter().rev().find(|(n, _)| n == "forloop").map(|(_, v)| v.clone());
                env.loops += 1;
                let mut result = Ok(());
                for (k, item) in items.into_iter().enumerate() {
                    env.iterations += 1;
                    if env.iterations > MAX_ITERATIONS {
                        result = Err(at(format!("Too many loop iterations (max {MAX_ITERATIONS})"), *line));
                        break;
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
                    match r {
                        Ok(Flow::Break) => break,
                        Ok(_) => {}
                        Err(e) => {
                            result = Err(e);
                            break;
                        }
                    }
                }
                env.loops -= 1;
                result?;
            }
        }
    }
    Ok(Flow::Normal)
}

pub fn render(template: &str, ctx: &Context) -> Result<String, String> {
    render_with(template, ctx, &Partials::new())
}

/// Like [`render`], with named templates that the template can `{% include "name" %}`.
pub fn render_with(template: &str, ctx: &Context, partials: &Partials) -> Result<String, String> {
    render_with_filters(template, ctx, partials, &NoFilters)
}

/// Like [`render_with`], with filters supplied by the caller (see [`FilterSet`]).
pub fn render_with_filters(template: &str, ctx: &Context, partials: &Partials, filters: &dyn FilterSet) -> Result<String, String> {
    let nodes = load(template, partials, 0)?;
    let mut out = String::new();
    let mut env = Env {
        globals: ctx,
        locals: Vec::new(),
        partials,
        custom: filters,
        depth: 0,
        includes: 0,
        iterations: 0,
        steps: Cell::new(0),
        level: 0,
        calls: Cell::new(0),
        loops: 0,
        assigned: 0,
        cache: HashMap::new(),
    };
    run(&nodes, &mut env, &mut out)?;
    Ok(out)
}
