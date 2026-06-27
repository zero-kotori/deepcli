use super::{
    dedup_preserve_order, git_stdout, git_stdout_bytes, parse_positive_usize, required_arg,
    set_command_output_path, truncate_display, write_command_output, CommandExit,
};
use crate::config::{AppConfig, PrivacyConfig};
use crate::privacy::redact_sensitive_text;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn handle_privacy_scan(
    workspace: &Path,
    config: &AppConfig,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_privacy_scan_options(&args)?;
    let report = build_privacy_scan_report(workspace, &options, &config.privacy)?;
    let output = if options.json_output {
        format_privacy_scan_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_findings && report.actionable_finding_count() > 0 {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrivacyScanOptions {
    json_output: bool,
    output_path: Option<String>,
    fail_on_findings: bool,
    include_history: bool,
    max_revisions: usize,
}

impl Default for PrivacyScanOptions {
    fn default() -> Self {
        Self {
            json_output: false,
            output_path: None,
            fail_on_findings: false,
            include_history: true,
            max_revisions: 200,
        }
    }
}

fn parse_privacy_scan_options(args: &[String]) -> Result<PrivacyScanOptions> {
    let mut options = PrivacyScanOptions::default();
    let mut index = usize::from(args.first().is_some_and(|arg| arg == "scan"));
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut options.output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(
                    &mut options.output_path,
                    value.trim_start_matches("--output="),
                )?;
                index += 1;
            }
            "--fail-on-findings" | "--strict" => {
                options.fail_on_findings = true;
                index += 1;
            }
            "--no-history" => {
                options.include_history = false;
                index += 1;
            }
            "--history" => {
                options.include_history = true;
                index += 1;
            }
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "revision limit")?;
                options.max_revisions =
                    parse_positive_usize(raw, "revision limit")?.clamp(1, 10_000);
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                options.max_revisions =
                    parse_positive_usize(value.trim_start_matches("--limit="), "revision limit")?
                        .clamp(1, 10_000);
                index += 1;
            }
            value => bail!("unsupported /privacy option `{value}`"),
        }
    }
    Ok(options)
}

#[derive(Debug, Clone)]
struct PrivacyScanReport {
    git_present: bool,
    include_history: bool,
    revision_limit: usize,
    revisions_scanned: usize,
    tracked_sensitive_paths: Vec<String>,
    findings: Vec<PrivacyFinding>,
    suppressed_findings: Vec<PrivacySuppressedFinding>,
    next_actions: Vec<String>,
    report: String,
}

impl PrivacyScanReport {
    fn high_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == "high")
            .count()
    }

    fn medium_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == "medium")
            .count()
    }

    fn low_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == "low")
            .count()
    }

    fn actionable_finding_count(&self) -> usize {
        self.high_count() + self.medium_count()
    }

    fn occurrence_count(&self) -> usize {
        self.findings
            .iter()
            .map(|finding| finding.occurrences)
            .sum()
    }

    fn suppressed_occurrence_count(&self) -> usize {
        self.suppressed_findings
            .iter()
            .map(|finding| finding.occurrences)
            .sum()
    }

    fn status(&self) -> &'static str {
        if !self.git_present {
            "no_git"
        } else if self.high_count() > 0 {
            "high_risk"
        } else if self.medium_count() > 0 {
            "needs_review"
        } else {
            "ok"
        }
    }
}

#[derive(Debug, Clone)]
struct PrivacyFinding {
    severity: String,
    category: String,
    source: String,
    revision: Option<String>,
    path: Option<String>,
    line: Option<usize>,
    detail: String,
    sample: Option<String>,
    occurrences: usize,
}

#[derive(Debug, Clone)]
struct PrivacySuppressedFinding {
    category: String,
    source: String,
    detail: String,
    occurrences: usize,
}

