use anyhow::{bail, Context, Result};
use reqwest::{redirect::Policy, Client, Response, Url};
use serde_json::Value;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

const MAX_WEB_REDIRECTS: usize = 5;
pub(super) const DEFAULT_WEB_FETCH_CHARS: usize = 20_000;
pub(super) const MAX_WEB_FETCH_CHARS: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BoundedResponseBody {
    pub text: String,
    pub truncated: bool,
    pub downloaded_bytes: usize,
}

pub(super) async fn safe_web_get(
    initial_url: Url,
    timeout: Duration,
    user_agent: &str,
) -> Result<(Url, Response)> {
    let mut url = initial_url;
    for redirect_count in 0..=MAX_WEB_REDIRECTS {
        let client = safe_web_client(&url, timeout, user_agent).await?;
        let response = client.get(url.clone()).send().await?;
        if !response.status().is_redirection() {
            return Ok((url, response));
        }
        if redirect_count == MAX_WEB_REDIRECTS {
            bail!("web_fetch exceeded {MAX_WEB_REDIRECTS} redirects");
        }
        let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
            return Ok((url, response));
        };
        let location = location
            .to_str()
            .context("web_fetch redirect location is not valid text")?;
        url = url
            .join(location)
            .with_context(|| format!("invalid web_fetch redirect target `{location}`"))?;
    }
    unreachable!("bounded redirect loop always returns or errors")
}

pub(super) async fn read_bounded_response_body(
    mut response: Response,
    max_chars: usize,
) -> Result<BoundedResponseBody> {
    let max_chars = max_chars.clamp(1, MAX_WEB_FETCH_CHARS);
    let max_bytes = max_chars.saturating_mul(4);
    let mut body = Vec::with_capacity(max_bytes.min(64 * 1024));
    let mut truncated = response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64);

    while body.len() < max_bytes {
        let Some(chunk) = response.chunk().await? else {
            break;
        };
        let remaining = max_bytes - body.len();
        if chunk.len() > remaining {
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
        if body.len() == max_bytes {
            truncated = true;
            break;
        }
    }

    let downloaded_bytes = body.len();
    Ok(BoundedResponseBody {
        text: String::from_utf8_lossy(&body).into_owned(),
        truncated,
        downloaded_bytes,
    })
}

pub(super) fn bounded_web_fetch_chars(value: Option<u64>) -> usize {
    value
        .unwrap_or(DEFAULT_WEB_FETCH_CHARS as u64)
        .clamp(1, MAX_WEB_FETCH_CHARS as u64) as usize
}

async fn safe_web_client(url: &Url, timeout: Duration, user_agent: &str) -> Result<Client> {
    validate_web_url(url)?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("web_fetch URL must include a host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("web_fetch URL must include a supported port"))?;
    let mut builder = Client::builder()
        .timeout(timeout)
        .user_agent(user_agent)
        .redirect(Policy::none())
        .no_proxy();
    if let Ok(ip) = host.parse::<IpAddr>() {
        ensure_public_web_ip(ip)?;
    } else {
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .with_context(|| format!("failed to resolve web_fetch host `{host}`"))?
            .collect::<Vec<SocketAddr>>();
        if addresses.is_empty() {
            bail!("web_fetch host `{host}` did not resolve to an address");
        }
        for address in &addresses {
            ensure_public_web_ip(address.ip())?;
        }
        builder = builder.resolve_to_addrs(host, &addresses);
    }
    builder.build().context("failed to build safe web client")
}

fn validate_web_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        bail!("web_fetch only supports http or https URLs");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("web_fetch URLs cannot contain embedded credentials");
    }
    if let Some(host) = url.host_str() {
        if let Ok(ip) = host.parse::<IpAddr>() {
            ensure_public_web_ip(ip)?;
        }
    }
    Ok(())
}

fn ensure_public_web_ip(ip: IpAddr) -> Result<()> {
    let blocked = match ip {
        IpAddr::V4(ip) => is_blocked_ipv4(ip),
        IpAddr::V6(ip) => is_blocked_ipv6(ip),
    };
    if blocked {
        bail!("web_fetch refused a non-public network address");
    }
    Ok(())
}

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 198 && matches!(b, 18 | 19))
        || a >= 240
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_blocked_ipv4(mapped);
    }
    let first = ip.segments()[0];
    let ipv4_compatible = ip.segments()[..6].iter().all(|segment| *segment == 0);
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first & 0xffc0) == 0xfec0
        || ipv4_compatible
        || (ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8)
}

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
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

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

    #[test]
    fn web_fetch_network_policy_rejects_local_and_private_addresses() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.169.254",
            "192.168.1.1",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            let ip = address.parse::<IpAddr>().unwrap();
            assert!(ensure_public_web_ip(ip).is_err(), "{address}");
        }
        for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            let ip = address.parse::<IpAddr>().unwrap();
            assert!(ensure_public_web_ip(ip).is_ok(), "{address}");
        }
    }

    #[test]
    fn web_fetch_network_policy_rejects_embedded_credentials_and_private_redirects() {
        let credentials = Url::parse("https://user:pass@example.com/").unwrap();
        assert!(validate_web_url(&credentials).is_err());

        let public = Url::parse("https://example.com/start").unwrap();
        let redirected = public
            .join("http://169.254.169.254/latest/meta-data")
            .unwrap();
        assert!(validate_web_url(&redirected).is_err());
    }

    #[test]
    fn web_fetch_character_limit_is_host_bounded() {
        assert_eq!(bounded_web_fetch_chars(None), DEFAULT_WEB_FETCH_CHARS);
        assert_eq!(bounded_web_fetch_chars(Some(0)), 1);
        assert_eq!(bounded_web_fetch_chars(Some(u64::MAX)), MAX_WEB_FETCH_CHARS);
    }

    #[tokio::test]
    async fn response_body_reader_stops_at_the_host_byte_limit() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let body = "x".repeat(1024);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
        let response = Client::new()
            .get(format!("http://{address}/large"))
            .send()
            .await
            .unwrap();

        let body = read_bounded_response_body(response, 10).await.unwrap();

        assert!(body.truncated);
        assert_eq!(body.downloaded_bytes, 40);
        assert_eq!(body.text.len(), 40);
        server.await.unwrap();
    }
}
