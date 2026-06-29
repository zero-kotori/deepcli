use super::{
    local_action_checklist, parse_positive_usize, required_arg, set_command_output_path,
    truncate_display, workspace_relative_display, write_command_output,
};
use crate::privacy::redact_sensitive_text;
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn handle_logs(workspace: &Path, args: Vec<String>) -> Result<String> {
    let options = parse_logs_options(&args)?;
    let report = build_logs_report(workspace, &options)?;
    let output = if options.json_output {
        format_logs_report_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogsOptions {
    limit: usize,
    list_only: bool,
    file: Option<String>,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_logs_options(args: &[String]) -> Result<LogsOptions> {
    let mut options = LogsOptions {
        limit: 80,
        list_only: false,
        file: None,
        json_output: false,
        output_path: None,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = parse_positive_usize(raw, "limit")?;
                index += 2;
            }
            value if index == 0 && value.parse::<usize>().is_ok() => {
                options.limit = parse_positive_usize(value, "limit")?;
                index += 1;
            }
            "--list" => {
                options.list_only = true;
                index += 1;
            }
            "--file" => {
                let raw = required_arg(args, index + 1, "log file")?;
                set_logs_file(&mut options.file, raw)?;
                index += 2;
            }
            value if value.starts_with("--file=") => {
                set_logs_file(&mut options.file, value.trim_start_matches("--file="))?;
                index += 1;
            }
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
            value if value.starts_with('-') => bail!("unsupported /logs option `{value}`"),
            value => {
                set_logs_file(&mut options.file, value)?;
                index += 1;
            }
        }
    }
    options.limit = options.limit.clamp(1, 1_000);
    Ok(options)
}

fn set_logs_file(file: &mut Option<String>, raw: &str) -> Result<()> {
    let value = raw.trim();
    if value.is_empty() {
        bail!("--file requires a log file name");
    }
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("log file path traversal is not allowed");
    }
    if file.is_some() {
        bail!("multiple log files were provided");
    }
    *file = Some(value.replace('\\', "/"));
    Ok(())
}