fn build_privacy_scan_report(
    workspace: &Path,
    options: &PrivacyScanOptions,
    privacy: &PrivacyConfig,
) -> Result<PrivacyScanReport> {
    let git_present = git_stdout(workspace, &["rev-parse", "--is-inside-work-tree"])?
        .as_deref()
        .is_some_and(|value| value.trim() == "true");
    let mut findings = Vec::new();
    let mut suppressed_findings = Vec::new();
    let mut tracked_sensitive_paths = Vec::new();
    let mut revisions_scanned = 0;

    if git_present {
        scan_remote_urls(workspace, &mut findings)?;
        scan_commit_metadata(
            workspace,
            options.max_revisions,
            privacy,
            &mut findings,
            &mut suppressed_findings,
        )?;
        tracked_sensitive_paths = scan_tracked_sensitive_paths(workspace, &mut findings)?;
        scan_historical_sensitive_paths(workspace, options.max_revisions, &mut findings)?;
        revisions_scanned = scan_git_content_history(
            workspace,
            options,
            privacy,
            &mut findings,
            &mut suppressed_findings,
        )?;
    }

    let mut report = PrivacyScanReport {
        git_present,
        include_history: options.include_history,
        revision_limit: options.max_revisions,
        revisions_scanned,
        tracked_sensitive_paths,
        findings,
        suppressed_findings,
        next_actions: Vec::new(),
        report: String::new(),
    };
    report.next_actions = privacy_next_actions(&report);
    report.report = format_privacy_scan_text(workspace, &report);
    Ok(report)
}

fn scan_remote_urls(workspace: &Path, findings: &mut Vec<PrivacyFinding>) -> Result<()> {
    let Some(output) = git_stdout(workspace, &["remote", "-v"])? else {
        return Ok(());
    };
    for line in output.lines() {
        if remote_url_contains_credentials(line) {
            push_privacy_finding(
                findings,
                PrivacyFinding {
                    severity: "high".to_string(),
                    category: "remote_embedded_credentials".to_string(),
                    source: "git_remote".to_string(),
                    revision: None,
                    path: None,
                    line: None,
                    detail: "git remote URL appears to contain embedded credentials".to_string(),
                    sample: Some(sanitize_privacy_sample(line)),
                    occurrences: 1,
                },
            );
        }
    }
    Ok(())
}

fn scan_commit_metadata(
    workspace: &Path,
    max_revisions: usize,
    privacy: &PrivacyConfig,
    findings: &mut Vec<PrivacyFinding>,
    suppressed_findings: &mut Vec<PrivacySuppressedFinding>,
) -> Result<()> {
    let limit = format!("--max-count={max_revisions}");
    let Some(output) = git_stdout(
        workspace,
        &[
            "log",
            "--all",
            &limit,
            "--format=%H%x09%an%x09%ae%x09%cn%x09%ce%x09%s",
        ],
    )?
    else {
        return Ok(());
    };
    for row in output.lines() {
        let parts = row.split('\t').collect::<Vec<_>>();
        if parts.len() < 6 {
            continue;
        }
        let revision = short_revision(parts[0]);
        for email in [parts[2], parts[4]] {
            if privacy_email_is_placeholder(email) {
                continue;
            }
            if privacy_commit_email_is_allowed(email, privacy) {
                push_privacy_suppressed_finding(
                    suppressed_findings,
                    PrivacySuppressedFinding {
                        category: "commit_email".to_string(),
                        source: "git_metadata".to_string(),
                        detail: format!("allowed commit email {}", redact_email(email)),
                        occurrences: 1,
                    },
                );
                continue;
            }
            push_privacy_finding(
                findings,
                PrivacyFinding {
                    severity: "medium".to_string(),
                    category: "commit_email".to_string(),
                    source: "git_metadata".to_string(),
                    revision: Some(revision.clone()),
                    path: None,
                    line: None,
                    detail: format!("commit metadata exposes {}", redact_email(email)),
                    sample: Some(sanitize_privacy_sample(row)),
                    occurrences: 1,
                },
            );
        }
        scan_blocked_terms_in_text(
            findings,
            suppressed_findings,
            privacy,
            PrivacyScanLocation {
                source: "git_metadata",
                revision: Some(&revision),
                path: None,
                line: None,
            },
            row,
        );
    }
    Ok(())
}

fn scan_tracked_sensitive_paths(
    workspace: &Path,
    findings: &mut Vec<PrivacyFinding>,
) -> Result<Vec<String>> {
    let Some(output) = git_stdout(workspace, &["ls-files"])? else {
        return Ok(Vec::new());
    };
    let mut sensitive = Vec::new();
    for path in output
        .lines()
        .filter(|line| privacy_path_looks_sensitive(line))
    {
        let path = path.to_string();
        sensitive.push(path.clone());
        push_privacy_finding(
            findings,
            PrivacyFinding {
                severity: "high".to_string(),
                category: "tracked_sensitive_path".to_string(),
                source: "git_index".to_string(),
                revision: None,
                path: Some(path.clone()),
                line: None,
                detail: format!("tracked sensitive-looking path `{path}`"),
                sample: None,
                occurrences: 1,
            },
        );
    }
    sensitive.sort();
    sensitive.dedup();
    Ok(sensitive)
}

