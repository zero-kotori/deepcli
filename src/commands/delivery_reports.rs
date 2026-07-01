use super::*;
use crate::schema_ids;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

pub(crate) enum VerificationDiffSource {
    Git {
        diff: String,
    },
    Session(SessionDiffSource),
    None {
        git_available: bool,
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VerificationTestRun {
    NotRequested,
    Completed {
        command: String,
        passed: bool,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VerificationEnvironmentCheck {
    Completed {
        target: String,
        report: EnvironmentReport,
        text: String,
    },
    Error {
        target: String,
        error: String,
    },
}

pub(crate) fn weak_test_command_reason(command: &str) -> Option<&'static str> {
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Some("empty command does not exercise project behavior");
    }
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if matches!(compact.as_str(), ":" | "true" | "exit 0") {
        return Some("no-op command does not exercise project behavior");
    }
    if compact.starts_with("printf ") || compact.starts_with("echo ") {
        return Some("prints fixed output instead of exercising project behavior");
    }
    if compact.contains(" printf ok")
        || compact.contains(" echo ok")
        || compact.contains(" printf 'ok'")
        || compact.contains(" echo 'ok'")
        || compact.contains(" printf \"ok\"")
        || compact.contains(" echo \"ok\"")
    {
        return Some("prints fixed output instead of exercising project behavior");
    }
    None
}

fn weak_test_command_blocker(command: &str, context: &str) -> Option<String> {
    weak_test_command_reason(command).map(|reason| {
        format!(
            "{context} is weak test evidence: command=`{}` ({reason})",
            truncate_display(command, 120)
        )
    })
}

fn verification_test_run_weak_blocker(test_run: &VerificationTestRun) -> Option<String> {
    match test_run {
        VerificationTestRun::Completed {
            command,
            passed: true,
            ..
        } => weak_test_command_blocker(command, "requested verification test"),
        _ => None,
    }
}

fn has_strong_passing_test(tests: &[TestRunRecord]) -> bool {
    tests
        .iter()
        .any(|test| test.passed && weak_test_command_reason(&test.command).is_none())
}

fn has_strong_passing_verification_test(test_run: &VerificationTestRun) -> bool {
    matches!(
        test_run,
        VerificationTestRun::Completed {
            command,
            passed: true,
            ..
        } if weak_test_command_reason(command).is_none()
    )
}

fn latest_strong_passing_test_at(tests: &[TestRunRecord]) -> Option<DateTime<Utc>> {
    tests
        .iter()
        .filter(|test| test.passed && weak_test_command_reason(&test.command).is_none())
        .map(|test| test.created_at)
        .max()
}

fn latest_session_diff_modified_at(records: &[SessionDiffRecord]) -> Option<DateTime<Utc>> {
    records.iter().map(|record| record.modified_at).max()
}

fn latest_workspace_diff_mtime(workspace: &Path, diff: &str) -> Option<DateTime<Utc>> {
    let mut latest: Option<DateTime<Utc>> = None;
    for summary in diff_file_summaries(diff) {
        let Some(path) = normalize_diff_path_for_filter(&summary.path) else {
            continue;
        };
        let target = workspace.join(path);
        let Ok(metadata) = fs::metadata(target) else {
            continue;
        };
        let modified_at =
            DateTime::<Utc>::from(metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        latest = Some(latest.map_or(modified_at, |current| current.max(modified_at)));
    }
    latest
}

fn stale_strong_test_evidence_blocker(
    tests: &[TestRunRecord],
    latest_change_at: Option<DateTime<Utc>>,
    path_filters: &[String],
) -> Option<String> {
    let latest_test_at = latest_strong_passing_test_at(tests)?;
    let latest_change_at = latest_change_at?;
    if latest_test_at >= latest_change_at {
        return None;
    }
    let scope = if path_filters.is_empty() {
        "diff".to_string()
    } else {
        "scoped diff".to_string()
    };
    Some(format!(
        "no strong passing test evidence after latest {scope} change"
    ))
}

pub(crate) struct VerificationStatusSource<'a> {
    pub(crate) available: bool,
    pub(crate) text: &'a str,
    pub(crate) detail: Option<String>,
}

pub(crate) struct VerificationReportInput<'a> {
    pub(crate) workspace: &'a Path,
    pub(crate) session: Option<&'a Session>,
    pub(crate) session_note: Option<String>,
    pub(crate) status: VerificationStatusSource<'a>,
    pub(crate) path_filters: &'a [String],
    pub(crate) diff_source: VerificationDiffSource,
    pub(crate) test_limit: usize,
    pub(crate) test_run: VerificationTestRun,
    pub(crate) environment_checks: &'a [VerificationEnvironmentCheck],
}

pub(crate) struct HandoffReportInput<'a> {
    pub(crate) workspace: &'a Path,
    pub(crate) session: Option<&'a Session>,
    pub(crate) session_note: Option<String>,
    pub(crate) status: VerificationStatusSource<'a>,
    pub(crate) path_filters: &'a [String],
    pub(crate) diff_source: VerificationDiffSource,
    pub(crate) limit: usize,
    pub(crate) environment_checks: &'a [VerificationEnvironmentCheck],
}

