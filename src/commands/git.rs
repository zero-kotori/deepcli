use super::{
    dedup_preserve_order, local_action_checklist, required_arg, set_command_output_path,
    write_command_output,
};
use crate::privacy::redact_sensitive_text;
use crate::schema_ids;
use crate::tools::ToolExecutor;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) async fn handle_git(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_git_options(&args)?;
    match options.action.as_str() {
        "status" => {
            let output = executor.execute("git_status", json!({})).await?;
            Ok(format_git_read_output(
                workspace,
                &options,
                "git status --short",
                output.content,
                output.raw,
            )?)
        }
        "diff" => {
            let command = if options.staged {
                "git diff --cached"
            } else {
                "git diff"
            };
            let output = executor
                .execute("git_diff", json!({"staged": options.staged}))
                .await?;
            Ok(format_git_read_output(
                workspace,
                &options,
                command,
                output.content,
                output.raw,
            )?)
        }
        "branch" => {
            let output = executor.execute("git_branch", json!({})).await?;
            Ok(format_git_read_output(
                workspace,
                &options,
                "git branch --show-current && git branch --list",
                output.content,
                output.raw,
            )?)
        }
        "message" => {
            let output = executor.execute("git_commit_message", json!({})).await?;
            let command = "git status --short && git diff --name-only && git diff --stat";
            Ok(format_git_read_output(
                workspace,
                &options,
                command,
                output.content,
                output.raw,
            )?)
        }
        "create-branch" => {
            let options = parse_git_create_branch_args(&args)?;
            let command = git_create_branch_command(&options.subject);
            if options.dry_run {
                return format_git_action_output(workspace, &options, "create-branch", &command);
            }
            Ok(executor
                .execute(
                    "git_create_branch",
                    json!({"name": options.subject, "approved": true}),
                )
                .await?
                .content)
        }
        "commit" => {
            let options = parse_git_commit_message_args(&args)?;
            let command = git_commit_command(&options.subject);
            if options.dry_run {
                return format_git_action_output(workspace, &options, "commit", &command);
            }
            Ok(executor
                .execute(
                    "git_commit",
                    json!({"message": options.subject, "approved": true}),
                )
                .await?
                .content)
        }
        other => bail!("unsupported /git action `{other}`"),
    }
}

struct GitWriteOptions {
    subject: String,
    dry_run: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_git_create_branch_args(args: &[String]) -> Result<GitWriteOptions> {
    let mut name = None;
    let mut dry_run = false;
    let mut json_output = false;
    let mut output_path = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" | "--preview" => {
                dry_run = true;
                index += 1;
            }
            "--json" => {
                json_output = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(&mut output_path, value.trim_start_matches("--output="))?;
                index += 1;
            }
            value if value.starts_with('-') && name.is_none() => {
                bail!("unsupported /git create-branch option `{value}`");
            }
            value => {
                if name.is_some() {
                    bail!("unexpected /git create-branch argument `{value}`");
                }
                name = Some(value.to_string());
                index += 1;
            }
        }
    }
    let subject =
        name.ok_or_else(|| anyhow::anyhow!("/git create-branch requires a branch name"))?;
    validate_git_branch_name(&subject)?;
    validate_git_write_output_flags("create-branch", dry_run, json_output, output_path.as_ref())?;
    Ok(GitWriteOptions {
        subject,
        dry_run,
        json_output,
        output_path,
    })
}

