use super::{
    dedup_preserve_order, git_stdout, redact_sensitive_text, required_arg, set_command_output_path,
    status_u128_value, truncate_display, write_command_output, CommandExit,
};
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::Instant;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PreflightOptions {
    pub(super) json_output: bool,
    pub(super) dry_run: bool,
    pub(super) quick: bool,
    pub(super) fail_fast: bool,
    pub(super) output_path: Option<String>,
}

#[derive(Debug, Clone)]
struct PreflightCheckSpec {
    name: String,
    command: String,
    program: Option<PathBuf>,
    args: Vec<String>,
    required: bool,
    skip_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct PreflightCheckResult {
    pub(super) name: String,
    pub(super) command: String,
    pub(super) status: String,
    pub(super) required: bool,
    pub(super) exit_code: Option<i32>,
    pub(super) duration_ms: Option<u128>,
    pub(super) stdout_chars: usize,
    pub(super) stderr_chars: usize,
    pub(super) output: Option<String>,
    pub(super) note: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct PreflightReport {
    pub(super) report: String,
    pub(super) status: String,
    pub(super) dry_run: bool,
    pub(super) quick: bool,
    pub(super) fail_fast: bool,
    pub(super) checks: Vec<PreflightCheckResult>,
    pub(super) next_actions: Vec<String>,
}

pub(crate) fn handle_preflight(workspace: &Path, args: Vec<String>) -> Result<String> {
    let options = parse_preflight_options(&args)?;
    let report = build_preflight_report(workspace, &options)?;
    let output = if options.json_output {
        format_preflight_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if report.status == "failed" {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

fn parse_preflight_options(args: &[String]) -> Result<PreflightOptions> {
    let mut options = PreflightOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--dry-run" | "--list" | "--plan" => {
                options.dry_run = true;
                index += 1;
            }
            "--quick" => {
                options.quick = true;
                index += 1;
            }
            "--fail-fast" => {
                options.fail_fast = true;
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
            value => bail!("unsupported /preflight option `{value}`"),
        }
    }
    Ok(options)
}

fn build_preflight_report(workspace: &Path, options: &PreflightOptions) -> Result<PreflightReport> {
    let specs = build_preflight_specs(workspace, options)?;
    let mut checks = Vec::new();
    for spec in specs {
        let result = if options.dry_run {
            preflight_planned_result(&spec)
        } else {
            run_preflight_check(workspace, &spec)
        };
        let failed = result.status == "failed" && result.required;
        checks.push(result);
        if failed && options.fail_fast {
            break;
        }
    }

    let status = if options.dry_run {
        "planned"
    } else if checks
        .iter()
        .any(|check| check.required && check.status == "failed")
    {
        "failed"
    } else {
        "ok"
    }
    .to_string();
    let next_actions = preflight_next_actions(&status, &checks, options);
    let report = format_preflight_text(workspace, &status, options, &checks, &next_actions);

    Ok(PreflightReport {
        report,
        status,
        dry_run: options.dry_run,
        quick: options.quick,
        fail_fast: options.fail_fast,
        checks,
        next_actions,
    })
}

fn build_preflight_specs(
    workspace: &Path,
    options: &PreflightOptions,
) -> Result<Vec<PreflightCheckSpec>> {
    let mut specs = Vec::new();
    let cargo_present = workspace.join("Cargo.toml").exists();
    let git_present = git_stdout(workspace, &["rev-parse", "--is-inside-work-tree"])
        .ok()
        .flatten()
        .as_deref()
        .is_some_and(|value| value.trim() == "true");

    specs.push(if cargo_present {
        process_preflight_spec("format", "cargo", &["fmt", "--check"], true)
    } else {
        skipped_preflight_spec("format", "cargo fmt --check", "Cargo.toml not found")
    });

    specs.push(if git_present {
        process_preflight_spec("diff-whitespace", "git", &["diff", "--check"], true)
    } else {
        skipped_preflight_spec(
            "diff-whitespace",
            "git diff --check",
            "not a git repository",
        )
    });

    if options.quick {
        specs.push(skipped_preflight_spec(
            "clippy",
            "cargo clippy --all-targets -- -D warnings",
            "skipped by --quick",
        ));
    } else if cargo_present {
        specs.push(process_preflight_spec(
            "clippy",
            "cargo",
            &["clippy", "--all-targets", "--", "-D", "warnings"],
            true,
        ));
    } else {
        specs.push(skipped_preflight_spec(
            "clippy",
            "cargo clippy --all-targets -- -D warnings",
            "Cargo.toml not found",
        ));
    }

    let deepcli = std::env::current_exe().context("failed to locate current deepcli binary")?;
    specs.push(deepcli_preflight_spec(
        workspace,
        &deepcli,
        "selftest",
        &["/selftest", "--json", "--fail-on-issues"],
    ));
    specs.push(deepcli_preflight_spec(
        workspace,
        &deepcli,
        "doctor",
        &["/doctor", "--quick", "--json"],
    ));
    let privacy_args = if options.quick {
        vec!["/privacy", "--json", "--fail-on-findings", "--no-history"]
    } else {
        vec!["/privacy", "--json", "--fail-on-findings"]
    };
    specs.push(deepcli_preflight_spec(
        workspace,
        &deepcli,
        "privacy",
        &privacy_args,
    ));
    if options.quick {
        specs.push(skipped_preflight_spec(
            "gate",
            "deepcli gate --json",
            "skipped by --quick",
        ));
    } else {
        specs.push(deepcli_preflight_spec(
            workspace,
            &deepcli,
            "gate",
            &["/gate", "--json"],
        ));
    }

    Ok(specs)
}

fn process_preflight_spec(
    name: &str,
    program: &str,
    args: &[&str],
    required: bool,
) -> PreflightCheckSpec {
    let args = args
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    PreflightCheckSpec {
        name: name.to_string(),
        command: display_process_command(program, &args),
        program: Some(PathBuf::from(program)),
        args,
        required,
        skip_reason: None,
    }
}

fn skipped_preflight_spec(name: &str, command: &str, reason: &str) -> PreflightCheckSpec {
    PreflightCheckSpec {
        name: name.to_string(),
        command: command.to_string(),
        program: None,
        args: Vec::new(),
        required: false,
        skip_reason: Some(reason.to_string()),
    }
}

fn deepcli_preflight_spec(
    workspace: &Path,
    deepcli: &Path,
    name: &str,
    slash_args: &[&str],
) -> PreflightCheckSpec {
    let mut args = vec![
        "-C".to_string(),
        workspace.display().to_string(),
        "--config".to_string(),
        workspace
            .join(".deepcli")
            .join("config.json")
            .display()
            .to_string(),
        "--yes".to_string(),
    ];
    args.extend(slash_args.iter().map(|arg| (*arg).to_string()));
    let display_args = slash_args
        .iter()
        .map(|arg| arg.trim_start_matches('/'))
        .collect::<Vec<_>>();
    PreflightCheckSpec {
        name: name.to_string(),
        command: display_process_command("deepcli", &display_args),
        program: Some(deepcli.to_path_buf()),
        args,
        required: true,
        skip_reason: None,
    }
}

fn display_process_command<S: AsRef<str>>(program: &str, args: &[S]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|arg| arg.as_ref().to_string()))
        .map(|part| shell_words::quote(&part).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn preflight_planned_result(spec: &PreflightCheckSpec) -> PreflightCheckResult {
    PreflightCheckResult {
        name: spec.name.clone(),
        command: spec.command.clone(),
        status: if spec.skip_reason.is_some() {
            "skipped".to_string()
        } else {
            "planned".to_string()
        },
        required: spec.required,
        exit_code: None,
        duration_ms: None,
        stdout_chars: 0,
        stderr_chars: 0,
        output: None,
        note: spec.skip_reason.clone(),
    }
}

fn run_preflight_check(workspace: &Path, spec: &PreflightCheckSpec) -> PreflightCheckResult {
    if spec.skip_reason.is_some() || spec.program.is_none() {
        return preflight_planned_result(spec);
    }
    let program = spec.program.as_ref().unwrap();
    let started = Instant::now();
    let output = ProcessCommand::new(program)
        .args(&spec.args)
        .current_dir(workspace)
        .output();
    let duration_ms = started.elapsed().as_millis();
    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            PreflightCheckResult {
                name: spec.name.clone(),
                command: spec.command.clone(),
                status: if output.status.success() {
                    "passed".to_string()
                } else {
                    "failed".to_string()
                },
                required: spec.required,
                exit_code: output.status.code(),
                duration_ms: Some(duration_ms),
                stdout_chars: stdout.chars().count(),
                stderr_chars: stderr.chars().count(),
                output: preflight_output_summary(&stdout, &stderr),
                note: None,
            }
        }
        Err(error) => PreflightCheckResult {
            name: spec.name.clone(),
            command: spec.command.clone(),
            status: "failed".to_string(),
            required: spec.required,
            exit_code: None,
            duration_ms: Some(duration_ms),
            stdout_chars: 0,
            stderr_chars: 0,
            output: Some(truncate_display(
                &redact_sensitive_text(&format!("failed to run command: {error}")),
                700,
            )),
            note: None,
        },
    }
}

fn preflight_output_summary(stdout: &str, stderr: &str) -> Option<String> {
    let raw = if !stderr.trim().is_empty() {
        stderr
    } else if !stdout.trim().is_empty() {
        stdout
    } else {
        return None;
    };
    Some(truncate_display(
        &redact_sensitive_text(&raw.replace('\n', "\\n")),
        900,
    ))
}

fn preflight_run_command(options: &PreflightOptions) -> String {
    let mut parts = vec!["deepcli", "preflight"];
    if options.quick {
        parts.push("--quick");
    }
    if options.fail_fast {
        parts.push("--fail-fast");
    }
    parts.push("--json");
    parts.join(" ")
}

pub(super) fn preflight_next_actions(
    status: &str,
    checks: &[PreflightCheckResult],
    options: &PreflightOptions,
) -> Vec<String> {
    let mut actions = Vec::new();
    match status {
        "planned" => {
            actions.push(preflight_run_command(options));
        }
        "failed" => {
            for check in checks
                .iter()
                .filter(|check| check.required && check.status == "failed")
                .take(4)
            {
                actions.push(check.command.clone());
            }
            actions.push(preflight_run_command(options));
        }
        _ => {
            actions.push("deepcli handoff --pr".to_string());
            actions.push("git status --short".to_string());
        }
    }
    dedup_preserve_order(actions)
}

pub(super) fn format_preflight_text(
    workspace: &Path,
    status: &str,
    options: &PreflightOptions,
    checks: &[PreflightCheckResult],
    next_actions: &[String],
) -> String {
    let mut lines = vec![
        "deepcli preflight".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("status: {status}"),
        format!("mode: {}", if options.quick { "quick" } else { "full" }),
    ];
    if options.dry_run {
        lines.push("dry-run: true".to_string());
    }
    if options.fail_fast {
        lines.push("fail-fast: true".to_string());
    }
    if let Some(diagnostics) = format_preflight_diagnostics_line(checks) {
        lines.push("diagnostics:".to_string());
        lines.push(format!("  {diagnostics}"));
    }
    lines.push("checks:".to_string());
    for check in checks {
        let mut line = format!("  - [{}] {}: {}", check.status, check.name, check.command);
        if let Some(exit_code) = check.exit_code {
            line.push_str(&format!(" exit={exit_code}"));
        }
        if let Some(duration_ms) = check.duration_ms {
            line.push_str(&format!(" duration={}ms", duration_ms));
        }
        if let Some(note) = &check.note {
            line.push_str(&format!(" note={}", redact_sensitive_text(note)));
        }
        lines.push(line);
        if let Some(output) = &check.output {
            lines.push(format!("    output: {output}"));
        }
    }
    lines.push("next actions:".to_string());
    lines.extend(next_actions.iter().map(|action| format!("  - {action}")));
    lines.join("\n")
}

fn format_preflight_diagnostics_line(checks: &[PreflightCheckResult]) -> Option<String> {
    let measured = checks
        .iter()
        .filter(|check| check.duration_ms.is_some())
        .count();
    let failed = preflight_failed_required_checks(checks);
    if measured == 0 && failed.is_empty() {
        return None;
    }

    let mut parts = vec![
        format!("total_duration={}ms", preflight_total_duration_ms(checks)),
        format!("measured_checks={measured}"),
    ];
    if let Some(check) = preflight_slowest_check(checks) {
        parts.push(format!(
            "slowest={} {}ms",
            check.name,
            check.duration_ms.unwrap_or_default()
        ));
    }
    if let Some(check) = preflight_largest_output_check(checks) {
        parts.push(format!(
            "largest_output={} {} chars",
            check.name,
            preflight_output_chars(check)
        ));
    }
    if !failed.is_empty() {
        parts.push(format!("failed_required={}", failed.join(",")));
    }
    Some(parts.join(" "))
}

pub(super) fn format_preflight_json(workspace: &Path, report: &PreflightReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::PREFLIGHT_V1,
        "status": report.status,
        "workspace": workspace.display().to_string(),
        "mode": if report.quick { "quick" } else { "full" },
        "dryRun": report.dry_run,
        "failFast": report.fail_fast,
        "counts": {
            "total": report.checks.len(),
            "passed": report.checks.iter().filter(|check| check.status == "passed").count(),
            "failed": report.checks.iter().filter(|check| check.status == "failed").count(),
            "skipped": report.checks.iter().filter(|check| check.status == "skipped").count(),
            "planned": report.checks.iter().filter(|check| check.status == "planned").count(),
        },
        "diagnostics": preflight_diagnostics_json(&report.checks),
        "checklist": preflight_checklist(&report.checks),
        "checks": report.checks.iter().map(|check| json!({
            "name": check.name,
            "command": check.command,
            "status": check.status,
            "required": check.required,
            "exitCode": check.exit_code,
            "durationMs": check.duration_ms,
            "stdoutChars": check.stdout_chars,
            "stderrChars": check.stderr_chars,
            "output": check.output,
            "note": check.note,
        })).collect::<Vec<_>>(),
        "nextActions": report.next_actions,
        "report": report.report,
    }))?)
}