fn scan_historical_sensitive_paths(
    workspace: &Path,
    max_revisions: usize,
    findings: &mut Vec<PrivacyFinding>,
) -> Result<()> {
    let limit = format!("--max-count={max_revisions}");
    let Some(output) = git_stdout(
        workspace,
        &["log", "--all", &limit, "--name-only", "--pretty=format:"],
    )?
    else {
        return Ok(());
    };
    for path in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| privacy_path_looks_sensitive(line))
    {
        push_privacy_finding(
            findings,
            PrivacyFinding {
                severity: "high".to_string(),
                category: "historical_sensitive_path".to_string(),
                source: "git_history_paths".to_string(),
                revision: None,
                path: Some(path.to_string()),
                line: None,
                detail: format!("git history contains sensitive-looking path `{path}`"),
                sample: None,
                occurrences: 1,
            },
        );
    }
    Ok(())
}

fn scan_git_content_history(
    workspace: &Path,
    options: &PrivacyScanOptions,
    privacy: &PrivacyConfig,
    findings: &mut Vec<PrivacyFinding>,
    suppressed_findings: &mut Vec<PrivacySuppressedFinding>,
) -> Result<usize> {
    let revisions = if options.include_history {
        let limit = format!("--max-count={}", options.max_revisions);
        git_stdout(workspace, &["rev-list", "--all", &limit])?
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        git_stdout(workspace, &["rev-parse", "HEAD"])?
            .unwrap_or_default()
            .lines()
            .take(1)
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    for revision in &revisions {
        scan_git_revision_content(workspace, revision, privacy, findings, suppressed_findings)?;
    }
    Ok(revisions.len())
}

fn scan_git_revision_content(
    workspace: &Path,
    revision: &str,
    privacy: &PrivacyConfig,
    findings: &mut Vec<PrivacyFinding>,
    suppressed_findings: &mut Vec<PrivacySuppressedFinding>,
) -> Result<()> {
    let Some(files) = git_stdout(workspace, &["ls-tree", "-r", "--name-only", revision])? else {
        return Ok(());
    };
    let short_revision = short_revision(revision);
    for path in files.lines().filter(|line| !line.trim().is_empty()) {
        let spec = format!("{revision}:{path}");
        let Some(bytes) = git_stdout_bytes(workspace, &["show", &spec])? else {
            continue;
        };
        if bytes.len() > 2_000_000 || bytes.iter().take(4096).any(|byte| *byte == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (line_index, line) in text.lines().enumerate() {
            scan_privacy_line(
                findings,
                suppressed_findings,
                privacy,
                &short_revision,
                path,
                line_index + 1,
                line,
            );
        }
    }
    Ok(())
}

fn scan_privacy_line(
    findings: &mut Vec<PrivacyFinding>,
    suppressed_findings: &mut Vec<PrivacySuppressedFinding>,
    privacy: &PrivacyConfig,
    revision: &str,
    path: &str,
    line_number: usize,
    line: &str,
) {
    scan_blocked_terms_in_text(
        findings,
        suppressed_findings,
        privacy,
        PrivacyScanLocation {
            source: "git_history_content",
            revision: Some(revision),
            path: Some(path),
            line: Some(line_number),
        },
        line,
    );

    if line.contains(USER_HOME_PREFIX) {
        let detail = first_redacted_user_path(line).unwrap_or_else(redacted_user_home);
        if privacy_user_path_is_allowed(&detail, privacy) {
            push_privacy_suppressed_finding(
                suppressed_findings,
                PrivacySuppressedFinding {
                    category: "absolute_user_path".to_string(),
                    source: "git_history_content".to_string(),
                    detail: format!("allowed local user path {detail}"),
                    occurrences: 1,
                },
            );
        } else {
            push_privacy_finding(
                findings,
                PrivacyFinding {
                    severity: "medium".to_string(),
                    category: "absolute_user_path".to_string(),
                    source: "git_history_content".to_string(),
                    revision: Some(revision.to_string()),
                    path: Some(path.to_string()),
                    line: Some(line_number),
                    detail,
                    sample: Some(sanitize_privacy_sample(line)),
                    occurrences: 1,
                },
            );
        }
    }

    if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
        let fixture = privacy_line_is_detector_literal(path, line);
        push_privacy_finding(
            findings,
            PrivacyFinding {
                severity: if fixture { "low" } else { "high" }.to_string(),
                category: if fixture {
                    "private_key_detector_literal".to_string()
                } else {
                    "private_key_block".to_string()
                },
                source: "git_history_content".to_string(),
                revision: Some(revision.to_string()),
                path: Some(path.to_string()),
                line: Some(line_number),
                detail: if fixture {
                    "private-key marker appears to be scanner/test source text".to_string()
                } else {
                    "private-key block marker appears in history content".to_string()
                },
                sample: Some(sanitize_privacy_sample(line)),
                occurrences: 1,
            },
        );
    }

    for token in privacy_token_candidates(line) {
        if let Some((category, severity, detail)) = classify_privacy_token(path, line, &token) {
            push_privacy_finding(
                findings,
                PrivacyFinding {
                    severity,
                    category,
                    source: "git_history_content".to_string(),
                    revision: Some(revision.to_string()),
                    path: Some(path.to_string()),
                    line: Some(line_number),
                    detail,
                    sample: Some(sanitize_privacy_sample(line)),
                    occurrences: 1,
                },
            );
        }
    }

    for email in privacy_email_candidates(line) {
        if privacy_email_is_placeholder(&email) {
            continue;
        }
        if privacy_email_is_allowed(&email, privacy) {
            push_privacy_suppressed_finding(
                suppressed_findings,
                PrivacySuppressedFinding {
                    category: "content_email".to_string(),
                    source: "git_history_content".to_string(),
                    detail: format!("allowed content email {}", redact_email(&email)),
                    occurrences: 1,
                },
            );
            continue;
        }
        push_privacy_finding(
            findings,
            PrivacyFinding {
                severity: "medium".to_string(),
                category: "content_email".to_string(),
                source: "git_history_content".to_string(),
                revision: Some(revision.to_string()),
                path: Some(path.to_string()),
                line: Some(line_number),
                detail: format!("file content exposes {}", redact_email(&email)),
                sample: Some(sanitize_privacy_sample(line)),
                occurrences: 1,
            },
        );
    }
}

#[derive(Debug, Clone, Copy)]
struct PrivacyScanLocation<'a> {
    source: &'a str,
    revision: Option<&'a str>,
    path: Option<&'a str>,
    line: Option<usize>,
}

