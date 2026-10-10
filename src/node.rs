//! Node.js binding: `render(template, context)` where `context` is a plain JS object.

use crate::{Context, Value};
use napi_derive::napi;

fn convert(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => Value::Num(n.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(s) => Value::Str(s),
        serde_json::Value::Array(a) => Value::List(a.into_iter().map(convert).collect()),
        serde_json::Value::Object(o) => Value::Map(o.into_iter().map(|(k, v)| (k, convert(v))).collect()),
    }
}

#[napi(ts_args_type = "template: string, context?: Record<string, unknown> | null, partials?: Record<string, string> | null")]
pub fn render(
    template: String,
    context: Option<serde_json::Value>,
    partials: Option<std::collections::HashMap<String, String>>,
) -> napi::Result<String> {
    let ctx: Context = match context.map(convert) {
        Some(Value::Map(m)) => m,
        None | Some(Value::Nil) => Context::new(),
        _ => return Err(napi::Error::from_reason("Context must be an object")),
    };
    crate::render_with(&template, &ctx, &partials.unwrap_or_default()).map_err(napi::Error::from_reason)
}
