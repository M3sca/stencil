//! Node.js binding: `render(template, context)` where `context` is a plain JS object.

use crate::{Context, Value};
use napi_derive::napi;

#[napi(ts_args_type = "template: string, context?: Record<string, unknown> | null, partials?: Record<string, string> | null")]
pub fn render(
    template: String,
    context: Option<serde_json::Value>,
    partials: Option<std::collections::HashMap<String, String>>,
) -> napi::Result<String> {
    let ctx: Context = match context.map(crate::json::from_json) {
        Some(Value::Map(m)) => m,
        None | Some(Value::Nil) => Context::new(),
        _ => return Err(napi::Error::from_reason("Context must be an object")),
    };
    crate::render_with(&template, &ctx, &partials.unwrap_or_default()).map_err(napi::Error::from_reason)
}
