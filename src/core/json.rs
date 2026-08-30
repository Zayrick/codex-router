use serde_json::{Map, Value};

pub type JsonObject = Map<String, Value>;

pub fn object(value: &Value) -> Option<&JsonObject> {
    value.as_object()
}

pub fn object_mut(value: &mut Value) -> Option<&mut JsonObject> {
    value.as_object_mut()
}

pub fn string_field<'a>(value: Option<&'a JsonObject>, key: &str) -> Option<&'a str> {
    value?.get(key)?.as_str().filter(|field| !field.is_empty())
}

pub fn number_field(value: Option<&JsonObject>, key: &str) -> Option<f64> {
    value?.get(key)?.as_f64().filter(|field| field.is_finite())
}

pub fn record_field<'a>(value: Option<&'a JsonObject>, key: &str) -> Option<&'a JsonObject> {
    value?.get(key)?.as_object()
}
