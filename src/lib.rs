//! Minimal template engine:
//! `{{ var | filter: arg }}`, `{% if/elsif/else %}`, `{% for x in xs %}`,
//! `{% include %}`, `{% extends %}`/`{% block %}`.

#[cfg(feature = "node")]
mod node;
#[cfg(feature = "wasm")]
mod wasm;
#[cfg(any(feature = "node", feature = "wasm"))]
mod json;

mod env;
mod expr;
mod filters;
mod inherit;
mod limits;
mod parse;
mod render;
mod value;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

pub use filters::FilterSet;
pub use render::{render, render_with, render_with_filters};
pub use value::Value;

pub type Context = HashMap<String, Value>;

/// Named templates available to `{% include "name" %}`. Names are plain map keys:
/// they are never interpreted as paths and nothing here touches the filesystem.
pub type Partials = HashMap<String, String>;