pub(crate) fn format_verification_report(input: VerificationReportInput<'_>) -> Result<String> {
    let mut lines = vec![
        "verification report".to_string(),
        format!("workspace: {}", input.workspace.display()),
    ];

    if let Some(session) = input.session {
        let title = session.metadata.title.as_deref().unwrap_or("<untitled>");
        let model = session.metadata.model.as_deref().unwrap_or("<default>");
        let mut session_line = format!(
            "session: id={} full={} title={} state={:?} provider={} model={}",
            short_id(&session.id()),
            session.id(),
            title,
            session.metadata.state,
            session.metadata.provider,
            model
        );
        if let Some(note) = input.session_note {
            session_line.push_str(&format!(" ({note})"));
        }
        lines.push(session_line);
    } else {
        lines.push("session: none found; report uses workspace state only".to_string());
    }

    lines.push(format_git_status_summary(
        input.status.available,
        input.status.text,
        input.status.detail,
    ));
    if !input.path_filters.is_empty() {
        lines.push(format!(
            "scope: paths={}",
            format_verify_path_filters(input.path_filters)
        ));
    }

    let mut blockers = Vec::new();
    let workspace_only_strong_test =
        input.session.is_none() && has_strong_passing_verification_test(&input.test_run);
    if let Some(session) = input.session {
        blockers.extend(session_verification_blockers(session)?);
    } else if workspace_only_strong_test {
        lines.push(
            "session evidence: none found; using workspace-only evidence from this report"
                .to_string(),
        );
    } else {
        blockers.push("no session context found; run deepcli on the task before relying on session-level evidence".to_string());
    }

    let (diff_label, review_input, latest_change_at) = match input.diff_source {
        VerificationDiffSource::Git { diff } => {
            let (added, removed) = diff_line_counts(&diff);
            let latest_change_at = latest_workspace_diff_mtime(input.workspace, &diff);
            let label = if input.path_filters.is_empty() {
                format!("git diff with +{added} -{removed} changed line(s)")
            } else {
                format!(
                    "git diff scoped to {} with +{} -{} changed line(s)",
                    format_verify_path_filters(input.path_filters),
                    added,
                    removed
                )
            };
            (label, Some(diff), latest_change_at)
        }
        VerificationDiffSource::Session(source) => {
            let latest_change_at = latest_session_diff_modified_at(&source.records);
            let review_input = session_diff_review_input(&source.records);
            let (added, removed) = diff_line_counts(&review_input);
            let scope = if input.path_filters.is_empty() {
                String::new()
            } else {
                format!(
                    " scoped to {}",
                    format_verify_path_filters(input.path_filters)
                )
            };
            let mut label = format!(
                "session diff fallback from {}{} with {} record(s), +{} -{} changed line(s)",
                source.session.id(),
                scope,
                source.records.len(),
                added,
                removed
            );
            if let Some(note) = source.note {
                label.push_str(&format!(" ({note})"));
            }
            (label, Some(review_input), latest_change_at)
        }
        VerificationDiffSource::None {
            git_available,
            detail,
        } => {
            let mut label = if git_available {
                "none: no local Git diff and no session diff records found".to_string()
            } else {
                "none: Git diff unavailable and no session diff records found".to_string()
            };
            if let Some(detail) = detail {
                label.push_str(&format!(" ({detail})"));
            }
            (label, None, None)
        }
    };
    lines.push(format!("diff source: {diff_label}"));

    lines.push("review:".to_string());
    let review = review_worktree(
        input.status.text,
        review_input.as_deref().unwrap_or_default(),
    );
    let review_risk = review_risk_summary_from_report(&review);
    if review_risk.high_findings > 0 {
        blockers.push(format!(
            "auto-reviewer reported {} high-risk finding type(s)",
            review_risk.high_findings
        ));
    }
    if review_risk.medium_findings > 0 {
        lines.push(format!(
            "review warnings: auto-reviewer reported {} medium-risk finding type(s); inspect before handoff",
            review_risk.medium_findings
        ));
    }
    lines.push(indent_text(&truncate_display(&review, 1_500), "  "));

    lines.push("tests:".to_string());
    let test_run_succeeded = matches!(
        &input.test_run,
        VerificationTestRun::Completed { passed: true, .. }
    );
    let test_run_failed = matches!(
        &input.test_run,
        VerificationTestRun::Completed { passed: false, .. } | VerificationTestRun::Error(_)
    );
    append_verification_test_run(&mut lines, &input.test_run);
    if let Some(blocker) = verification_test_run_weak_blocker(&input.test_run) {
        lines.push(format!("  evidence warning: {blocker}"));
        blockers.push(blocker);
    }
    if let Some(session) = input.session {
        let tests = session.load_recent_test_runs(input.test_limit)?;
        let failed = tests.iter().filter(|item| !item.passed).count();
        let passed = tests.iter().filter(|item| item.passed).count();
        lines.push(format!(
            "  latest {} recorded test run(s): passed={} failed={}",
            tests.len(),
            passed,
            failed
        ));
        if let Some(latest) = tests.last() {
            lines.push(format!(
                "  latest: [{}] exit={:?} command={}",
                if latest.passed { "passed" } else { "failed" },
                latest.exit_code,
                latest.command
            ));
            if let Some(reason) = weak_test_command_reason(&latest.command) {
                lines.push(format!(
                    "  evidence warning: latest recorded test command is weak evidence ({reason})"
                ));
            }
        }
        if tests.is_empty() && !test_run_succeeded {
            blockers.push("no test runs recorded for the selected session".to_string());
        }
        if !tests.is_empty()
            && !has_strong_passing_test(&tests)
            && !has_strong_passing_verification_test(&input.test_run)
        {
            blockers.push(
                "no strong passing test evidence recorded for the selected session".to_string(),
            );
        }
        if has_strong_passing_test(&tests) && !has_strong_passing_verification_test(&input.test_run)
        {
            if let Some(blocker) =
                stale_strong_test_evidence_blocker(&tests, latest_change_at, input.path_filters)
            {
                lines.push(format!("  evidence warning: {blocker}"));
                blockers.push(blocker);
            }
        }
        if test_run_failed {
            blockers.push("requested verification test run failed".to_string());
        }
    } else {
        lines.push("  no session selected; no recorded tests available".to_string());
        if test_run_failed {
            blockers.push("requested verification test run failed".to_string());
        }
    }

    lines.push("environment:".to_string());
    append_verification_environment(
        &mut lines,
        &mut blockers,
        input.environment_checks,
        "  not requested; use `/verify --env-check docker` or `/verify --env-check compiler` when environment readiness matters",
    );

    if blockers.is_empty() {
        lines.push("blockers: none detected from recorded session signals".to_string());
    } else {
        lines.push("blockers:".to_string());
        lines.extend(blockers.iter().map(|item| format!("- {item}")));
    }

    lines.push("next actions:".to_string());
    let needs_fresh_test_evidence = blockers
        .iter()
        .any(|item| blocker_needs_fresh_test_evidence(item));
    lines.extend(verification_next_actions(
        input.session,
        review_input.is_some(),
        blockers.is_empty(),
        !matches!(input.test_run, VerificationTestRun::NotRequested),
        needs_fresh_test_evidence,
        input.path_filters,
        input.environment_checks,
    ));
    Ok(lines.join("\n"))
}