fn preflight_checklist(checks: &[PreflightCheckResult]) -> Vec<Value> {
    checks
        .iter()
        .enumerate()
        .map(|(index, check)| {
            json!({
                "step": index + 1,
                "name": check.name,
                "label": preflight_checklist_label(&check.name),
                "command": check.command,
                "status": check.status,
                "required": check.required,
            })
        })
        .collect()
}

fn preflight_checklist_label(name: &str) -> &'static str {
    match name {
        "format" => "Check Rust formatting",
        "diff-whitespace" => "Check diff whitespace",
        "clippy" => "Run clippy",
        "selftest" => "Run deepcli selftest",
        "doctor" => "Run doctor diagnostics",
        "privacy" => "Run privacy scan",
        "gate" => "Run delivery gate",
        _ => "Run preflight check",
    }
}

fn preflight_diagnostics_json(checks: &[PreflightCheckResult]) -> Value {
    json!({
        "totalDurationMs": status_u128_value(preflight_total_duration_ms(checks)),
        "measuredChecks": checks.iter().filter(|check| check.duration_ms.is_some()).count(),
        "slowestCheck": preflight_slowest_check(checks)
            .map(preflight_duration_check_json)
            .unwrap_or(Value::Null),
        "largestOutputCheck": preflight_largest_output_check(checks)
            .map(preflight_output_check_json)
            .unwrap_or(Value::Null),
        "failedRequiredChecks": preflight_failed_required_checks(checks),
    })
}