#[derive(Debug, Clone)]
struct LogsReport {
    logs_dir: PathBuf,
    files: Vec<LogFileSummary>,
    selected: Option<LogFileSummary>,
    tail: Option<LogTail>,
    limit: usize,
    list_only: bool,
    next_actions: Vec<String>,
    report: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LogFileSummary {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) bytes: u64,
    pub(crate) modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct LogTail {
    lines: Vec<String>,
    total_lines: usize,
}

fn build_logs_report(workspace: &Path, options: &LogsOptions) -> Result<LogsReport> {
    let logs_dir = workspace.join(".deepcli/logs");
    let files = list_log_files(&logs_dir)?;
    let selected = select_log_file(&logs_dir, &files, options.file.as_deref())?;
    let tail = selected
        .as_ref()
        .filter(|_| !options.list_only)
        .map(|file| read_log_tail(file, options.limit))
        .transpose()?;
    let next_actions = logs_next_actions(selected.is_some());
    let report = format_logs_report(
        workspace,
        LogsReportFormatInput {
            logs_dir: &logs_dir,
            files: &files,
            selected: selected.as_ref(),
            tail: tail.as_ref(),
            limit: options.limit,
            list_only: options.list_only,
            next_actions: &next_actions,
        },
    );
    Ok(LogsReport {
        logs_dir,
        files,
        selected,
        tail,
        limit: options.limit,
        list_only: options.list_only,
        next_actions,
        report,
    })
}

pub(crate) fn list_log_files(logs_dir: &Path) -> Result<Vec<LogFileSummary>> {
    if !logs_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in
        fs::read_dir(logs_dir).with_context(|| format!("failed to read {}", logs_dir.display()))?
    {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        files.push(LogFileSummary {
            name: entry.file_name().to_string_lossy().to_string(),
            path: entry.path(),
            bytes: metadata.len(),
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
        });
    }
    files.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(files)
}

fn select_log_file(
    logs_dir: &Path,
    files: &[LogFileSummary],
    requested: Option<&str>,
) -> Result<Option<LogFileSummary>> {
    if let Some(requested) = requested {
        let requested = requested.replace('\\', "/");
        if let Some(file) = files.iter().find(|file| file.name == requested) {
            return Ok(Some(file.clone()));
        }
        let path = logs_dir.join(&requested);
        if path.is_file() {
            let metadata = fs::metadata(&path)
                .with_context(|| format!("failed to stat {}", path.display()))?;
            return Ok(Some(LogFileSummary {
                name: requested,
                path,
                bytes: metadata.len(),
                modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            }));
        }
        bail!("log file `{requested}` was not found in .deepcli/logs");
    }
    Ok(files.first().cloned())
}

fn read_log_tail(file: &LogFileSummary, limit: usize) -> Result<LogTail> {
    let bytes =
        fs::read(&file.path).with_context(|| format!("failed to read {}", file.path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let redacted = redact_sensitive_text(&text);
    let lines = redacted.lines().collect::<Vec<_>>();
    let skip = lines.len().saturating_sub(limit);
    Ok(LogTail {
        lines: lines
            .iter()
            .skip(skip)
            .map(|line| truncate_display(line, 1_000))
            .collect(),
        total_lines: lines.len(),
    })
}

struct LogsReportFormatInput<'a> {
    logs_dir: &'a Path,
    files: &'a [LogFileSummary],
    selected: Option<&'a LogFileSummary>,
    tail: Option<&'a LogTail>,
    limit: usize,
    list_only: bool,
    next_actions: &'a [String],
}

fn format_logs_report(workspace: &Path, input: LogsReportFormatInput<'_>) -> String {
    let mut lines = vec![
        "deepcli logs".to_string(),
        format!(
            "logs dir: {}",
            workspace_relative_display(workspace, input.logs_dir)
        ),
    ];
    if input.files.is_empty() && input.selected.is_none() {
        lines.push("status: no log files found".to_string());
    } else {
        lines.push(format!("log files: {}", input.files.len()));
        for file in input.files.iter().take(20) {
            lines.push(format!("  - {}", format_log_file_summary(file)));
        }
        if input.files.len() > 20 {
            lines.push(format!("  ... {} more file(s)", input.files.len() - 20));
        }
    }

    if let Some(file) = input.selected {
        lines.push(format!("selected: {}", format_log_file_summary(file)));
    }
    if let Some(tail) = input.tail {
        let shown = tail.lines.len();
        lines.push(format!(
            "showing latest {shown}/{} line(s), limit={limit}",
            tail.total_lines,
            limit = input.limit
        ));
        if tail.lines.is_empty() {
            lines.push("<empty log file>".to_string());
        } else {
            lines.extend(tail.lines.iter().cloned());
        }
    } else if input.list_only {
        lines.push("tail: skipped because --list was requested".to_string());
    }

    lines.push("next actions:".to_string());
    lines.extend(
        input
            .next_actions
            .iter()
            .map(|action| format!("- {action}")),
    );
    lines.join("\n")
}

fn format_log_file_summary(file: &LogFileSummary) -> String {
    format!(
        "{} bytes={} modified={}",
        redact_sensitive_text(&file.name),
        file.bytes,
        file.modified_at
            .map(|time| time.to_rfc3339())
            .unwrap_or_else(|| "<unknown>".to_string())
    )
}

fn logs_next_actions(has_logs: bool) -> Vec<String> {
    let mut actions = vec![
        "deepcli trace --limit 30".to_string(),
        "deepcli usage --json".to_string(),
        "deepcli support".to_string(),
    ];
    if !has_logs {
        actions.insert(
            0,
            "deepcli diagnose --bundle .deepcli/support/latest".to_string(),
        );
    }
    actions
}

fn format_logs_report_json(workspace: &Path, report: &LogsReport) -> Result<String> {
    let shown_lines = report
        .tail
        .as_ref()
        .map(|tail| tail.lines.len())
        .unwrap_or_default();
    let total_lines = report
        .tail
        .as_ref()
        .map(|tail| tail.total_lines)
        .unwrap_or_default();
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::LOGS_V1,
        "status": if report.selected.is_some() { "ok" } else { "no_logs" },
        "workspace": workspace.display().to_string(),
        "logsDir": workspace_relative_display(workspace, &report.logs_dir),
        "limit": report.limit,
        "listOnly": report.list_only,
        "fileCount": report.files.len(),
        "files": report.files.iter().map(log_file_summary_json).collect::<Vec<_>>(),
        "selectedFile": report
            .selected
            .as_ref()
            .map(log_file_summary_json)
            .unwrap_or(Value::Null),
        "lines": report
            .tail
            .as_ref()
            .map(|tail| tail.lines.clone())
            .unwrap_or_default(),
        "lineCount": shown_lines,
        "totalLines": total_lines,
        "truncated": total_lines > shown_lines,
        "checklist": local_action_checklist(&report.next_actions),
        "nextActions": report.next_actions,
        "report": report.report,
    }))?)
}

fn log_file_summary_json(file: &LogFileSummary) -> Value {
    json!({
        "name": redact_sensitive_text(&file.name),
        "bytes": file.bytes,
        "modifiedAt": file.modified_at.map(|time| time.to_rfc3339()),
    })
}
