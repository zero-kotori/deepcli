use super::*;
use anyhow::Result;
use serde_json::json;

pub(crate) async fn handle_diff(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_diff_args(&args)?;
    let output = executor
        .execute("git_diff", json!({ "staged": options.staged }))
        .await?;
    let git_diff_available = command_succeeded(&output.raw);
    if git_diff_available && !output.content.trim().is_empty() {
        let diff = filter_diff_by_paths(&output.content, &options.path_filters);
        if !diff.trim().is_empty() {
            return Ok(format_diff_display(&diff, &options));
        }
        if options.staged {
            return Ok(format!(
                "no staged Git diff matched path scope {}",
                format_verify_path_filters(&options.path_filters)
            ));
        }
    }
    if options.staged {
        return Ok(output.content);
    }

    if let Some(source) = resolve_scoped_session_diff_source(
        workspace,
        current.as_deref(),
        SESSION_DIFF_FALLBACK_LIMIT,
        &options.path_filters,
    )? {
        return Ok(format_session_diff_display(&source, &options));
    }

    let mut report = if git_diff_available {
        "no local Git diff and no session diff records found".to_string()
    } else {
        "no Git diff available and no session diff records found".to_string()
    };
    if !git_diff_available {
        if let Some(detail) = command_failure_detail(&output) {
            report.push_str(&format!("\nnote: git diff unavailable: {detail}"));
        }
        report.push_str(
            "\nnext: run `/session diffs` after a deepcli file edit or initialize Git for workspace diff",
        );
    }
    if !options.path_filters.is_empty() {
        report.push_str(&format!(
            "\nnote: no changes matched path scope {}",
            format_verify_path_filters(&options.path_filters)
        ));
    }
    Ok(report)
}

pub(crate) async fn handle_review(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let path_filters = parse_review_args(&args)?;
    let status_output = executor.execute("git_status", json!({})).await?;
    let diff_output = executor.execute("git_diff", json!({})).await?;
    let status = if command_succeeded(&status_output.raw) {
        status_output.content.as_str()
    } else {
        ""
    };
    let git_diff_available = command_succeeded(&diff_output.raw);
    if git_diff_available && !diff_output.content.trim().is_empty() {
        let diff = filter_diff_by_paths(&diff_output.content, &path_filters);
        if !diff.trim().is_empty() {
            let mut report = scoped_report_prefix(&path_filters);
            report.push_str(&review_worktree(status, &diff));
            return Ok(report);
        }
    }

    if let Some(source) = resolve_scoped_session_diff_source(
        workspace,
        current.as_deref(),
        SESSION_DIFF_FALLBACK_LIMIT,
        &path_filters,
    )? {
        let mut report = format!("session diff review: session {}", source.session.id());
        if let Some(title) = source.session.metadata.title.as_deref() {
            report.push_str(&format!(" ({title})"));
        }
        if let Some(note) = source.note {
            report.push_str(&format!("\nnote: {note}"));
        }
        if !path_filters.is_empty() {
            report.push_str(&format!(
                "\nscope: paths={}",
                format_verify_path_filters(&path_filters)
            ));
        }
        report.push('\n');
        report.push_str(&review_worktree(
            status,
            &session_diff_review_input(&source.records),
        ));
        return Ok(report);
    }

    let mut report = review_worktree(status, "");
    if !git_diff_available {
        if let Some(detail) = command_failure_detail(&diff_output) {
            report.push_str(&format!("\nnote: git diff unavailable: {detail}"));
        }
        report.push_str(
            "\nnext: run `/session diffs` after a deepcli file edit or initialize Git for workspace diff review",
        );
    }
    if !path_filters.is_empty() {
        report.push_str(&format!(
            "\nnote: no changes matched path scope {}",
            format_verify_path_filters(&path_filters)
        ));
    }
    Ok(report)
}
