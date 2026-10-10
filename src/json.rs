//! Conversion between `serde_json::Value` (what the bindings receive and return) and `Value`.

use crate::Value;

pub(crate) fn from_json(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => Value::Num(n.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(s) => Value::Str(s),
        serde_json::Value::Array(a) => Value::List(a.into_iter().map(from_json).collect()),
        serde_json::Value::Object(o) => Value::Map(o.into_iter().map(|(k, v)| (k, from_json(v))).collect()),
    }
}

pub(crate) fn to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Nil => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Num(n) => serde_json::Number::from_f64(*n).map_or(serde_json::Value::Null, serde_json::Value::Number),
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::List(l) => serde_json::Value::Array(l.iter().map(to_json).collect()),
        Value::Map(m) => serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), to_json(v))).collect()),
    }
}