fn preflight_total_duration_ms(checks: &[PreflightCheckResult]) -> u128 {
    checks
        .iter()
        .filter_map(|check| check.duration_ms)
        .sum::<u128>()
}

fn preflight_slowest_check(checks: &[PreflightCheckResult]) -> Option<&PreflightCheckResult> {
    checks
        .iter()
        .filter(|check| check.duration_ms.is_some())
        .max_by_key(|check| check.duration_ms.unwrap_or_default())
}

fn preflight_largest_output_check(
    checks: &[PreflightCheckResult],
) -> Option<&PreflightCheckResult> {
    checks
        .iter()
        .filter(|check| preflight_output_chars(check) > 0)
        .max_by_key(|check| preflight_output_chars(check))
}

fn preflight_output_chars(check: &PreflightCheckResult) -> usize {
    check.stdout_chars.saturating_add(check.stderr_chars)
}

fn preflight_failed_required_checks(checks: &[PreflightCheckResult]) -> Vec<String> {
    checks
        .iter()
        .filter(|check| check.required && check.status == "failed")
        .map(|check| check.name.clone())
        .collect::<Vec<_>>()
}

fn preflight_duration_check_json(check: &PreflightCheckResult) -> Value {
    json!({
        "name": check.name,
        "command": check.command,
        "status": check.status,
        "durationMs": check.duration_ms.map(status_u128_value).unwrap_or(Value::Null),
    })
}

fn preflight_output_check_json(check: &PreflightCheckResult) -> Value {
    json!({
        "name": check.name,
        "command": check.command,
        "status": check.status,
        "outputChars": preflight_output_chars(check),
        "stdoutChars": check.stdout_chars,
        "stderrChars": check.stderr_chars,
    })
}