fn scan_blocked_terms_in_text(
    findings: &mut Vec<PrivacyFinding>,
    suppressed_findings: &mut Vec<PrivacySuppressedFinding>,
    privacy: &PrivacyConfig,
    location: PrivacyScanLocation<'_>,
    text: &str,
) {
    if privacy
        .blocked_terms
        .iter()
        .all(|term| term.trim().is_empty())
    {
        return;
    }
    if location
        .path
        .is_some_and(|path| path.replace('\\', "/") == ".deepcli/config.json")
    {
        return;
    }
    for term in privacy_blocked_terms(privacy) {
        let occurrences = blocked_term_occurrences(text, &term);
        if occurrences == 0 {
            continue;
        }
        if privacy_term_is_allowed(&term, privacy) {
            push_privacy_suppressed_finding(
                suppressed_findings,
                PrivacySuppressedFinding {
                    category: "blocked_term".to_string(),
                    source: location.source.to_string(),
                    detail: "allowed configured blocked term match <blocked-term>".to_string(),
                    occurrences,
                },
            );
            continue;
        }
        push_privacy_finding(
            findings,
            PrivacyFinding {
                severity: "medium".to_string(),
                category: "blocked_term".to_string(),
                source: location.source.to_string(),
                revision: location.revision.map(ToString::to_string),
                path: location.path.map(ToString::to_string),
                line: location.line,
                detail: "configured blocked term appears in repository history".to_string(),
                sample: Some("<blocked-term>".to_string()),
                occurrences,
            },
        );
    }
}