fn parse_git_commit_message_args(args: &[String]) -> Result<GitWriteOptions> {
    let mut parts = Vec::new();
    let mut dry_run = false;
    let mut json_output = false;
    let mut output_path = None;
    let mut literal_message = false;
    let mut index = 1;
    while index < args.len() {
        let value = args[index].as_str();
        if !literal_message {
            match value {
                "--" => {
                    literal_message = true;
                    index += 1;
                    continue;
                }
                "--dry-run" | "--preview" => {
                    dry_run = true;
                    index += 1;
                    continue;
                }
                "--json" => {
                    json_output = true;
                    index += 1;
                    continue;
                }
                "--output" | "-o" => {
                    let raw = required_arg(args, index + 1, "output path")?;
                    set_command_output_path(&mut output_path, raw)?;
                    index += 2;
                    continue;
                }
                _ if value.starts_with("--output=") => {
                    set_command_output_path(
                        &mut output_path,
                        value.trim_start_matches("--output="),
                    )?;
                    index += 1;
                    continue;
                }
                _ if value.starts_with('-') => {
                    bail!("unexpected /git commit argument `{value}`");
                }
                _ => {}
            }
        }
        parts.push(value);
        index += 1;
    }
    let subject = parts.join(" ");
    if subject.trim().is_empty() {
        bail!("/git commit requires a message");
    }
    validate_git_write_output_flags("commit", dry_run, json_output, output_path.as_ref())?;
    Ok(GitWriteOptions {
        subject,
        dry_run,
        json_output,
        output_path,
    })
}

fn validate_git_write_output_flags(
    action: &str,
    dry_run: bool,
    json_output: bool,
    output_path: Option<&String>,
) -> Result<()> {
    if !dry_run && json_output {
        bail!("/git {action} --json is only supported with --dry-run");
    }
    if !dry_run && output_path.is_some() {
        bail!("/git {action} --output is only supported with --dry-run");
    }
    Ok(())
}

fn validate_git_branch_name(name: &str) -> Result<()> {
    if name.starts_with('-')
        || name.contains("..")
        || name.contains('@')
        || name.contains('\\')
        || name.contains(' ')
        || name.trim().is_empty()
    {
        bail!("invalid branch name `{name}`");
    }
    Ok(())
}

fn git_create_branch_command(name: &str) -> String {
    format!("git switch -c {}", shell_words::quote(name))
}

fn git_commit_command(message: &str) -> String {
    format!("git commit -m {}", shell_words::quote(message))
}

fn format_git_action_output(
    workspace: &Path,
    options: &GitWriteOptions,
    action: &str,
    command: &str,
) -> Result<String> {
    let next_actions = git_action_next_actions(action, &options.subject);
    let report = format_git_action_report(action, &options.subject, command, &next_actions);
    let output = if options.json_output {
        serde_json::to_string_pretty(&json!({
            "schema": schema_ids::GIT_ACTION_V1,
            "status": "dry_run",
            "dryRun": true,
            "workspace": workspace.display().to_string(),
            "action": action,
            "subject": options.subject,
            "command": command,
            "nextActions": next_actions,
            "report": report,
        }))?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn format_git_action_report(
    action: &str,
    subject: &str,
    command: &str,
    next_actions: &[String],
) -> String {
    let mut lines = vec![
        format!("git {action}: dry-run"),
        format!("subject: {}", redact_sensitive_text(subject)),
        format!("planned command: {}", redact_sensitive_text(command)),
        "no git write was executed".to_string(),
        "next actions:".to_string(),
    ];
    lines.extend(next_actions.iter().map(|action| format!("  - {action}")));
    lines.join("\n")
}

fn git_action_next_actions(action: &str, subject: &str) -> Vec<String> {
    let subject = shell_words::quote(subject);
    let actions = match action {
        "create-branch" => vec![
            format!("deepcli git create-branch {subject}"),
            "deepcli git branch --json".to_string(),
            "deepcli git status --json".to_string(),
        ],
        "commit" => vec![
            format!("deepcli git commit {subject}"),
            "deepcli git status --json".to_string(),
            "deepcli git message --json".to_string(),
        ],
        _ => vec!["deepcli git status --json".to_string()],
    };
    dedup_preserve_order(actions)
}

struct GitOptions {
    action: String,
    json_output: bool,
    staged: bool,
    output_path: Option<String>,
}

fn parse_git_options(args: &[String]) -> Result<GitOptions> {
    let mut action = args.first().map(String::as_str).unwrap_or("status");
    if action.starts_with('-') {
        action = "status";
    }
    if !matches!(
        action,
        "status" | "diff" | "branch" | "message" | "create-branch" | "commit"
    ) {
        bail!("unsupported /git action `{action}`");
    }
    if matches!(action, "create-branch" | "commit") {
        return Ok(GitOptions {
            action: action.to_string(),
            json_output: false,
            staged: false,
            output_path: None,
        });
    }

    let start = usize::from(args.first().is_some_and(|value| !value.starts_with('-')));
    let mut json_output = false;
    let mut staged = false;
    let mut output_path = None;
    let mut index = start;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("/git {action} --output requires a path"))?;
                set_command_output_path(&mut output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(&mut output_path, value.trim_start_matches("--output="))?;
                index += 1;
            }
            "--staged" | "--cached" if action == "diff" => {
                staged = true;
                index += 1;
            }
            "--staged" | "--cached" => bail!("unsupported /git {action} option `{}`", args[index]),
            value if value.starts_with('-') => bail!("unsupported /git {action} option `{value}`"),
            value => bail!("unexpected /git {action} argument `{value}`"),
        }
    }
    Ok(GitOptions {
        action: action.to_string(),
        json_output,
        staged,
        output_path,
    })
}

