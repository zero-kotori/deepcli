use serde_json::Value;

const SECRET_MARKERS: &[&str] = &[
    "authorization:",
    "bearer ",
    "sk-",
    "-----BEGIN PRIVATE KEY-----",
];

pub fn looks_sensitive(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = text.to_ascii_lowercase();
    if SECRET_MARKERS
        .iter()
        .any(|marker| lower.contains(&marker.to_ascii_lowercase()))
    {
        return true;
    }

    if is_sensitive_key(&normalize_key(trimmed)) {
        return true;
    }

    ['=', ':'].iter().any(|separator| {
        lower
            .split_once(*separator)
            .map(|(left, _)| is_sensitive_key(&last_key_segment(left)))
            .unwrap_or(false)
    })
}

pub fn redact_sensitive_text(text: &str) -> String {
    text.lines()
        .map(|line| {
            if looks_sensitive(line) {
                redact_line(line)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn redact_sensitive_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_sensitive_text(text)),
        Value::Array(items) => Value::Array(items.iter().map(redact_sensitive_value).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    if looks_sensitive(key) {
                        (key.clone(), Value::String("<redacted>".to_string()))
                    } else {
                        (key.clone(), redact_sensitive_value(value))
                    }
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn redact_line(line: &str) -> String {
    let separators = ['=', ':'];
    for separator in separators {
        if let Some((left, _right)) = line.split_once(separator) {
            return format!("{left}{separator} <redacted>");
        }
    }
    "<redacted sensitive line>".to_string()
}

fn normalize_key(value: &str) -> String {
    value
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .to_ascii_lowercase()
}

fn last_key_segment(value: &str) -> String {
    value
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .rfind(|part| !part.is_empty())
        .map(normalize_key)
        .unwrap_or_default()
}

fn is_sensitive_key(key: &str) -> bool {
    key == "token"
        || key.ends_with("_token")
        || key.ends_with("-token")
        || key == "password"
        || key.contains("secret")
        || key == "authorization"
        || key == "api-key"
        || key == "apikey"
        || key.contains("api_key")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_secret_like_lines() {
        assert_eq!(
            redact_sensitive_text("api_key = abc123"),
            "api_key = <redacted>"
        );
        assert_eq!(
            redact_sensitive_text("export TOKEN=abc123"),
            "export TOKEN= <redacted>"
        );
        assert_eq!(redact_sensitive_text("normal text"), "normal text");
    }

    #[test]
    fn does_not_redact_token_identifiers_in_source_code() {
        assert_eq!(redact_sensitive_text("mod token;"), "mod token;");
        assert_eq!(
            redact_sensitive_text("use crate::token::TokenKind;"),
            "use crate::token::TokenKind;"
        );
    }

    #[test]
    fn redacts_sensitive_json_keys() {
        let redacted = redact_sensitive_value(&json!({
            "apiKey": "secret",
            "nested": {"token": "abc"},
            "safe": "ok"
        }));
        assert_eq!(redacted["apiKey"], "<redacted>");
        assert_eq!(redacted["nested"]["token"], "<redacted>");
        assert_eq!(redacted["safe"], "ok");
    }
}
