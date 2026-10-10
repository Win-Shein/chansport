pub mod admin;
pub mod auth;
pub mod member;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Strip HTML angle brackets (stored-XSS guard) and cap the length.
pub fn clean_string(val: &str, max_len: usize) -> String {
    val.replace(['<', '>'], "").trim().chars().take(max_len).collect()
}

pub fn js_str(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

pub fn js_num(v: &Value, k: &str) -> f64 {
    match v.get(k) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

pub fn form_str(m: &HashMap<String, String>, k: &str) -> String {
    m.get(k).cloned().unwrap_or_default()
}

pub fn form_num(m: &HashMap<String, String>, k: &str) -> f64 {
    m.get(k)
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(0.0)
}

pub fn is_valid_id(v: &str) -> bool {
    v.parse::<i64>().map(|n| n > 0).unwrap_or(false)
}

pub fn json_err(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Json(json!({ "success": false, "message": msg })),
    )
        .into_response()
}

pub fn json_ok(msg: &str) -> Response {
    Json(json!({ "success": true, "message": msg })).into_response()
}
