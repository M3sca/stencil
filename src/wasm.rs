//! WebAssembly binding: `render(template, contextJson, partials?, filters?)`.
//!
//! The context is passed as a JSON string (cheaper than walking a JS object from Rust);
//! `partials` is an object of strings and `filters` an object of synchronous functions.

use crate::json::{from_json, to_json};
use crate::filters::NoFilters;
use crate::{Context, FilterSet, Value, render_with_filters};
use js_sys::{Array, Function, JSON, Object, Promise, Reflect};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

fn js_error(e: JsValue) -> String {
    match e.dyn_into::<js_sys::Error>() {
        Ok(err) => String::from(err.message()),
        Err(other) => other.as_string().unwrap_or_else(|| "unknown error".into()),
    }
}

fn to_js(v: &Value) -> Result<JsValue, String> {
    JSON::parse(&to_json(v).to_string()).map_err(js_error)
}

/// Own enumerable functions of a JS object, called synchronously from the template.
struct JsFilters(HashMap<String, Function>);

impl FilterSet for JsFilters {
    fn call(&self, name: &str, value: &Value, args: &[Value]) -> Option<Result<Value, String>> {
        let f = self.0.get(name)?;
        Some((|| {
            let argv = Array::new();
            argv.push(&to_js(value)?);
            for a in args {
                argv.push(&to_js(a)?);
            }
            let ret = f.apply(&JsValue::UNDEFINED, &argv).map_err(js_error)?;
            if ret.is_instance_of::<Promise>() {
                return Err("filters must be synchronous (got a Promise)".into());
            }
            if ret.is_undefined() {
                return Ok(Value::Nil);
            }
            let json = JSON::stringify(&ret).map_err(js_error)?;
            let json = json.as_string().ok_or("filter returned a value that is not JSON")?;
            Ok(from_json(serde_json::from_str(&json).map_err(|e| e.to_string())?))
        })())
    }
}

#[wasm_bindgen]
pub fn render(template: &str, context_json: &str, partials: JsValue, filters: JsValue) -> Result<String, JsError> {
    let ctx = match from_json(serde_json::from_str(context_json).map_err(|e| JsError::new(&format!("Invalid context JSON: {e}")))?) {
        Value::Map(m) => m,
        Value::Nil => Context::new(),
        _ => return Err(JsError::new("Context must be an object")),
    };
    let partials: HashMap<String, String> = if partials.is_undefined() || partials.is_null() {
        HashMap::new()
    } else {
        let json = JSON::stringify(&partials).map_err(|e| JsError::new(&js_error(e)))?;
        serde_json::from_str(&json.as_string().unwrap_or_default()).map_err(|e| JsError::new(&format!("partials must be an object of strings: {e}")))?
    };
    let custom;
    let set: &dyn FilterSet = if filters.is_undefined() || filters.is_null() {
        &NoFilters
    } else {
        let obj: Object = filters.dyn_into().map_err(|_| JsError::new("filters must be an object"))?;
        let mut funcs = HashMap::new();
        for key in Object::keys(&obj) {
            let name = key.as_string().unwrap_or_default();
            let f = Reflect::get(&obj, &key).map_err(|e| JsError::new(&js_error(e)))?;
            let f: Function = f.dyn_into().map_err(|_| JsError::new(&format!("filters.{name} must be a function")))?;
            funcs.insert(name, f);
        }
        custom = JsFilters(funcs);
        &custom
    };
    render_with_filters(template, &ctx, &partials, set).map_err(|e| JsError::new(&e))
}