pub(crate) fn format_handoff_report(input: HandoffReportInput<'_>) -> Result<String> {
    let mut lines = vec![
        "handoff report".to_string(),
        "summary:".to_string(),
        format!("- workspace: {}", input.workspace.display()),
    ];

    if let Some(session) = input.session {
        let title = session.metadata.title.as_deref().unwrap_or("<untitled>");
        let model = session.metadata.model.as_deref().unwrap_or("<default>");
        let mut line = format!(
            "- session: id={} full={} title={} state={:?} provider={} model={}",
            short_id(&session.id()),
            session.id(),
            title,
            session.metadata.state,
            session.metadata.provider,
            model
        );
        if let Some(note) = input.session_note {
            line.push_str(&format!(" ({note})"));
        }
        lines.push(line);
    } else {
        lines.push("- session: none found; report uses workspace state only".to_string());
    }
    lines.push(format!(
        "- git: {}",
        format_git_status_summary(
            input.status.available,
            input.status.text,
            input.status.detail
        )
        .trim_start_matches("git status: ")
    ));
    if !input.path_filters.is_empty() {
        lines.push(format!(
            "- scope: paths={}",
            format_verify_path_filters(input.path_filters)
        ));
    }

    let (diff_label, review_input, latest_change_at) =
        handoff_diff_label_and_review_input(input.workspace, input.diff_source, input.path_filters);
    lines.push(format!("- diff: {diff_label}"));

    lines.push("changed files:".to_string());
    if let Some(diff) = review_input.as_deref() {
        lines.push(indent_text(
            &format_diff_stat(diff, Some(input.limit)),
            "  ",
        ));
    } else {
        lines.push("  none".to_string());
    }

    let review = review_worktree(
        input.status.text,
        review_input.as_deref().unwrap_or_default(),
    );
    let review_risk = review_risk_summary_from_report(&review);
    lines.push("review:".to_string());
    lines.push(format!(
        "  risk: high={} medium={}",
        review_risk.high_findings, review_risk.medium_findings
    ));
    lines.push(indent_text(&truncate_display(&review, 1_000), "  "));

    let mut blockers = Vec::new();
    lines.push("tests:".to_string());
    if let Some(session) = input.session {
        blockers.extend(session_verification_blockers(session)?);
        let tests = session.load_recent_test_runs(input.limit)?;
        let passed = tests.iter().filter(|test| test.passed).count();
        let failed = tests.iter().filter(|test| !test.passed).count();
        lines.push(format!(
            "  latest {} recorded test run(s): passed={} failed={}",
            tests.len(),
            passed,
            failed
        ));
        if let Some(latest) = tests.last() {
            lines.push(format!(
                "  latest: [{}] exit={:?} command={}",
                if latest.passed { "passed" } else { "failed" },
                latest.exit_code,
                latest.command
            ));
            if let Some(reason) = weak_test_command_reason(&latest.command) {
                lines.push(format!(
                    "  evidence warning: latest recorded test command is weak evidence ({reason})"
                ));
            }
        }
        if tests.is_empty() {
            blockers.push("no test runs recorded for the selected session".to_string());
        } else if !has_strong_passing_test(&tests) {
            blockers.push(
                "no strong passing test evidence recorded for the selected session".to_string(),
            );
        } else if let Some(blocker) =
            stale_strong_test_evidence_blocker(&tests, latest_change_at, input.path_filters)
        {
            lines.push(format!("  evidence warning: {blocker}"));
            blockers.push(blocker);
        }
    } else {
        lines.push("  no session selected; no recorded tests available".to_string());
        blockers
            .push("no session context found; run deepcli on the task before handoff".to_string());
    }

    lines.push("environment:".to_string());
    append_verification_environment(
        &mut lines,
        &mut blockers,
        input.environment_checks,
        "  not requested; use `/handoff --env-check docker` or `/handoff --env-check compiler` when environment readiness matters",
    );

    if review_risk.high_findings > 0 {
        blockers.push(format!(
            "auto-reviewer reported {} high-risk finding type(s)",
            review_risk.high_findings
        ));
    }
    if review_input.is_none() {
        blockers.push("no diff evidence found".to_string());
    }

    lines.push("risks and blockers:".to_string());
    if blockers.is_empty() {
        lines.push("  none detected from recorded session signals".to_string());
    } else {
        lines.extend(blockers.iter().map(|item| format!("  - {item}")));
    }

    let scope_args = format_path_scope_args(input.path_filters);
    lines.push("next actions:".to_string());
    if review_risk.medium_findings > 0 {
        lines.push(format!(
            "  - inspect review warnings: `/review{scope_args}`"
        ));
    }
    if review_input.is_some() {
        lines.push(format!(
            "  - inspect changed files: `/diff --stat{scope_args}`"
        ));
    }
    if blockers
        .iter()
        .any(|item| blocker_needs_fresh_test_evidence(item))
    {
        let scope_args = format_path_scope_args(input.path_filters);
        lines.push(
            format!(
                "  - add strong test evidence: `/verify --test-command 'cargo test'{scope_args}` or `/verify --run-tests{scope_args}`"
            ),
        );
    }
    append_handoff_environment_next_actions(&mut lines, input.environment_checks);
    if blockers.is_empty() {
        lines.push("  - generate commit message: `/git message`".to_string());
        lines.push("  - commit when ready: `/git commit <message>`".to_string());
    } else {
        lines.push("  - resolve blockers, then rerun `/handoff`".to_string());
    }

    Ok(lines.join("\n"))
}

