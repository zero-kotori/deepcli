use super::{
    dedup_preserve_order, local_action_checklist, set_command_output_path, write_command_output,
};
use crate::privacy::redact_sensitive_text;
use crate::schema_ids;
use crate::tools::ToolExecutor;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) async fn handle_test(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("discover") => {
            let option_args = if args.first().map(String::as_str) == Some("discover") {
                &args[1..]
            } else {
                args.as_slice()
            };
            let options = parse_test_read_options(option_args, "/test discover")?;
            let output = executor.execute("discover_tests", json!({})).await?;
            let text = if output.content.trim().is_empty() {
                "no test command discovered".to_string()
            } else {
                output.content.clone()
            };
            let output = if options.json_output {
                format_test_discover_json(workspace, &output.raw, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_test_read_options(&args, "/test discover")?;
            let output = executor.execute("discover_tests", json!({})).await?;
            let text = if output.content.trim().is_empty() {
                "no test command discovered".to_string()
            } else {
                output.content.clone()
            };
            let output = if options.json_output {
                format_test_discover_json(workspace, &output.raw, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("run") => {
            let parsed = parse_test_run_args(&args[1..])?;
            let tool_args = if parsed.command.trim().is_empty() {
                json!({})
            } else {
                json!({ "command": parsed.command })
            };
            let output = executor.execute("run_tests", tool_args).await?;
            let text = output.content.clone();
            let output = if parsed.options.json_output {
                format_test_run_json(workspace, &output.raw, &text)?
            } else {
                text
            };
            if let Some(output_path) = &parsed.options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(other) => bail!("unsupported /test action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TestReadOptions {
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TestRunArgs {
    options: TestReadOptions,
    command: String,
}

fn parse_test_read_options(args: &[String], command: &str) -> Result<TestReadOptions> {
    let mut options = TestReadOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("{command} --output requires a path"))?;
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
            value => bail!("unsupported {command} option `{value}`"),
        }
    }
    Ok(options)
}

fn parse_test_run_args(args: &[String]) -> Result<TestRunArgs> {
    let mut parsed = TestRunArgs::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                parsed.options.json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("/test run --output requires a path"))?;
                set_command_output_path(&mut parsed.options.output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(
                    &mut parsed.options.output_path,
                    value.trim_start_matches("--output="),
                )?;
                index += 1;
            }
            "--" => {
                parsed.command = args[index + 1..].join(" ");
                break;
            }
            _ => {
                parsed.command = args[index..].join(" ");
                break;
            }
        }
    }
    Ok(parsed)
}

fn format_test_discover_json(workspace: &Path, raw: &Value, report: &str) -> Result<String> {
    let commands = raw
        .get("commands")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let next_actions = test_discover_next_actions(raw);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::TEST_INSPECT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "discover",
        "commandCount": commands.len(),
        "commands": commands
            .into_iter()
            .map(|command| discovered_test_command_json(workspace, command))
            .collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(report),
        "format": "json",
    }))?)
}

fn format_test_run_json(workspace: &Path, raw: &Value, report: &str) -> Result<String> {
    let output = raw.get("output").cloned().unwrap_or(Value::Null);
    let passed = raw.get("passed").and_then(Value::as_bool).unwrap_or(false);
    let command = output
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let stdout = output
        .get("stdout")
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .unwrap_or_default();
    let stderr = output
        .get("stderr")
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .unwrap_or_default();
    let next_actions = test_run_next_actions(passed, command);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::TEST_INSPECT_V1,
        "status": if passed { "passed" } else { "failed" },
        "workspace": workspace.display().to_string(),
        "kind": "run",
        "passed": passed,
        "command": redact_sensitive_text(command),
        "exitCode": output.get("exit_code").cloned().unwrap_or(Value::Null),
        "stdout": stdout,
        "stderr": stderr,
        "stdoutChars": stdout.chars().count(),
        "stderrChars": stderr.chars().count(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": redact_sensitive_text(report),
        "format": "json",
    }))?)
}

pub(super) fn discovered_test_command_json(workspace: &Path, command: Value) -> Value {
    let source = command
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source_path = Path::new(source);
    let relative_source = source_path
        .strip_prefix(workspace)
        .unwrap_or(source_path)
        .display()
        .to_string();
    json!({
        "source": relative_source,
        "sourcePath": source,
        "command": command.get("command").and_then(Value::as_str).unwrap_or_default(),
        "requiresDocker": command.get("requires_docker").and_then(Value::as_bool).unwrap_or(false),
        "available": command.get("available").cloned().unwrap_or(Value::Null),
        "note": command.get("note").and_then(Value::as_str),
    })
}

fn test_discover_next_actions(raw: &Value) -> Vec<String> {
    let commands = raw
        .get("commands")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if commands.is_empty() {
        vec![
            "deepcli help test".to_string(),
            "deepcli quickstart --check --json".to_string(),
        ]
    } else {
        let mut actions = vec!["deepcli test run --json".to_string()];
        if let Some(command) = commands
            .iter()
            .filter_map(|command| command.get("command").and_then(Value::as_str))
            .find(|command| !command.trim().is_empty())
        {
            actions.push(format_test_run_command_action(command));
        }
        actions.push("deepcli help test".to_string());
        dedup_preserve_order(actions)
    }
}

fn test_run_next_actions(passed: bool, command: &str) -> Vec<String> {
    let mut actions = Vec::new();
    if passed {
        actions.push("deepcli accept --json".to_string());
        actions.push("deepcli gate --json".to_string());
    } else {
        actions.push("deepcli test discover --json".to_string());
        actions.push("deepcli logs --json".to_string());
    }
    if !command.trim().is_empty() && command != "<unknown>" {
        actions.push(format_test_run_command_action(command));
    }
    dedup_preserve_order(actions)
}

fn format_test_run_command_action(command: &str) -> String {
    format!(
        "deepcli test run --json -- {}",
        shell_words::quote(&redact_sensitive_text(command))
    )
}
