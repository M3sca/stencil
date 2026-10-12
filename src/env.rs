use crate::FilterSet;
use crate::limits::*;
use crate::parse::Node;
use crate::{Context, Partials, Value};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

/// Caller context plus loop-local variables (a stack: no clone per iteration).
pub(crate) struct Env<'a> {
    pub(crate) globals: &'a Context,
    pub(crate) locals: Vec<(String, Value)>,
    pub(crate) partials: &'a Partials,
    pub(crate) custom: &'a dyn FilterSet,
    pub(crate) depth: usize,
    pub(crate) includes: usize,
    pub(crate) iterations: usize,
    pub(crate) steps: Cell<usize>,
    pub(crate) level: usize,
    /// Function calls being evaluated right now (they nest through their arguments).
    pub(crate) calls: Cell<usize>,
    /// Loops currently being executed in the running template (`break`/`continue` need one).
    pub(crate) loops: usize,
    /// Bytes stored by `assign`/`capture` so far.
    pub(crate) assigned: usize,
    /// Parsed partials by name: each is parsed once per render, however often it is included.
    pub(crate) cache: HashMap<String, Rc<Vec<Node>>>,
}

impl Env<'_> {
    pub(crate) fn get(&self, k: &str) -> Option<&Value> {
        self.locals.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v).or_else(|| self.globals.get(k))
    }

    /// Counts `n` units of work; fails once a render has done too much.
    pub(crate) fn charge(&self, n: usize) -> Result<(), String> {
        let steps = self.steps.get().saturating_add(n);
        self.steps.set(steps);
        if steps > MAX_STEPS {
            return Err(format!("Template too complex (max {MAX_STEPS} steps)"));
        }
        Ok(())
    }

    /// `assign`/`capture`: updates the nearest variable with that name, or creates one
    /// that lives until the end of the enclosing loop iteration or include.
    pub(crate) fn assign(&mut self, name: &str, v: Value) -> Result<(), String> {
        let w = v.weight();
        if w > MAX_VALUE_SIZE {
            return Err(format!("Value too large (max {MAX_VALUE_SIZE} bytes)"));
        }
        self.assigned += w;
        if self.assigned > MAX_ASSIGNED {
            return Err(format!("Too much data stored by assign/capture (max {MAX_ASSIGNED} bytes)"));
        }
        self.charge(w / 256)?;
        match self.locals.iter_mut().rev().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = v,
            None => self.locals.push((name.to_string(), v)),
        }
        Ok(())
    }
}
