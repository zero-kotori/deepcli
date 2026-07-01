use crate::privacy::{
    has_secret_value_marker as privacy_has_secret_value_marker, looks_sensitive,
    redact_sensitive_text,
};

use super::{compact_text_line, diff_line_counts, is_added_diff_line, review_path_from_diff_line};

pub(crate) fn review_diff(diff: &str) -> String {
    if diff.trim().is_empty() {
        return "auto-reviewer: no local diff to review".to_string();
    }

    let mut high = ReviewFindings::default();
    let mut medium = ReviewFindings::default();
    let mut low = ReviewFindings::default();
    let (added_lines, removed_lines) = diff_line_counts(diff);
    let mut current_path: Option<String> = None;
    let mut in_probable_test_context = false;

    for (index, line) in diff.lines().enumerate() {
        let line_number = index + 1;
        if let Some(path) = review_path_from_diff_line(line) {
            current_path = Some(path);
            in_probable_test_context = false;
        }
        if is_review_test_marker_line(line) {
            in_probable_test_context = true;
        }
        let path = current_path.as_deref();
        if is_sensitive_review_line(line, path, in_probable_test_context) {
            high.add(
                "added line appears to contain sensitive material",
                Some(review_finding_example(line_number, line)),
            );
        }
        if is_dangerous_command_review_line(line, path, in_probable_test_context) {
            high.add(
                "diff contains a dangerous command pattern",
                Some(review_finding_example(line_number, line)),
            );
        }
        if line.starts_with("diff --git") && review_path_touches_credentials(path) {
            high.add(
                "diff touches local credentials path",
                Some(review_finding_example(line_number, line)),
            );
        }
        if is_panic_prone_review_line(line, path, in_probable_test_context) {
            medium.add(
                "added Rust panic-prone call; confirm it is acceptable",
                Some(review_finding_example(line_number, line)),
            );
        }
    }

    if added_lines + removed_lines > 500 {
        medium.add("large diff; consider splitting review scope", None);
    }
    if high.is_empty() && medium.is_empty() {
        low.add("no obvious high-risk pattern found", None);
    }

    let mut report = vec![
        "auto-reviewer report".to_string(),
        format!("changed lines: +{added_lines} -{removed_lines}"),
    ];
    append_findings(&mut report, "high", &high);
    append_findings(&mut report, "medium", &medium);
    append_findings(&mut report, "low", &low);
    report.join("\n")
}

#[derive(Debug, Default)]
struct ReviewFindings {
    items: Vec<ReviewFinding>,
}

#[derive(Debug)]
struct ReviewFinding {
    message: &'static str,
    count: usize,
    examples: Vec<String>,
}

impl ReviewFindings {
    fn add(&mut self, message: &'static str, example: Option<String>) {
        if let Some(item) = self.items.iter_mut().find(|item| item.message == message) {
            item.count += 1;
            if let Some(example) = example {
                if item.examples.len() < 3 {
                    item.examples.push(example);
                }
            }
            return;
        }

        self.items.push(ReviewFinding {
            message,
            count: 1,
            examples: example.into_iter().collect(),
        });
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn review_path_touches_credentials(path: Option<&str>) -> bool {
    path.is_some_and(|path| {
        let normalized = path.replace('\\', "/");
        normalized == ".deepcli/credentials" || normalized.ends_with("/.deepcli/credentials")
    })
}

fn is_review_test_or_doc_path(path: Option<&str>) -> bool {
    path.is_some_and(|path| {
        let normalized = path.replace('\\', "/");
        normalized.starts_with("tests/")
            || normalized.contains("/tests/")
            || normalized.starts_with("docs/")
            || normalized.contains("/docs/")
            || normalized.ends_with("_test.rs")
            || normalized.ends_with(".md")
            || normalized.ends_with(".rst")
            || normalized.ends_with(".txt")
    })
}

fn is_review_test_marker_line(line: &str) -> bool {
    let text = diff_line_text(line).trim();
    text.starts_with("#[test]") || text.starts_with("#[tokio::test]") || text.contains("mod tests")
}

fn diff_line_text(line: &str) -> &str {
    match line.as_bytes().first() {
        Some(b'+' | b'-' | b' ') => &line[1..],
        _ => line,
    }
}

fn is_sensitive_review_line(
    line: &str,
    path: Option<&str>,
    in_probable_test_context: bool,
) -> bool {
    if !is_added_diff_line(line) || is_review_test_or_doc_path(path) || in_probable_test_context {
        return false;
    }
    let text = diff_line_text(line).trim();
    if text.starts_with("//") || text.starts_with('#') || text.starts_with('*') {
        return false;
    }
    if !looks_sensitive(text) {
        return false;
    }
    if is_sensitive_review_detector_source_line(text) {
        return false;
    }
    if has_explicit_secret_review_marker(text) {
        return true;
    }
    !is_safe_sensitive_review_source_line(text)
}

fn is_sensitive_review_detector_source_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let defines_secret_marker = lower.contains("lower.contains(")
        && (lower.contains("sk-")
            || lower.contains("bearer ")
            || lower.contains("-----begin private key-----"));
    let defines_api_key_rule = (lower.contains("lower.contains(")
        || lower.contains("lower.starts_with("))
        && (lower.contains("api_key") || lower.contains("apikey"));
    let defines_api_key_trim_rule = lower.contains("trim_end_matches") && lower.contains("api_key");
    lower.contains("has_explicit_secret_review_marker")
        || lower.contains("secret_markers")
        || lower.contains("secret_value_markers")
        || lower.contains("sensitive_header_markers")
        || lower.contains("has_secret_value_marker")
        || lower.contains("has_sensitive_header_marker")
        || lower.contains("contains_sk_secret_marker")
        || lower.contains("privacy_has_secret_value_marker")
        || lower.contains("mentions_api_key")
        || lower.contains("defines_api_key_rule")
        || lower.contains("defines_api_key_trim_rule")
        || lower.contains("safe_api_key_source_reference")
        || defines_secret_marker
        || defines_api_key_rule
        || defines_api_key_trim_rule
}

fn has_explicit_secret_review_marker(text: &str) -> bool {
    privacy_has_secret_value_marker(text)
}

fn is_safe_sensitive_review_source_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("<redacted>")
        || lower.contains("redacted")
        || lower.contains("secret_review_marker")
        || lower.contains("secret_marker")
        || lower.contains("secret_markers")
        || lower.contains("looks_sensitive")
        || lower.contains("redact_sensitive")
        || lower.contains("is_sensitive_key")
        || lower.contains("sensitive material")
        || lower.contains("look like secrets")
    {
        return true;
    }
    if lower.contains("authorization: {}")
        || (lower.contains('$') && lower.contains("_api_key"))
        || (lower.contains("provider api keys") && lower.contains("_api_key"))
        || (lower.contains("format!(") && lower.contains("_api_key"))
        || lower.contains("api_key={}")
        || lower.contains("apikey redacted")
        || lower.contains("apikey must not be empty")
        || lower.contains("apikey for provider")
        || lower.contains("api_key missing")
        || lower.contains("api_key=missing")
        || lower.contains("api_key=configured")
    {
        return true;
    }
    let mentions_api_key = lower.contains("api_key") || lower.contains("apikey");
    let safe_api_key_source_reference = lower.contains(".api_key")
        || lower.contains(" api_key:")
        || lower.starts_with("api_key:")
        || lower.contains("api_key: string")
        || lower.contains("api_key: option")
        || lower.contains("let api_key =")
        || lower.contains("let mut api_key")
        || lower.contains("file_api_key")
        || lower.contains("read_api_key")
        || lower.contains("set_credentials_api_key")
        || lower.contains("store_provider_api_key")
        || lower.contains("provider_api_key")
        || lower.contains("provider_env_key")
        || lower.contains("api_key.trim")
        || lower.contains("api_key.pop")
        || lower.contains("api_key.push")
        || lower.contains("api_key.is_empty")
        || lower.contains("&mut api_key")
        || lower.contains("ok(api_key)")
        || lower.trim_end_matches(',') == "api_key";
    if mentions_api_key && safe_api_key_source_reference {
        return true;
    }
    false
}