fn privacy_blocked_terms(privacy: &PrivacyConfig) -> Vec<String> {
    let mut terms = privacy
        .blocked_terms
        .iter()
        .map(|term| term.trim())
        .filter(|term| !term.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    terms.sort_by_key(|term| std::cmp::Reverse(term.chars().count()));
    terms.dedup_by(|a, b| normalize_privacy_term(a) == normalize_privacy_term(b));
    terms
}

fn blocked_term_occurrences(text: &str, term: &str) -> usize {
    let normalized_text = normalize_privacy_term(text);
    let normalized_term = normalize_privacy_term(term);
    if normalized_term.is_empty() {
        return 0;
    }
    normalized_text.matches(&normalized_term).count()
}

fn privacy_term_is_allowed(term: &str, privacy: &PrivacyConfig) -> bool {
    let term = normalize_privacy_term(term);
    !term.is_empty()
        && privacy
            .allowed_terms
            .iter()
            .map(|allowed| normalize_privacy_term(allowed))
            .any(|allowed| allowed == term)
}

fn normalize_privacy_term(value: &str) -> String {
    value.trim().to_lowercase()
}

fn classify_privacy_token(path: &str, line: &str, token: &str) -> Option<(String, String, String)> {
    let lower = token.to_ascii_lowercase();
    if token.starts_with("github_pat_") && token.len() >= 30 {
        return Some((
            "github_token".to_string(),
            "high".to_string(),
            "GitHub token-shaped value appears in history content".to_string(),
        ));
    }
    if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|prefix| token.starts_with(prefix))
        && token.len() >= 20
    {
        return Some((
            "github_token".to_string(),
            "high".to_string(),
            "GitHub token-shaped value appears in history content".to_string(),
        ));
    }
    if token.starts_with("AKIA")
        && token.len() == 20
        && token
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
    {
        return Some((
            "aws_access_key".to_string(),
            "high".to_string(),
            "AWS access-key-shaped value appears in history content".to_string(),
        ));
    }
    if token.starts_with("xox") && token.len() >= 20 {
        return Some((
            "slack_token".to_string(),
            "high".to_string(),
            "Slack token-shaped value appears in history content".to_string(),
        ));
    }
    if token.starts_with("sk-") && token.len() >= 20 {
        let fixture = privacy_token_is_fixture_like(path, line, &lower);
        return Some((
            if fixture {
                "secret_shaped_fixture".to_string()
            } else {
                "openai_deepseek_style_key".to_string()
            },
            if fixture { "low" } else { "high" }.to_string(),
            if fixture {
                "sk-shaped value appears to be a test fixture or detector sample".to_string()
            } else {
                "OpenAI/DeepSeek-style key-shaped value appears in history content".to_string()
            },
        ));
    }
    None
}

fn privacy_token_is_fixture_like(path: &str, line: &str, lower_token: &str) -> bool {
    let lower_line = line.to_ascii_lowercase();
    let lower_path = path.to_ascii_lowercase();
    lower_path.contains("test")
        || lower_path.ends_with("_test.rs")
        || lower_path.contains("fixture")
        || lower_line.contains("assert")
        || lower_line.contains("redact")
        || [
            "test", "fixture", "dummy", "fake", "example", "replace", "secret",
        ]
        .iter()
        .any(|marker| lower_token.contains(marker))
}

fn privacy_line_is_detector_literal(path: &str, line: &str) -> bool {
    let lower_path = path.to_ascii_lowercase();
    let lower_line = line.to_ascii_lowercase();
    lower_path.ends_with("privacy.rs")
        || lower_path.contains("test")
        || lower_line.contains("secret_markers")
        || lower_line.contains("redact")
        || line.contains('"')
        || line.contains('\'')
}

