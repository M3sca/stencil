//! Hard limits that keep a render bounded, whatever the template does.

/// Depth of the include/extends chain.
pub(crate) const MAX_INCLUDE_DEPTH: usize = 32;
/// Includes in one render; bounds growth from templates that include each other many times over.
pub(crate) const MAX_INCLUDES: usize = 100_000;
/// Total loop iterations in one render (all loops, all includes).
pub(crate) const MAX_ITERATIONS: usize = 1_000_000;
/// Nodes executed in one render: bounds CPU however the work is spread over loops and includes.
pub(crate) const MAX_STEPS: usize = 2_000_000;
/// Nesting of blocks (`if`/`for`/`block`) and of parentheses in conditions; bounds recursion depth.
pub(crate) const MAX_NESTING: usize = 50;
/// Total nested `run` calls across blocks, includes and loops: bounds stack use for any template.
pub(crate) const MAX_RUN_DEPTH: usize = 100;
/// Maximum size of a rendered output, in bytes.
pub(crate) const MAX_OUTPUT: usize = 10 * 1024 * 1024;