fn handoff_diff_label_and_review_input(
    workspace: &Path,
    diff_source: VerificationDiffSource,
    path_filters: &[String],
) -> (String, Option<String>, Option<DateTime<Utc>>) {
    match diff_source {
        VerificationDiffSource::Git { diff } => {
            let (added, removed) = diff_line_counts(&diff);
            let latest_change_at = latest_workspace_diff_mtime(workspace, &diff);
            let label = if path_filters.is_empty() {
                format!("git diff with +{added} -{removed} changed line(s)")
            } else {
                format!(
                    "git diff scoped to {} with +{} -{} changed line(s)",
                    format_verify_path_filters(path_filters),
                    added,
                    removed
                )
            };
            (label, Some(diff), latest_change_at)
        }
        VerificationDiffSource::Session(source) => {
            let latest_change_at = latest_session_diff_modified_at(&source.records);
            let review_input = session_diff_review_input(&source.records);
            let (added, removed) = diff_line_counts(&review_input);
            let scope = if path_filters.is_empty() {
                String::new()
            } else {
                format!(" scoped to {}", format_verify_path_filters(path_filters))
            };
            let mut label = format!(
                "session diff fallback from {}{} with {} record(s), +{} -{} changed line(s)",
                source.session.id(),
                scope,
                source.records.len(),
                added,
                removed
            );
            if let Some(note) = source.note {
                label.push_str(&format!(" ({note})"));
            }
            (label, Some(review_input), latest_change_at)
        }
        VerificationDiffSource::None {
            git_available,
            detail,
        } => {
            let mut label = if git_available {
                "none: no local Git diff and no session diff records found".to_string()
            } else {
                "none: Git diff unavailable and no session diff records found".to_string()
            };
            if let Some(detail) = detail {
                label.push_str(&format!(" ({detail})"));
            }
            (label, None, None)
        }
    }
}

pub(crate) fn format_handoff_report_markdown(report: &str) -> String {
    let mut lines = vec!["# deepcli Handoff".to_string()];
    for line in report.lines() {
        if line == "handoff report" || line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            if let Some(section) = line.strip_suffix(':') {
                if let Some(title) = handoff_markdown_section_title(section) {
                    lines.push(String::new());
                    lines.push(format!("## {title}"));
                    continue;
                }
            }
        }
        if let Some(item) = line.strip_prefix("  - ") {
            lines.push(format!("- {item}"));
        } else if let Some(item) = line.strip_prefix("- ") {
            lines.push(format!("- {item}"));
        } else if let Some(item) = line.strip_prefix("  ") {
            lines.push(item.to_string());
        } else {
            lines.push(line.to_string());
        }
    }
    while matches!(lines.last(), Some(line) if line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn handoff_markdown_section_title(section: &str) -> Option<&'static str> {
    match section {
        "summary" => Some("Summary"),
        "changed files" => Some("Changed Files"),
        "review" => Some("Review"),
        "tests" => Some("Tests"),
        "environment" => Some("Environment"),
        "risks and blockers" => Some("Risks and Blockers"),
        "next actions" => Some("Next Actions"),
        _ => None,
    }
}

