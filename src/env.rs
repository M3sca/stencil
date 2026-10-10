use crate::parse::Node;
use crate::{Context, Partials, Value};
use std::collections::HashMap;
use std::rc::Rc;

/// Caller context plus loop-local variables (a stack: no clone per iteration).
pub(crate) struct Env<'a> {
    pub(crate) globals: &'a Context,
    pub(crate) locals: Vec<(String, Value)>,
    pub(crate) partials: &'a Partials,
    pub(crate) depth: usize,
    pub(crate) includes: usize,
    pub(crate) iterations: usize,
    pub(crate) steps: usize,
    pub(crate) level: usize,
    /// Parsed partials by name: each is parsed once per render, however often it is included.
    pub(crate) cache: HashMap<String, Rc<Vec<Node>>>,
}

impl Env<'_> {
    pub(crate) fn get(&self, k: &str) -> Option<&Value> {
        self.locals.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v).or_else(|| self.globals.get(k))
    }
}