fn is_dangerous_command_review_line(
    line: &str,
    path: Option<&str>,
    in_probable_test_context: bool,
) -> bool {
    if !is_added_diff_line(line) || is_review_test_or_doc_path(path) || in_probable_test_context {
        return false;
    }
    let text = diff_line_text(line).trim();
    if text.starts_with("//") || text.starts_with('#') || text.starts_with('*') {
        return false;
    }
    if is_review_detector_literal_line(text) {
        return false;
    }
    text.contains("rm -rf") || text.contains("git reset --hard")
}

fn is_review_detector_literal_line(text: &str) -> bool {
    (text.contains("rm -rf") || text.contains("git reset --hard")) && text.contains(".contains(")
}

fn is_panic_prone_review_line(
    line: &str,
    path: Option<&str>,
    in_probable_test_context: bool,
) -> bool {
    if !is_added_diff_line(line) || is_review_test_or_doc_path(path) || in_probable_test_context {
        return false;
    }
    let text = diff_line_text(line).trim();
    if text.starts_with("//")
        || text.starts_with("assert!")
        || text.starts_with("assert_eq!")
        || text.starts_with("assert_ne!")
        || is_panic_review_detector_source_line(text)
        || is_documented_invariant_expect_line(text)
    {
        return false;
    }
    text.contains("unwrap()") || text.contains("expect(")
}

fn is_panic_review_detector_source_line(text: &str) -> bool {
    text.contains("text.contains(\"unwrap()\")")
        || text.contains("text.contains(\"expect(\")")
        || text.contains("is_documented_invariant_expect_line")
}

fn is_documented_invariant_expect_line(text: &str) -> bool {
    if !text.contains("expect(") {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    lower.contains("checked")
        || lower.contains("validated")
        || lower.contains("guaranteed")
        || lower.contains("known")
        || lower.contains("already")
        || lower.contains("invariant")
}

fn review_finding_example(line_number: usize, line: &str) -> String {
    let redacted = redact_sensitive_text(line);
    format!("line {line_number}: {}", compact_text_line(&redacted, 180))
}

pub(crate) fn review_worktree(status: &str, diff: &str) -> String {
    let mut report = review_diff(diff);
    let untracked = status
        .lines()
        .filter_map(|line| line.strip_prefix("?? "))
        .collect::<Vec<_>>();
    if !untracked.is_empty() {
        report.push_str("\nworktree:");
        report.push_str(&format!("\n- untracked files: {}", untracked.len()));
        for path in untracked.iter().take(8) {
            report.push_str(&format!("\n  - {path}"));
        }
        if untracked.len() > 8 {
            report.push_str("\n  - ...");
        }
    }
    report
}

fn append_findings(report: &mut Vec<String>, label: &str, findings: &ReviewFindings) {
    if findings.is_empty() {
        return;
    }
    report.push(format!("{label}:"));
    for finding in &findings.items {
        if finding.count == 1 {
            report.push(format!("- {}", finding.message));
        } else {
            report.push(format!(
                "- {} ({} occurrences)",
                finding.message, finding.count
            ));
        }
        for example in &finding.examples {
            report.push(format!("  example: {example}"));
        }
    }
}