pub(crate) fn format_handoff_report_pr_description(report: &str) -> String {
    let blockers = handoff_report_blockers(report);
    let mut lines = vec![
        "<!-- generated by deepcli handoff --pr -->".to_string(),
        "## Summary".to_string(),
    ];
    append_pr_section_items(
        &mut lines,
        &handoff_report_section_lines(report, "summary:"),
        "No summary evidence available.",
    );

    append_pr_section(
        &mut lines,
        "Changes",
        &handoff_report_section_lines(report, "changed files:"),
        "No changed files were detected.",
    );
    append_pr_section(
        &mut lines,
        "Test Plan",
        &handoff_report_section_lines(report, "tests:"),
        "No test evidence was recorded.",
    );
    append_pr_section(
        &mut lines,
        "Environment",
        &handoff_report_section_lines(report, "environment:"),
        "No environment evidence was requested.",
    );

    lines.push(String::new());
    lines.push("## Risks and Blockers".to_string());
    if blockers.is_empty() {
        lines.push("- No blockers detected by deepcli handoff.".to_string());
    } else {
        lines.extend(blockers.iter().map(|item| format!("- BLOCKER: {item}")));
    }

    append_pr_section(
        &mut lines,
        "Next Actions",
        &handoff_report_section_lines(report, "next actions:"),
        "No next actions were suggested.",
    );

    lines.push(String::new());
    lines.push("## Checklist".to_string());
    lines.push("- [ ] Review the changed files and generated diff summary".to_string());
    lines.push("- [ ] Confirm the test evidence is sufficient for this change".to_string());
    if blockers.is_empty() {
        lines.push("- [ ] Complete human review before merge".to_string());
    } else {
        lines.push("- [ ] Resolve all blockers before merge".to_string());
    }

    lines.join("\n")
}

fn append_pr_section(lines: &mut Vec<String>, title: &str, section_lines: &[String], empty: &str) {
    lines.push(String::new());
    lines.push(format!("## {title}"));
    append_pr_section_items(lines, section_lines, empty);
}

fn append_pr_section_items(lines: &mut Vec<String>, section_lines: &[String], empty: &str) {
    let mut appended = false;
    for line in section_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            lines.push(format!("- {item}"));
        } else {
            lines.push(format!("- {trimmed}"));
        }
        appended = true;
    }
    if !appended {
        lines.push(format!("- {empty}"));
    }
}

fn handoff_report_section_lines(report: &str, section: &str) -> Vec<String> {
    let mut in_section = false;
    let mut lines = Vec::new();
    for line in report.lines() {
        if line == section {
            in_section = true;
            continue;
        }
        if in_section && !line.starts_with(' ') && line.ends_with(':') {
            break;
        }
        if in_section {
            lines.push(line.to_string());
        }
    }
    lines
}

