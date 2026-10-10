use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Value>),
    Map(HashMap<String, Value>),
}

impl From<&str> for Value { fn from(s: &str) -> Self { Value::Str(s.into()) } }
impl From<i64> for Value { fn from(n: i64) -> Self { Value::Num(n as f64) } }
impl From<bool> for Value { fn from(b: bool) -> Self { Value::Bool(b) } }
impl From<Vec<Value>> for Value { fn from(v: Vec<Value>) -> Self { Value::List(v) } }

impl Value {
    pub(crate) fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub(crate) fn show(&self) -> String {
        match self {
            Value::Nil => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => n.to_string(),
            Value::Str(s) => s.clone(),
            Value::List(l) => l.iter().map(Value::show).collect(),
            Value::Map(_) => String::new(),
        }
    }

    pub(crate) fn len(&self) -> Option<usize> {
        match self {
            Value::Str(s) => Some(s.chars().count()),
            Value::List(l) => Some(l.len()),
            Value::Map(m) => Some(m.len()),
            _ => None,
        }
    }
}