fn privacy_token_candidates(line: &str) -> Vec<String> {
    line.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn privacy_email_candidates(line: &str) -> Vec<String> {
    line.split(|ch: char| {
        !(ch.is_ascii_alphanumeric() || matches!(ch, '@' | '.' | '_' | '%' | '+' | '-'))
    })
    .filter(|token| {
        let Some((local, domain)) = token.split_once('@') else {
            return false;
        };
        !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
    })
    .map(|token| token.trim_matches('.').to_string())
    .collect()
}

fn privacy_email_is_placeholder(email: &str) -> bool {
    let lower = email.to_ascii_lowercase();
    lower.ends_with("@local")
        || lower.ends_with(".local")
        || lower.ends_with("@example.com")
        || lower.ends_with("@example.test")
        || lower.ends_with(".example")
        || lower.contains("@example.")
}

fn privacy_commit_email_is_allowed(email: &str, privacy: &PrivacyConfig) -> bool {
    if privacy_email_is_allowed(email, privacy) {
        return true;
    }
    let email = email.trim().to_ascii_lowercase();
    if email.is_empty() {
        return false;
    }
    if privacy
        .allowed_commit_emails
        .iter()
        .map(|allowed| allowed.trim().to_ascii_lowercase())
        .any(|allowed| allowed == email)
    {
        return true;
    }
    let Some((_local, domain)) = email.split_once('@') else {
        return false;
    };
    privacy
        .allowed_commit_domains
        .iter()
        .map(|allowed| allowed.trim().trim_start_matches('@').to_ascii_lowercase())
        .any(|allowed| !allowed.is_empty() && allowed == domain)
}

fn privacy_email_is_allowed(email: &str, privacy: &PrivacyConfig) -> bool {
    let email = email.trim().to_ascii_lowercase();
    if email.is_empty() {
        return false;
    }
    if privacy
        .allowed_emails
        .iter()
        .map(|allowed| allowed.trim().to_ascii_lowercase())
        .any(|allowed| allowed == email)
    {
        return true;
    }
    let Some((_local, domain)) = email.split_once('@') else {
        return false;
    };
    privacy
        .allowed_email_domains
        .iter()
        .map(|allowed| allowed.trim().trim_start_matches('@').to_ascii_lowercase())
        .any(|allowed| !allowed.is_empty() && allowed == domain)
}

fn privacy_user_path_is_allowed(redacted_path: &str, privacy: &PrivacyConfig) -> bool {
    let path = normalize_privacy_user_path(redacted_path);
    if path.is_empty() {
        return false;
    }
    privacy.allowed_user_paths.iter().any(|allowed| {
        let allowed = normalize_privacy_user_path(allowed);
        !allowed.is_empty() && (path == allowed || path.starts_with(&format!("{allowed}/")))
    })
}

fn normalize_privacy_user_path(path: &str) -> String {
    path.trim()
        .trim_end_matches('/')
        .replace('\\', "/")
        .to_string()
}

fn redact_email(email: &str) -> String {
    let Some((local, domain)) = email.split_once('@') else {
        return "<email:redacted>".to_string();
    };
    let prefix = local.chars().next().unwrap_or('*');
    format!("{prefix}***@{domain}")
}

fn redact_emails(value: &str) -> String {
    let mut output = value.to_string();
    for email in privacy_email_candidates(value) {
        output = output.replace(&email, &redact_email(&email));
    }
    output
}

fn privacy_path_looks_sensitive(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let segments = normalized.split('/').collect::<Vec<_>>();
    let file_name = segments.last().copied().unwrap_or_default();
    if normalized == ".env" || file_name == ".env" || file_name.starts_with(".env.") {
        return true;
    }
    if matches!(
        file_name,
        "id_rsa" | "id_ed25519" | "credentials.json" | "authorization.json"
    ) {
        return true;
    }
    if file_name.ends_with("-credentials.json")
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || file_name.ends_with(".p12")
        || file_name.ends_with(".pfx")
    {
        return true;
    }
    segments
        .windows(2)
        .any(|pair| pair[0] == ".deepcli" && matches!(pair[1], "credentials" | "sessions" | "logs"))
        || segments
            .iter()
            .any(|segment| matches!(*segment, "credentials" | "secrets" | "secret"))
}

fn remote_url_contains_credentials(line: &str) -> bool {
    let Some(url) = line.split_whitespace().nth(1) else {
        return false;
    };
    let Some((_, after_scheme)) = url.split_once("://") else {
        return false;
    };
    let authority = after_scheme.split('/').next().unwrap_or_default();
    let Some((userinfo, _host)) = authority.rsplit_once('@') else {
        return false;
    };
    !userinfo.is_empty()
}

fn first_redacted_user_path(line: &str) -> Option<String> {
    let start = line.find(USER_HOME_PREFIX)?;
    let rest = &line[start..];
    let already_redacted = rest.starts_with(&redacted_user_home());
    let end = rest
        .find(|ch: char| {
            ch.is_whitespace()
                || matches!(ch, '"' | '\'' | '`' | ')' | '(' | ',' | ';' | '}')
                || (!already_redacted && matches!(ch, '<' | '>'))
        })
        .unwrap_or(rest.len());
    Some(redact_user_paths(&rest[..end]))
}

fn redact_user_paths(value: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    while let Some(index) = rest.find(USER_HOME_PREFIX) {
        output.push_str(&rest[..index]);
        rest = &rest[index + USER_HOME_PREFIX.len()..];
        let user_end = rest.find('/').unwrap_or(rest.len());
        output.push_str(&redacted_user_home());
        output.push_str(&rest[user_end..]);
        rest = "";
    }
    output.push_str(rest);
    output
}

fn sanitize_privacy_sample(line: &str) -> String {
    let redacted = redact_sensitive_text(&redact_emails(&redact_user_paths(line)));
    truncate_display(&redacted, 240)
}

fn push_privacy_finding(findings: &mut Vec<PrivacyFinding>, mut finding: PrivacyFinding) {
    finding.occurrences = finding.occurrences.max(1);
    if let Some(existing) = findings
        .iter_mut()
        .find(|existing| privacy_findings_equivalent(existing, &finding))
    {
        existing.occurrences += finding.occurrences;
        return;
    }
    if findings.len() < 250 {
        findings.push(finding);
    }
}

fn push_privacy_suppressed_finding(
    suppressed_findings: &mut Vec<PrivacySuppressedFinding>,
    mut finding: PrivacySuppressedFinding,
) {
    finding.occurrences = finding.occurrences.max(1);
    if let Some(existing) = suppressed_findings
        .iter_mut()
        .find(|existing| privacy_suppressed_findings_equivalent(existing, &finding))
    {
        existing.occurrences += finding.occurrences;
        return;
    }
    if suppressed_findings.len() < 100 {
        suppressed_findings.push(finding);
    }
}

fn privacy_suppressed_findings_equivalent(
    existing: &PrivacySuppressedFinding,
    finding: &PrivacySuppressedFinding,
) -> bool {
    existing.category == finding.category
        && existing.source == finding.source
        && existing.detail == finding.detail
}

fn privacy_findings_equivalent(existing: &PrivacyFinding, finding: &PrivacyFinding) -> bool {
    if existing.severity != finding.severity
        || existing.category != finding.category
        || existing.source != finding.source
        || existing.path != finding.path
        || existing.detail != finding.detail
    {
        return false;
    }

    match finding.category.as_str() {
        "absolute_user_path"
        | "blocked_term"
        | "commit_email"
        | "historical_sensitive_path"
        | "private_key_detector_literal"
        | "secret_shaped_fixture" => true,
        _ => {
            existing.revision == finding.revision
                && existing.line == finding.line
                && existing.sample == finding.sample
        }
    }
}

fn short_revision(revision: &str) -> String {
    revision.chars().take(8).collect()
}
fn privacy_next_actions(report: &PrivacyScanReport) -> Vec<String> {
    if !report.git_present {
        return vec!["run `/privacy` inside a Git repository".to_string()];
    }
    let mut actions = Vec::new();
    if report.high_count() > 0 {
        actions.push(
            "rotate any real exposed credentials, then remove them from history before sharing"
                .to_string(),
        );
        actions.push(
            "rewrite history only after coordinating force-push impact with collaborators"
                .to_string(),
        );
    }
    if report.medium_count() > 0 {
        actions.push("review metadata findings before making the repository public".to_string());
    }
    if report
        .findings
        .iter()
        .any(|finding| finding.category == "blocked_term")
    {
        actions.push(
            "review configured blocked-term matches, then rename current content or plan a history rewrite"
                .to_string(),
        );
    }
    if report.low_count() > 0 {
        actions.push(
            "consider renaming test fixtures that look like real secrets to reduce scanner noise"
                .to_string(),
        );
    }
    if actions.is_empty() {
        actions.push("no privacy findings detected by this local scan".to_string());
    }
    actions.push("export a machine-readable report with `/privacy --json --output .deepcli/exports/privacy.json`".to_string());
    dedup_preserve_order(actions)
}

fn format_privacy_scan_text(workspace: &Path, report: &PrivacyScanReport) -> String {
    let mut lines = vec![
        "deepcli privacy scan".to_string(),
        format!("workspace: {}", workspace.display()),
        format!(
            "git: {}",
            if report.git_present {
                "present"
            } else {
                "missing"
            }
        ),
        format!("status: {}", report.status()),
        format!(
            "history: {} revision_limit={} revisions_scanned={}",
            if report.include_history {
                "enabled"
            } else {
                "current-only"
            },
            report.revision_limit,
            report.revisions_scanned
        ),
        format!(
            "findings: high={} medium={} low={} occurrences={}",
            report.high_count(),
            report.medium_count(),
            report.low_count(),
            report.occurrence_count()
        ),
    ];
    if !report.suppressed_findings.is_empty() {
        lines.push(format!(
            "suppressed: findings={} occurrences={}",
            report.suppressed_findings.len(),
            report.suppressed_occurrence_count()
        ));
    }

    if !report.tracked_sensitive_paths.is_empty() {
        lines.push("tracked sensitive paths:".to_string());
        for path in report.tracked_sensitive_paths.iter().take(20) {
            lines.push(format!("  - {path}"));
        }
        if report.tracked_sensitive_paths.len() > 20 {
            lines.push(format!(
                "  ... {} more",
                report.tracked_sensitive_paths.len() - 20
            ));
        }
    }

    if report.findings.is_empty() {
        lines.push("privacy findings: none".to_string());
    } else {
        lines.push("privacy findings:".to_string());
        for finding in report.findings.iter().take(40) {
            lines.push(format_privacy_finding_line(finding));
        }
        if report.findings.len() > 40 {
            lines.push(format!("  ... {} more", report.findings.len() - 40));
        }
    }
    if !report.suppressed_findings.is_empty() {
        lines.push("suppressed findings:".to_string());
        for finding in report.suppressed_findings.iter().take(20) {
            lines.push(format_privacy_suppressed_finding_line(finding));
        }
        if report.suppressed_findings.len() > 20 {
            lines.push(format!(
                "  ... {} more",
                report.suppressed_findings.len() - 20
            ));
        }
    }

    lines.push("next actions:".to_string());
    lines.extend(
        report
            .next_actions
            .iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn format_privacy_suppressed_finding_line(finding: &PrivacySuppressedFinding) -> String {
    let occurrences = if finding.occurrences > 1 {
        format!(" occurrences={}", finding.occurrences)
    } else {
        String::new()
    };
    format!(
        "  - {} {}: {}{}",
        finding.category, finding.source, finding.detail, occurrences
    )
}

fn format_privacy_finding_line(finding: &PrivacyFinding) -> String {
    let location = match (&finding.revision, &finding.path, finding.line) {
        (Some(rev), Some(path), Some(line)) => format!("{rev} {path}:{line}"),
        (Some(rev), Some(path), None) => format!("{rev} {path}"),
        (Some(rev), None, _) => rev.clone(),
        (None, Some(path), Some(line)) => format!("{path}:{line}"),
        (None, Some(path), None) => path.clone(),
        (None, None, _) => finding.source.clone(),
    };
    let sample = finding
        .sample
        .as_ref()
        .map(|sample| format!(" sample={sample}"))
        .unwrap_or_default();
    let occurrences = if finding.occurrences > 1 {
        format!(" occurrences={}", finding.occurrences)
    } else {
        String::new()
    };
    format!(
        "  - [{}] {} {}: {}{}{}",
        finding.severity, finding.category, location, finding.detail, occurrences, sample
    )
}

fn format_privacy_scan_json(workspace: &Path, report: &PrivacyScanReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.privacy.scan.v1",
        "status": report.status(),
        "workspace": workspace.display().to_string(),
        "git": {
            "present": report.git_present,
            "includeHistory": report.include_history,
            "revisionLimit": report.revision_limit,
            "revisionsScanned": report.revisions_scanned,
        },
        "counts": {
            "high": report.high_count(),
            "medium": report.medium_count(),
            "low": report.low_count(),
            "total": report.findings.len(),
            "occurrences": report.occurrence_count(),
            "actionable": report.actionable_finding_count(),
            "suppressed": report.suppressed_findings.len(),
            "suppressedOccurrences": report.suppressed_occurrence_count(),
        },
        "trackedSensitivePaths": report.tracked_sensitive_paths,
        "findings": report.findings.iter().map(privacy_finding_json).collect::<Vec<_>>(),
        "suppressedFindings": report
            .suppressed_findings
            .iter()
            .map(privacy_suppressed_finding_json)
            .collect::<Vec<_>>(),
        "nextActions": report.next_actions,
        "report": report.report,
    }))?)
}

fn privacy_finding_json(finding: &PrivacyFinding) -> Value {
    json!({
        "severity": finding.severity,
        "category": finding.category,
        "source": finding.source,
        "revision": finding.revision,
        "path": finding.path,
        "line": finding.line,
        "detail": finding.detail,
        "sample": finding.sample,
        "occurrences": finding.occurrences,
    })
}

fn privacy_suppressed_finding_json(finding: &PrivacySuppressedFinding) -> Value {
    json!({
        "category": finding.category,
        "source": finding.source,
        "detail": finding.detail,
        "occurrences": finding.occurrences,
    })
}

pub(super) const USER_HOME_PREFIX: &str = concat!("/", "Users", "/");

pub(super) fn redacted_user_home() -> String {
    format!("{USER_HOME_PREFIX}<user>")
}