pub(crate) fn format_handoff_report_json(
    report: &str,
    environment_checks: &[VerificationEnvironmentCheck],
) -> Result<String> {
    let blockers = handoff_report_blockers(report);
    let next_actions = handoff_report_next_actions(report);
    let checklist = delivery_action_checklist(&next_actions);
    let value = json!({
        "schema": schema_ids::HANDOFF_V1,
        "status": if blockers.is_empty() { "ok" } else { "blocked" },
        "hasBlockers": !blockers.is_empty(),
        "blockers": blockers,
        "nextActions": next_actions,
        "checklist": checklist,
        "workspace": handoff_report_prefixed_value(report, "- workspace: "),
        "session": handoff_report_prefixed_value(report, "- session: "),
        "gitStatus": handoff_report_prefixed_value(report, "- git: "),
        "scope": handoff_report_scope_paths(report),
        "diffSource": handoff_report_prefixed_value(report, "- diff: "),
        "environment": verification_environment_json(environment_checks),
        "report": report,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

pub(crate) fn handoff_report_blockers(report: &str) -> Vec<String> {
    let mut in_blockers = false;
    let mut blockers = Vec::new();
    for line in report.lines() {
        if line == "risks and blockers:" {
            in_blockers = true;
            continue;
        }
        if in_blockers {
            if let Some(item) = line.strip_prefix("  - ") {
                blockers.push(item.to_string());
                continue;
            }
            if line == "next actions:" {
                break;
            }
        }
    }
    blockers
}

fn handoff_report_next_actions(report: &str) -> Vec<String> {
    let mut in_next_actions = false;
    let mut actions = Vec::new();
    for line in report.lines() {
        if line == "next actions:" {
            in_next_actions = true;
            continue;
        }
        if in_next_actions {
            if let Some(item) = line.strip_prefix("  - ") {
                actions.extend(report_next_action_commands(item));
            }
        }
    }
    dedup_preserve_order(actions)
}

fn handoff_report_prefixed_value(report: &str, prefix: &str) -> Option<String> {
    report
        .lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::to_string))
}

fn handoff_report_scope_paths(report: &str) -> Vec<String> {
    handoff_report_prefixed_value(report, "- scope: paths=")
        .map(|paths| {
            paths
                .split(',')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn blocker_needs_fresh_test_evidence(item: &str) -> bool {
    item.contains("test runs")
        || item.contains("test run failed")
        || item.contains("strong passing test evidence")
        || item.contains("weak test evidence")
}

fn verification_report_has_blockers(report: &str) -> bool {
    !verification_report_blockers(report).is_empty()
}

pub(crate) fn verification_output_has_blockers(output: &str, json_output: bool) -> bool {
    if json_output {
        return serde_json::from_str::<Value>(output)
            .ok()
            .and_then(|value| value.get("hasBlockers").and_then(Value::as_bool))
            .unwrap_or(true);
    }
    verification_report_has_blockers(output)
}

pub(crate) fn format_verification_report_json(
    report: &str,
    environment_checks: &[VerificationEnvironmentCheck],
) -> Result<String> {
    let blockers = verification_report_blockers(report);
    let next_actions = verification_report_next_actions(report);
    let checklist = delivery_action_checklist(&next_actions);
    let value = json!({
        "schema": schema_ids::VERIFY_V1,
        "status": if blockers.is_empty() { "ok" } else { "blocked" },
        "hasBlockers": !blockers.is_empty(),
        "blockers": blockers,
        "nextActions": next_actions,
        "checklist": checklist,
        "workspace": verification_report_prefixed_value(report, "workspace: "),
        "session": verification_report_prefixed_value(report, "session: "),
        "gitStatus": verification_report_prefixed_value(report, "git status: "),
        "scope": verification_report_scope_paths(report),
        "diffSource": verification_report_prefixed_value(report, "diff source: "),
        "environment": verification_environment_json(environment_checks),
        "report": report,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

fn delivery_action_checklist(actions: &[String]) -> Vec<Value> {
    actions
        .iter()
        .enumerate()
        .map(|(index, command)| {
            json!({
                "step": index + 1,
                "label": delivery_action_label(command),
                "command": command,
            })
        })
        .collect()
}

fn delivery_action_label(command: &str) -> &'static str {
    if command.starts_with("deepcli verify --test-command") {
        "Record cargo test evidence"
    } else if command.starts_with("deepcli verify --run-tests") {
        "Run discovered tests"
    } else if command.starts_with("deepcli verify --env-check docker") {
        "Verify Docker environment"
    } else if command.starts_with("deepcli verify --env-check compiler") {
        "Verify compiler environment"
    } else if command.starts_with("deepcli handoff --env-check docker") {
        "Prepare handoff with Docker evidence"
    } else if command.starts_with("deepcli handoff --env-check compiler") {
        "Prepare handoff with compiler evidence"
    } else if command.starts_with("deepcli handoff") {
        "Prepare handoff report"
    } else if command.starts_with("deepcli session diffs") {
        "Inspect session diffs"
    } else if command.starts_with("deepcli review") {
        "Review current diff"
    } else if command.starts_with("deepcli diff --stat") {
        "Review diff summary"
    } else if command.starts_with("deepcli diff") {
        "Review current diff"
    } else if command.starts_with("git status") {
        "Inspect Git status"
    } else if command.starts_with("cargo test") {
        "Run cargo test"
    } else {
        generic_recipe_command_label(command)
    }
}

fn verification_report_blockers(report: &str) -> Vec<String> {
    let mut in_blockers = false;
    let mut blockers = Vec::new();
    for line in report.lines() {
        if line == "blockers:" {
            in_blockers = true;
            continue;
        }
        if in_blockers {
            if let Some(item) = line.strip_prefix("- ") {
                blockers.push(item.to_string());
                continue;
            }
            if line == "next actions:" {
                break;
            }
        }
    }
    blockers
}

fn verification_report_next_actions(report: &str) -> Vec<String> {
    let mut in_next_actions = false;
    let mut actions = Vec::new();
    for line in report.lines() {
        if line == "next actions:" {
            in_next_actions = true;
            continue;
        }
        if in_next_actions {
            if let Some(item) = line.strip_prefix("- ") {
                actions.extend(report_next_action_commands(item));
            }
        }
    }
    dedup_preserve_order(actions)
}

fn report_next_action_commands(item: &str) -> Vec<String> {
    let quoted = backtick_segments(item);
    let candidates = if quoted.is_empty() {
        vec![item.trim().to_string()]
    } else {
        quoted
    };
    candidates
        .into_iter()
        .filter_map(|candidate| normalize_report_next_action_command(&candidate))
        .collect()
}

fn backtick_segments(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_segment = false;
    for ch in text.chars() {
        if ch == '`' {
            if in_segment {
                let value = current.trim();
                if !value.is_empty() {
                    segments.push(value.to_string());
                }
                current.clear();
                in_segment = false;
            } else {
                in_segment = true;
            }
            continue;
        }
        if in_segment {
            current.push(ch);
        }
    }
    segments
}

fn normalize_report_next_action_command(raw: &str) -> Option<String> {
    let command = slash_to_deepcli_command(raw.trim());
    if command.is_empty() || command.contains('<') {
        return None;
    }
    if command.starts_with("deepcli ")
        || command.starts_with("cargo ")
        || command.starts_with("git ")
    {
        Some(command)
    } else {
        None
    }
}

fn verification_report_prefixed_value(report: &str, prefix: &str) -> Option<String> {
    report
        .lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::to_string))
}

fn verification_report_scope_paths(report: &str) -> Vec<String> {
    verification_report_prefixed_value(report, "scope: paths=")
        .map(|paths| {
            paths
                .split(',')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReviewRiskSummary {
    pub(crate) high_findings: usize,
    pub(crate) medium_findings: usize,
}

pub(crate) fn review_risk_summary_from_report(report: &str) -> ReviewRiskSummary {
    #[derive(Clone, Copy)]
    enum Section {
        High,
        Medium,
        Other,
    }

    let mut section = Section::Other;
    let mut summary = ReviewRiskSummary::default();
    for line in report.lines() {
        match line {
            "high:" => section = Section::High,
            "medium:" => section = Section::Medium,
            "low:" | "worktree:" => section = Section::Other,
            value if value.starts_with("- ") => match section {
                Section::High => summary.high_findings += 1,
                Section::Medium => summary.medium_findings += 1,
                Section::Other => {}
            },
            _ => {}
        }
    }
    summary
}

fn append_verification_test_run(lines: &mut Vec<String>, test_run: &VerificationTestRun) {
    match test_run {
        VerificationTestRun::NotRequested => {}
        VerificationTestRun::Completed {
            command,
            passed,
            exit_code,
            stdout,
            stderr,
        } => {
            lines.push(format!(
                "  requested test run: [{}] exit={exit_code:?} command={command}",
                if *passed { "passed" } else { "failed" }
            ));
            let detail = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            if !detail.is_empty() {
                lines.push(format!(
                    "  requested output: {}",
                    compact_text_line(detail, 240)
                ));
            }
        }
        VerificationTestRun::Error(error) => {
            lines.push(format!(
                "  requested test run: error {}",
                compact_text_line(error, 240)
            ));
        }
    }
}

fn append_verification_environment(
    lines: &mut Vec<String>,
    blockers: &mut Vec<String>,
    checks: &[VerificationEnvironmentCheck],
    empty_hint: &str,
) {
    if checks.is_empty() {
        lines.push(empty_hint.to_string());
        return;
    }

    for check in checks {
        match check {
            VerificationEnvironmentCheck::Completed {
                target,
                report,
                text,
            } => {
                lines.push(format!(
                    "  {target}: [{}] ready={} recommended={}",
                    environment_status(report.ready),
                    report.ready,
                    report
                        .recommended_action
                        .as_deref()
                        .map(with_smoke)
                        .unwrap_or_else(|| "<none>".to_string())
                ));
                let missing = report
                    .checks
                    .iter()
                    .filter(|check| !check.available)
                    .map(|check| check.name.as_str())
                    .collect::<Vec<_>>();
                if missing.is_empty() {
                    lines.push("  checks: all available".to_string());
                } else {
                    lines.push(format!("  missing checks: {}", missing.join(", ")));
                }
                let detail = first_line(text);
                if !detail.is_empty() {
                    lines.push(format!(
                        "  environment report: {}",
                        compact_text_line(detail, 240)
                    ));
                }
                if !report.ready {
                    let action = report
                        .recommended_action
                        .as_deref()
                        .map(with_smoke)
                        .unwrap_or_else(|| env_inspect_slash(target));
                    blockers.push(format!(
                        "environment `{target}` is not ready; run `{}`",
                        action
                    ));
                }
            }
            VerificationEnvironmentCheck::Error { target, error } => {
                lines.push(format!(
                    "  {target}: error {}",
                    compact_text_line(error, 240)
                ));
                blockers.push(format!("environment `{target}` check failed"));
            }
        }
    }
}

fn verification_environment_json(checks: &[VerificationEnvironmentCheck]) -> Value {
    if checks.is_empty() {
        return json!({
            "requested": false,
            "targets": [],
        });
    }

    json!({
        "requested": true,
        "targets": checks.iter().map(verification_environment_check_json).collect::<Vec<_>>(),
    })
}

fn verification_environment_check_json(check: &VerificationEnvironmentCheck) -> Value {
    match check {
        VerificationEnvironmentCheck::Completed {
            target,
            report,
            text,
        } => json!({
            "target": target,
            "status": environment_status(report.ready),
            "ready": report.ready,
            "checks": environment_checks_json(report),
        "recommendedAction": report.recommended_action.as_deref().map(|action| redact_sensitive_text(&with_smoke(action))),
            "report": redact_sensitive_text(text),
        }),
        VerificationEnvironmentCheck::Error { target, error } => json!({
            "target": target,
            "status": "error",
            "ready": false,
            "error": redact_sensitive_text(error),
        }),
    }
}

fn format_git_status_summary(available: bool, status: &str, detail: Option<String>) -> String {
    if !available {
        return format!(
            "git status: unavailable{}",
            detail
                .map(|detail| format!(" ({detail})"))
                .unwrap_or_default()
        );
    }
    let changed = status
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let untracked = status
        .lines()
        .filter(|line| line.starts_with("?? "))
        .count();
    if changed == 0 {
        "git status: clean".to_string()
    } else {
        format!("git status: {changed} changed path(s), {untracked} untracked")
    }
}

pub(crate) fn diff_line_counts(diff: &str) -> (usize, usize) {
    let added = diff
        .lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
        .count();
    let removed = diff
        .lines()
        .filter(|line| line.starts_with('-') && !line.starts_with("---"))
        .count();
    (added, removed)
}

fn session_verification_blockers(session: &Session) -> Result<Vec<String>> {
    let mut blockers = Vec::new();
    if matches!(
        session.metadata.state,
        SessionState::AwaitingApproval | SessionState::Failed | SessionState::Paused
    ) {
        blockers.push(format!("session state is {:?}", session.metadata.state));
    }
    let pending_approvals = session
        .load_approval_requests()?
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    if pending_approvals > 0 {
        blockers.push(format!("{pending_approvals} pending approval request(s)"));
    }
    let open_questions = session
        .load_side_questions()?
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    if open_questions > 0 {
        blockers.push(format!("{open_questions} open by-the-way question(s)"));
    }
    let failed_tools = session
        .load_tool_calls()?
        .iter()
        .filter(|item| is_failed_or_denied_tool_call(item))
        .count();
    if failed_tools > 0 {
        blockers.push(format!("{failed_tools} failed or denied tool call(s)"));
    }
    let failed_tests = session
        .load_test_runs()?
        .iter()
        .filter(|item| !item.passed)
        .count();
    if failed_tests > 0 {
        blockers.push(format!("{failed_tests} failed test run(s)"));
    }
    if let Some(plan) = session.load_plan()? {
        let failed_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Failed)
            .count();
        let incomplete_steps = plan
            .steps
            .iter()
            .filter(|step| {
                matches!(
                    step.status,
                    PlanStepStatus::Pending | PlanStepStatus::InProgress
                )
            })
            .count();
        if failed_steps > 0 {
            blockers.push(format!("{failed_steps} failed plan step(s)"));
        }
        if incomplete_steps > 0 {
            blockers.push(format!("{incomplete_steps} incomplete plan step(s)"));
        }
    }
    Ok(blockers)
}

fn verification_next_actions(
    session: Option<&Session>,
    has_diff: bool,
    no_blockers: bool,
    test_run_requested: bool,
    needs_fresh_test_evidence: bool,
    path_filters: &[String],
    environment_checks: &[VerificationEnvironmentCheck],
) -> Vec<String> {
    let session_short = session.map(|session| short_id(&session.id()));
    let mut actions = Vec::new();
    if let Some(short) = session_short.as_deref() {
        if !no_blockers {
            actions.push(format!("- inspect recovery plan: `/session next {short}`"));
        }
        actions.push(format!(
            "- inspect recorded tests: `/session tests --limit 5 {short}`"
        ));
        actions.push(format!(
            "- inspect session usage and trace: `/usage {short}` and `/trace --limit 30 {short}`"
        ));
    }
    if has_diff {
        let scope_args = format_path_scope_args(path_filters);
        actions.push(format!("- review changes: `/review{scope_args}`"));
        actions.push(format!(
            "- inspect diff summary: `/diff --stat{scope_args}`"
        ));
        actions.push(format!(
            "- inspect limited diff: `/diff --limit 200{scope_args}`"
        ));
    }
    if needs_fresh_test_evidence {
        let scope_args = format_path_scope_args(path_filters);
        actions.push(format!(
            "- add strong test evidence: `/verify --run-tests{scope_args}` or `/verify --test-command 'cargo test'{scope_args}`"
        ));
    } else if !test_run_requested {
        actions.push(
            "- include a fresh test run in this report: `/verify --run-tests` or `/verify --test-command 'cargo test'`"
                .to_string(),
        );
    }
    if environment_checks.is_empty() {
        actions.push(
            "- include environment readiness if Docker/compiler matters: `/verify --env-check docker` or `/verify --env-check compiler`"
                .to_string(),
        );
    } else {
        for check in environment_checks {
            match check {
                VerificationEnvironmentCheck::Completed { target, report, .. } if !report.ready => {
                    if let Some(action) = &report.recommended_action {
                        actions.push(format!(
                            "- repair environment `{target}`: `{}`",
                            with_smoke(action)
                        ));
                    } else {
                        actions.push(format!(
                            "- inspect environment `{target}`: `{}`",
                            env_inspect_slash(target)
                        ));
                    }
                }
                VerificationEnvironmentCheck::Error { target, .. } => actions.push(format!(
                    "- inspect environment `{target}`: `{}`",
                    env_inspect_slash(target)
                )),
                _ => {}
            }
        }
    }
    if no_blockers && has_diff {
        actions.push("- if the report matches expectations, prepare handoff with `/git message` or commit through `/git commit <message>`".to_string());
    } else if !has_diff {
        actions.push("- no diff evidence found; run the task or inspect `/session diffs` before accepting implementation work".to_string());
    }
    actions
}

fn append_handoff_environment_next_actions(
    lines: &mut Vec<String>,
    environment_checks: &[VerificationEnvironmentCheck],
) {
    if environment_checks.is_empty() {
        lines.push(
            "  - include environment readiness if Docker/compiler matters: `/handoff --env-check docker` or `/handoff --env-check compiler`"
                .to_string(),
        );
        return;
    }
    for check in environment_checks {
        match check {
            VerificationEnvironmentCheck::Completed { target, report, .. } if !report.ready => {
                if let Some(action) = &report.recommended_action {
                    lines.push(format!(
                        "  - repair environment `{target}`: `{}`",
                        with_smoke(action)
                    ));
                } else {
                    lines.push(format!(
                        "  - inspect environment `{target}`: `{}`",
                        env_inspect_slash(target)
                    ));
                }
            }
            VerificationEnvironmentCheck::Error { target, .. } => {
                lines.push(format!(
                    "  - inspect environment `{target}`: `{}`",
                    env_inspect_slash(target)
                ));
            }
            _ => {}
        }
    }
}
