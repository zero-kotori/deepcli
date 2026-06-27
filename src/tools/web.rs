use serde_json::Value;

pub(super) fn format_web_search_result(query: &str, value: &Value) -> String {
    let mut lines = Vec::new();
    let heading = value
        .get("Heading")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());
    let answer = value
        .get("Answer")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());
    let abstract_text = value
        .get("AbstractText")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());
    let abstract_url = value
        .get("AbstractURL")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());

    if let Some(heading) = heading {
        lines.push(format!("heading: {}", heading.trim()));
    }
    if let Some(answer) = answer {
        lines.push(format!("answer: {}", answer.trim()));
    }
    if let Some(abstract_text) = abstract_text {
        lines.push(format!("summary: {}", abstract_text.trim()));
    }
    if let Some(abstract_url) = abstract_url {
        lines.push(format!("source: {}", abstract_url.trim()));
    }

    let related = collect_web_related_topics(value, 5);
    if !related.is_empty() {
        lines.push("related:".to_string());
        for (text, url) in related {
            match url {
                Some(url) if !url.trim().is_empty() => {
                    lines.push(format!("  - {} ({})", text.trim(), url.trim()))
                }
                _ => lines.push(format!("  - {}", text.trim())),
            }
        }
    }

    if lines.is_empty() {
        format!("no web search summary found for `{query}`")
    } else {
        lines.join("\n")
    }
}

fn collect_web_related_topics(value: &Value, limit: usize) -> Vec<(String, Option<String>)> {
    let mut results = Vec::new();
    if let Some(topics) = value.get("RelatedTopics") {
        collect_web_related_topic_values(topics, limit, &mut results);
    }
    results
}

fn collect_web_related_topic_values(
    value: &Value,
    limit: usize,
    results: &mut Vec<(String, Option<String>)>,
) {
    if results.len() >= limit {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_web_related_topic_values(item, limit, results);
                if results.len() >= limit {
                    break;
                }
            }
        }
        Value::Object(map) => {
            if let Some(text) = map
                .get("Text")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
            {
                let url = map
                    .get("FirstURL")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                results.push((text.to_string(), url));
                return;
            }
            if let Some(topics) = map.get("Topics") {
                collect_web_related_topic_values(topics, limit, results);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_web_search_result_with_related_topic_fallback() {
        let value = json!({
            "Heading": "",
            "AbstractText": "",
            "RelatedTopics": [
                {
                    "Text": "Rust is a systems programming language.",
                    "FirstURL": "https://duckduckgo.com/Rust"
                },
                {
                    "Topics": [
                        {
                            "Text": "Ownership is Rust's memory model.",
                            "FirstURL": "https://duckduckgo.com/Ownership"
                        }
                    ]
                }
            ]
        });
        let output = format_web_search_result("rust ownership", &value);
        assert!(output.contains("related:"));
        assert!(output.contains("Rust is a systems programming language."));
        assert!(output.contains("Ownership is Rust's memory model."));
    }

    #[test]
    fn formats_web_search_empty_result_with_query() {
        let output = format_web_search_result("unknown", &json!({}));
        assert_eq!(output, "no web search summary found for `unknown`");
    }
}
