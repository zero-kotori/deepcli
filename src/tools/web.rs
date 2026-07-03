use serde_json::Value;

pub(super) fn format_web_fetch_text(body: &str, content_type: &str) -> String {
    if content_type.to_ascii_lowercase().contains("html") || body.contains("<html") {
        html_to_text(body)
    } else {
        collapse_blank_lines(body)
    }
}

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

fn html_to_text(body: &str) -> String {
    let without_scripts = strip_html_block(body, "script");
    let without_styles = strip_html_block(&without_scripts, "style");
    let mut text = String::new();
    let mut in_tag = false;
    for ch in without_styles.chars() {
        match ch {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    collapse_blank_lines(&decode_basic_entities(&text))
}

fn strip_html_block(body: &str, tag: &str) -> String {
    let lower = body.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut output = String::new();
    let mut index = 0usize;
    while let Some(start) = lower[index..].find(&open).map(|offset| index + offset) {
        output.push_str(&body[index..start]);
        let after_start = lower[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .unwrap_or(body.len());
        let end = lower[after_start..]
            .find(&close)
            .map(|offset| after_start + offset + close.len())
            .unwrap_or(after_start);
        index = end;
    }
    output.push_str(&body[index..]);
    output
}

fn decode_basic_entities(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn collapse_blank_lines(value: &str) -> String {
    let mut lines = Vec::new();
    let mut previous_blank = false;
    for line in value.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        let blank = line.is_empty();
        if blank && previous_blank {
            continue;
        }
        lines.push(line);
        previous_blank = blank;
    }
    lines.join("\n").trim().to_string()
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

    #[test]
    fn formats_web_fetch_html_as_readable_text() {
        let output = format_web_fetch_text(
            "<html><head><style>.x{}</style><script>alert(1)</script></head><body><h1>Title</h1><p>A &amp; B</p></body></html>",
            "text/html",
        );

        assert!(output.contains("Title"));
        assert!(output.contains("A & B"));
        assert!(!output.contains("alert"));
        assert!(!output.contains(".x"));
    }
}
