use serde::Serialize;

pub fn render_json<T: Serialize>(value: &T) -> String {
    match serde_json::to_string_pretty(value) {
        Ok(text) => format!("{text}\n"),
        Err(e) => format!("{{\"status\":\"error\",\"message\":{}}}\n", serde_json::Value::String(e.to_string())),
    }
}