fn format_git_read_output(
    workspace: &Path,
    options: &GitOptions,
    command: &str,
    content: String,
    raw: Value,
) -> Result<String> {
    if !options.json_output {
        if let Some(output_path) = &options.output_path {
            write_command_output(workspace, output_path, &content)?;
        }
        return Ok(content);
    }
    let report = format_git_read_report(&options.action, command, &content, &raw);
    let next_actions = git_read_next_actions(&options.action);
    let checklist = local_action_checklist(&next_actions);
    let output = serde_json::to_string_pretty(&json!({
        "schema": schema_ids::GIT_INSPECT_V1,
        "status": if git_raw_exit_code(&raw) == Some(0) { "ok" } else { "failed" },
        "kind": options.action,
        "command": command,
        "exitCode": git_raw_exit_code(&raw),
        "stdout": git_raw_string(&raw, "stdout"),
        "stderr": git_raw_string(&raw, "stderr"),
        "output": content,
        "raw": raw,
        "nextActions": next_actions,
        "checklist": checklist,
        "report": report,
    }))?;
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn format_git_read_report(kind: &str, command: &str, content: &str, raw: &Value) -> String {
    let status = if git_raw_exit_code(raw) == Some(0) {
        "ok"
    } else {
        "failed"
    };
    let mut lines = vec![
        format!("git {kind}: {status}"),
        format!("command: {command}"),
    ];
    if content.trim().is_empty() {
        lines.push("output: clean or empty".to_string());
    } else {
        lines.push("output:".to_string());
        lines.extend(content.lines().map(|line| format!("  {line}")));
    }
    lines.push("next actions:".to_string());
    lines.extend(
        git_read_next_actions(kind)
            .iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn git_read_next_actions(kind: &str) -> Vec<String> {
    let mut actions = match kind {
        "status" => vec![
            "deepcli git diff --json".to_string(),
            "deepcli git message --json".to_string(),
            "deepcli review".to_string(),
        ],
        "diff" => vec![
            "deepcli git status --json".to_string(),
            "deepcli git message --json".to_string(),
            "deepcli review".to_string(),
        ],
        "branch" => vec![
            "deepcli git status --json".to_string(),
            "deepcli git message --json".to_string(),
        ],
        "message" => vec![
            "deepcli git status --json".to_string(),
            "deepcli git diff --json".to_string(),
            "deepcli gate --json".to_string(),
        ],
        _ => vec!["deepcli git status --json".to_string()],
    };
    actions.push("deepcli help git".to_string());
    dedup_preserve_order(actions)
}

fn git_raw_exit_code(raw: &Value) -> Option<i64> {
    raw.get("exit_code").and_then(Value::as_i64)
}

fn git_raw_string(raw: &Value, key: &str) -> String {
    raw.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
