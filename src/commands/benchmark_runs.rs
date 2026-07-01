use super::*;
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) const BENCHMARK_RUN_SUITE_REMEDIATION_ACTION: &str =
    "deepcli benchmark run-suite --json --fail-on-command";

const DEFAULT_BENCHMARK_SUITE: &str = "product";
const DEFAULT_BENCHMARK_CASE: &str = "scorecard";
const DEFAULT_BENCHMARK_RUN_CASE: &str = "command";
pub(crate) const BENCHMARK_SUITE_SCHEMA: &str = schema_ids::BENCHMARK_SUITE_V1;
const DEFAULT_BENCHMARK_TIMEOUT_SECONDS: u64 = 120;
const BENCHMARK_OUTPUT_SAMPLE_CHARS: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BenchmarkRecordOptions {
    json_output: bool,
    output_path: Option<String>,
    suite: String,
    case_name: String,
    commands: Vec<String>,
    notes: Option<String>,
    include_scorecard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BenchmarkRunOptions {
    json_output: bool,
    output_path: Option<String>,
    preset: Option<String>,
    suite: String,
    case_name: String,
    command: Option<String>,
    notes: Option<String>,
    include_scorecard: bool,
    timeout_seconds: u64,
    fail_on_command: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BenchmarkRunSuiteOptions {
    pub(crate) json_output: bool,
    pub(crate) output_path: Option<String>,
    pub(crate) presets: Vec<String>,
    pub(crate) include_scorecard: bool,
    pub(crate) fail_on_command: bool,
    pub(crate) fail_fast: bool,
}

impl Default for BenchmarkRunSuiteOptions {
    fn default() -> Self {
        Self {
            json_output: false,
            output_path: None,
            presets: Vec::new(),
            include_scorecard: true,
            fail_on_command: false,
            fail_fast: false,
        }
    }
}

impl Default for BenchmarkRunOptions {
    fn default() -> Self {
        Self {
            json_output: false,
            output_path: None,
            preset: None,
            suite: DEFAULT_BENCHMARK_SUITE.to_string(),
            case_name: DEFAULT_BENCHMARK_RUN_CASE.to_string(),
            command: None,
            notes: None,
            include_scorecard: true,
            timeout_seconds: DEFAULT_BENCHMARK_TIMEOUT_SECONDS,
            fail_on_command: false,
        }
    }
}

impl Default for BenchmarkRecordOptions {
    fn default() -> Self {
        Self {
            json_output: false,
            output_path: None,
            suite: DEFAULT_BENCHMARK_SUITE.to_string(),
            case_name: DEFAULT_BENCHMARK_CASE.to_string(),
            commands: Vec::new(),
            notes: None,
            include_scorecard: true,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkRunArtifact {
    pub(crate) artifact: Value,
    pub(crate) relative_path: String,
    pub(crate) execution: BenchmarkCommandExecution,
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkCommandExecution {
    pub(crate) command: String,
    pub(crate) status: &'static str,
    pub(crate) exit_code: Option<i32>,
    pub(crate) timed_out: bool,
    pub(crate) duration_ms: u128,
    pub(crate) stdout_chars: usize,
    pub(crate) stderr_chars: usize,
    pub(crate) stdout_sample: String,
    pub(crate) stderr_sample: String,
    pub(crate) error: Option<String>,
}

pub(crate) fn handle_benchmark_run(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    args: &[String],
) -> Result<String> {
    let options = parse_benchmark_run_options(args)?;
    let run = execute_benchmark_run_artifact(workspace, config, registry, &options)?;
    let artifact_output = serde_json::to_string_pretty(&run.artifact)?;
    let output = if options.json_output {
        artifact_output
    } else {
        format_benchmark_artifact_text(
            workspace,
            "deepcli benchmark run",
            &run.relative_path,
            &run.artifact,
        )
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_command && run.execution.status != "passed" {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

pub(crate) fn execute_benchmark_run_artifact(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    options: &BenchmarkRunOptions,
) -> Result<BenchmarkRunArtifact> {
    let command = options
        .command
        .as_deref()
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "/benchmark run requires `--preset <name>`, `--command <cmd>`, or `-- <cmd>`"
            )
        })?;
    let created_at = Utc::now();
    let (artifact_path, relative_path) =
        unique_benchmark_artifact_path(workspace, created_at, &options.suite, &options.case_name);
    if let Some(parent) = artifact_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if options.include_scorecard {
        let placeholder = json!({
            "schema": BENCHMARK_ARTIFACT_SCHEMA,
            "createdAt": created_at.to_rfc3339(),
            "artifactPath": relative_path,
            "status": "pending",
        });
        fs::write(&artifact_path, serde_json::to_string_pretty(&placeholder)?)
            .with_context(|| format!("failed to write {}", artifact_path.display()))?;
    }
    let execution = run_benchmark_shell_command(workspace, command, options.timeout_seconds);
    let artifact = build_benchmark_run_json(
        workspace,
        config,
        registry,
        options,
        &execution,
        created_at,
        &relative_path,
    );
    let artifact_output = serde_json::to_string_pretty(&artifact)?;
    fs::write(&artifact_path, &artifact_output)
        .with_context(|| format!("failed to write {}", artifact_path.display()))?;
    Ok(BenchmarkRunArtifact {
        artifact,
        relative_path,
        execution,
    })
}

pub(crate) fn handle_benchmark_run_suite(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    args: &[String],
) -> Result<String> {
    let options = parse_benchmark_run_suite_options(args)?;
    let preset_names = benchmark_run_suite_preset_names(&options)?;
    let mut runs = Vec::new();
    let mut stopped_early = false;

    for preset_name in preset_names {
        let run_options = benchmark_run_options_for_suite_preset(&preset_name, &options)?;
        let run = execute_benchmark_run_artifact(workspace, config, registry, &run_options)?;
        let failed = run.execution.status != "passed";
        runs.push(run);
        if failed && options.fail_fast {
            stopped_early = true;
            break;
        }
    }

    let artifacts = load_benchmark_artifacts(workspace)?;
    let benchmark_status = build_benchmark_status_report(workspace, &artifacts, Utc::now());
    let output = if options.json_output {
        format_benchmark_run_suite_json(
            workspace,
            &options,
            &runs,
            stopped_early,
            &benchmark_status,
        )?
    } else {
        format_benchmark_run_suite_text(
            workspace,
            &options,
            &runs,
            stopped_early,
            &benchmark_status,
        )
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_command && benchmark_run_suite_status(&runs) != "passed" {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

fn parse_benchmark_run_suite_options(args: &[String]) -> Result<BenchmarkRunSuiteOptions> {
    let mut options = BenchmarkRunSuiteOptions::default();
    let mut index = 0;
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
            "--preset" => {
                let name = required_arg(args, index + 1, "benchmark preset")?;
                push_benchmark_run_suite_preset(&mut options.presets, name)?;
                index += 2;
            }
            value if value.starts_with("--preset=") => {
                push_benchmark_run_suite_preset(
                    &mut options.presets,
                    value.trim_start_matches("--preset="),
                )?;
                index += 1;
            }
            "--presets" => {
                let raw = required_arg(args, index + 1, "benchmark presets")?;
                push_benchmark_run_suite_presets(&mut options.presets, raw)?;
                index += 2;
            }
            value if value.starts_with("--presets=") => {
                push_benchmark_run_suite_presets(
                    &mut options.presets,
                    value.trim_start_matches("--presets="),
                )?;
                index += 1;
            }
            "--scorecard" => {
                options.include_scorecard = true;
                index += 1;
            }
            "--no-scorecard" => {
                options.include_scorecard = false;
                index += 1;
            }
            "--fail-on-command" | "--strict" => {
                options.fail_on_command = true;
                index += 1;
            }
            "--fail-fast" => {
                options.fail_fast = true;
                index += 1;
            }
            value => bail!("unsupported /benchmark run-suite option `{value}`"),
        }
    }
    Ok(options)
}

pub(crate) fn push_benchmark_run_suite_presets(target: &mut Vec<String>, raw: &str) -> Result<()> {
    for name in raw.split(',') {
        push_benchmark_run_suite_preset(target, name)?;
    }
    Ok(())
}

pub(crate) fn push_benchmark_run_suite_preset(target: &mut Vec<String>, raw: &str) -> Result<()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("benchmark preset must not be empty");
    }
    let preset = benchmark_preset_by_name(trimmed)?;
    if !target.iter().any(|name| name == preset.name) {
        target.push(preset.name.to_string());
    }
    Ok(())
}

pub(crate) fn benchmark_run_suite_preset_names(
    options: &BenchmarkRunSuiteOptions,
) -> Result<Vec<String>> {
    if options.presets.is_empty() {
        return Ok(DEFAULT_BENCHMARK_RUN_SUITE_PRESETS
            .iter()
            .map(|preset| (*preset).to_string())
            .collect());
    }
    let mut names = Vec::new();
    for preset_name in &options.presets {
        let preset = benchmark_preset_by_name(preset_name)?;
        if !names.iter().any(|name| name == preset.name) {
            names.push(preset.name.to_string());
        }
    }
    Ok(names)
}

pub(crate) fn benchmark_run_options_for_suite_preset(
    preset_name: &str,
    suite_options: &BenchmarkRunSuiteOptions,
) -> Result<BenchmarkRunOptions> {
    let mut options = BenchmarkRunOptions {
        json_output: true,
        preset: Some(preset_name.to_string()),
        include_scorecard: suite_options.include_scorecard,
        fail_on_command: false,
        ..BenchmarkRunOptions::default()
    };
    apply_benchmark_run_preset(&mut options, false, false, false, false)?;
    Ok(options)
}

pub(crate) fn benchmark_run_suite_status(runs: &[BenchmarkRunArtifact]) -> &'static str {
    if runs.is_empty() {
        return "empty";
    }
    if runs.iter().any(|run| run.execution.status == "failed") {
        return "failed";
    }
    if runs.iter().any(|run| run.execution.status == "timeout") {
        return "timeout";
    }
    if runs.iter().all(|run| run.execution.status == "passed") {
        "passed"
    } else {
        "mixed"
    }
}

fn format_benchmark_run_suite_json(
    workspace: &Path,
    options: &BenchmarkRunSuiteOptions,
    runs: &[BenchmarkRunArtifact],
    stopped_early: bool,
    benchmark_status: &BenchmarkStatusReport,
) -> Result<String> {
    let next_actions = benchmark_run_suite_next_actions(runs, benchmark_status);
    let checklist = benchmark_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": BENCHMARK_SUITE_SCHEMA,
        "status": benchmark_run_suite_status(runs),
        "workspace": workspace.display().to_string(),
        "presetCount": runs.len(),
        "requestedPresets": benchmark_run_suite_preset_names(options)?,
        "passedCount": runs.iter().filter(|run| run.execution.status == "passed").count(),
        "failedCount": runs.iter().filter(|run| run.execution.status == "failed").count(),
        "timeoutCount": runs.iter().filter(|run| run.execution.status == "timeout").count(),
        "stoppedEarly": stopped_early,
        "failFast": options.fail_fast,
        "failOnCommand": options.fail_on_command,
        "artifacts": runs.iter().map(benchmark_run_suite_artifact_json).collect::<Vec<_>>(),
        "benchmarkStatus": round_benchmark_status_json(benchmark_status),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": format_benchmark_run_suite_text(workspace, options, runs, stopped_early, benchmark_status),
    }))?)
}

pub(crate) fn benchmark_run_suite_artifact_json(run: &BenchmarkRunArtifact) -> Value {
    json!({
        "artifactPath": run.relative_path,
        "createdAt": run.artifact.get("createdAt").cloned().unwrap_or(Value::Null),
        "suite": run.artifact.get("suite").cloned().unwrap_or(Value::Null),
        "case": run.artifact.get("case").cloned().unwrap_or(Value::Null),
        "preset": run.artifact.get("preset").cloned().unwrap_or(Value::Null),
        "status": run.execution.status,
        "exitCode": run.execution.exit_code,
        "timedOut": run.execution.timed_out,
        "durationMs": run.execution.duration_ms,
        "stdoutChars": run.execution.stdout_chars,
        "stderrChars": run.execution.stderr_chars,
    })
}

fn format_benchmark_run_suite_text(
    workspace: &Path,
    options: &BenchmarkRunSuiteOptions,
    runs: &[BenchmarkRunArtifact],
    stopped_early: bool,
    benchmark_status: &BenchmarkStatusReport,
) -> String {
    let mut lines = vec![
        "deepcli benchmark run-suite".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("status: {}", benchmark_run_suite_status(runs)),
        format!("presets: {}", runs.len()),
        format!("fail-fast: {}", options.fail_fast),
        format!("stopped-early: {stopped_early}"),
    ];
    if runs.is_empty() {
        lines.push("results: none".to_string());
    } else {
        lines.push("results:".to_string());
        for run in runs {
            lines.push(format!(
                "  - {}: status={} exit={} duration={}ms artifact={}",
                run.artifact
                    .get("preset")
                    .and_then(Value::as_str)
                    .unwrap_or("<none>"),
                run.execution.status,
                run.execution
                    .exit_code
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "null".to_string()),
                run.execution.duration_ms,
                run.relative_path
            ));
        }
    }
    lines.push(format!(
        "benchmark status: {} ready={} artifacts={}",
        benchmark_status.status,
        benchmark_status.status == "ready",
        benchmark_status.artifact_count
    ));
    if !benchmark_status.gaps.is_empty() {
        lines.push("benchmark gaps:".to_string());
        lines.extend(benchmark_status.gaps.iter().map(|gap| format!("  - {gap}")));
    }
    lines.push("next actions:".to_string());
    lines.extend(
        benchmark_run_suite_next_actions(runs, benchmark_status)
            .into_iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn benchmark_run_suite_next_actions(
    runs: &[BenchmarkRunArtifact],
    benchmark_status: &BenchmarkStatusReport,
) -> Vec<String> {
    let mut actions = Vec::new();
    if benchmark_run_suite_status(runs) != "passed" {
        actions.push("deepcli benchmark run-suite --json --fail-on-command".to_string());
    }
    actions.push("deepcli benchmark status --json".to_string());
    actions.push("deepcli benchmark summary --json".to_string());
    actions.push("deepcli benchmark trends --json".to_string());
    if benchmark_status.status != "ready" {
        actions.push("deepcli benchmark presets --json".to_string());
    }
    actions.push("deepcli round --json".to_string());
    actions
}

fn parse_benchmark_run_options(args: &[String]) -> Result<BenchmarkRunOptions> {
    let mut options = BenchmarkRunOptions::default();
    let mut suite_explicit = false;
    let mut case_explicit = false;
    let mut notes_explicit = false;
    let mut timeout_explicit = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--" => {
                let command = args[index + 1..].join(" ");
                set_benchmark_command(&mut options.command, &command)?;
                break;
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
            "--preset" => {
                let name = required_arg(args, index + 1, "benchmark preset")?;
                set_benchmark_preset(&mut options.preset, name)?;
                index += 2;
            }
            value if value.starts_with("--preset=") => {
                set_benchmark_preset(&mut options.preset, value.trim_start_matches("--preset="))?;
                index += 1;
            }
            "--suite" => {
                options.suite =
                    parse_benchmark_label(required_arg(args, index + 1, "suite")?, "suite")?;
                suite_explicit = true;
                index += 2;
            }
            value if value.starts_with("--suite=") => {
                options.suite =
                    parse_benchmark_label(value.trim_start_matches("--suite="), "suite")?;
                suite_explicit = true;
                index += 1;
            }
            "--case" | "--name" => {
                options.case_name =
                    parse_benchmark_label(required_arg(args, index + 1, "case")?, "case")?;
                case_explicit = true;
                index += 2;
            }
            value if value.starts_with("--case=") => {
                options.case_name =
                    parse_benchmark_label(value.trim_start_matches("--case="), "case")?;
                case_explicit = true;
                index += 1;
            }
            value if value.starts_with("--name=") => {
                options.case_name =
                    parse_benchmark_label(value.trim_start_matches("--name="), "case")?;
                case_explicit = true;
                index += 1;
            }
            "--command" | "--cmd" => {
                let command = required_arg(args, index + 1, "benchmark command")?;
                set_benchmark_command(&mut options.command, command)?;
                index += 2;
            }
            value if value.starts_with("--command=") => {
                set_benchmark_command(
                    &mut options.command,
                    value.trim_start_matches("--command="),
                )?;
                index += 1;
            }
            value if value.starts_with("--cmd=") => {
                set_benchmark_command(&mut options.command, value.trim_start_matches("--cmd="))?;
                index += 1;
            }
            "--notes" | "--note" => {
                let notes = required_arg(args, index + 1, "notes")?;
                set_benchmark_notes(&mut options.notes, notes)?;
                notes_explicit = true;
                index += 2;
            }
            value if value.starts_with("--notes=") => {
                set_benchmark_notes(&mut options.notes, value.trim_start_matches("--notes="))?;
                notes_explicit = true;
                index += 1;
            }
            value if value.starts_with("--note=") => {
                set_benchmark_notes(&mut options.notes, value.trim_start_matches("--note="))?;
                notes_explicit = true;
                index += 1;
            }
            "--timeout" | "--timeout-seconds" => {
                options.timeout_seconds =
                    parse_benchmark_timeout(required_arg(args, index + 1, "timeout seconds")?)?;
                timeout_explicit = true;
                index += 2;
            }
            value if value.starts_with("--timeout=") => {
                options.timeout_seconds =
                    parse_benchmark_timeout(value.trim_start_matches("--timeout="))?;
                timeout_explicit = true;
                index += 1;
            }
            value if value.starts_with("--timeout-seconds=") => {
                options.timeout_seconds =
                    parse_benchmark_timeout(value.trim_start_matches("--timeout-seconds="))?;
                timeout_explicit = true;
                index += 1;
            }
            "--scorecard" => {
                options.include_scorecard = true;
                index += 1;
            }
            "--no-scorecard" => {
                options.include_scorecard = false;
                index += 1;
            }
            "--fail-on-command" | "--strict" => {
                options.fail_on_command = true;
                index += 1;
            }
            value => bail!("unsupported /benchmark run option `{value}`"),
        }
    }
    apply_benchmark_run_preset(
        &mut options,
        suite_explicit,
        case_explicit,
        notes_explicit,
        timeout_explicit,
    )?;
    Ok(options)
}

fn set_benchmark_preset(target: &mut Option<String>, raw: &str) -> Result<()> {
    if target.is_some() {
        bail!("multiple benchmark presets were provided");
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("benchmark preset must not be empty");
    }
    *target = Some(trimmed.to_ascii_lowercase());
    Ok(())
}

fn apply_benchmark_run_preset(
    options: &mut BenchmarkRunOptions,
    suite_explicit: bool,
    case_explicit: bool,
    notes_explicit: bool,
    timeout_explicit: bool,
) -> Result<()> {
    let Some(name) = options.preset.clone() else {
        return Ok(());
    };
    let preset = benchmark_preset_by_name(&name)?;
    if options.command.is_some() {
        bail!("`--preset` cannot be combined with `--command` or `-- <cmd>`");
    }
    options.preset = Some(preset.name.to_string());
    options.command = Some(redact_sensitive_text(preset.command));
    if !suite_explicit {
        options.suite = preset.suite.to_string();
    }
    if !case_explicit {
        options.case_name = preset.case_name.to_string();
    }
    if !notes_explicit {
        options.notes = Some(redact_sensitive_text(preset.summary));
    }
    if !timeout_explicit {
        options.timeout_seconds = preset.timeout_seconds;
    }
    Ok(())
}

fn set_benchmark_command(command: &mut Option<String>, raw: &str) -> Result<()> {
    if command.is_some() {
        bail!("multiple benchmark commands were provided");
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("benchmark command must not be empty");
    }
    *command = Some(redact_sensitive_text(trimmed));
    Ok(())
}

fn parse_benchmark_timeout(raw: &str) -> Result<u64> {
    let parsed = raw
        .parse::<u64>()
        .with_context(|| format!("timeout seconds must be a positive integer, got `{raw}`"))?;
    if parsed == 0 {
        bail!("timeout seconds must be greater than 0");
    }
    if parsed > 86_400 {
        bail!("timeout seconds must be 86400 or less");
    }
    Ok(parsed)
}

pub(crate) fn handle_benchmark_record(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    args: &[String],
) -> Result<String> {
    let options = parse_benchmark_record_options(args)?;
    let created_at = Utc::now();
    let (artifact_path, relative_path) =
        unique_benchmark_artifact_path(workspace, created_at, &options.suite, &options.case_name);
    if let Some(parent) = artifact_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if options.include_scorecard {
        let placeholder = json!({
            "schema": BENCHMARK_ARTIFACT_SCHEMA,
            "createdAt": created_at.to_rfc3339(),
            "artifactPath": relative_path,
            "status": "pending",
        });
        fs::write(&artifact_path, serde_json::to_string_pretty(&placeholder)?)
            .with_context(|| format!("failed to write {}", artifact_path.display()))?;
    }
    let artifact = build_benchmark_record_json(
        workspace,
        config,
        registry,
        &options,
        created_at,
        &relative_path,
    );
    let artifact_output = serde_json::to_string_pretty(&artifact)?;
    fs::write(&artifact_path, &artifact_output)
        .with_context(|| format!("failed to write {}", artifact_path.display()))?;
    let output = if options.json_output {
        artifact_output
    } else {
        format_benchmark_artifact_text(
            workspace,
            "deepcli benchmark record",
            &relative_path,
            &artifact,
        )
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_benchmark_record_options(args: &[String]) -> Result<BenchmarkRecordOptions> {
    let mut options = BenchmarkRecordOptions::default();
    let mut index = 0;
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
            "--suite" => {
                options.suite =
                    parse_benchmark_label(required_arg(args, index + 1, "suite")?, "suite")?;
                index += 2;
            }
            value if value.starts_with("--suite=") => {
                options.suite =
                    parse_benchmark_label(value.trim_start_matches("--suite="), "suite")?;
                index += 1;
            }
            "--case" | "--name" => {
                options.case_name =
                    parse_benchmark_label(required_arg(args, index + 1, "case")?, "case")?;
                index += 2;
            }
            value if value.starts_with("--case=") => {
                options.case_name =
                    parse_benchmark_label(value.trim_start_matches("--case="), "case")?;
                index += 1;
            }
            value if value.starts_with("--name=") => {
                options.case_name =
                    parse_benchmark_label(value.trim_start_matches("--name="), "case")?;
                index += 1;
            }
            "--command" | "--cmd" => {
                let command = required_arg(args, index + 1, "benchmark command")?;
                options.commands.push(redact_sensitive_text(command));
                index += 2;
            }
            value if value.starts_with("--command=") => {
                options.commands.push(redact_sensitive_text(
                    value.trim_start_matches("--command="),
                ));
                index += 1;
            }
            value if value.starts_with("--cmd=") => {
                options
                    .commands
                    .push(redact_sensitive_text(value.trim_start_matches("--cmd=")));
                index += 1;
            }
            "--notes" | "--note" => {
                let notes = required_arg(args, index + 1, "notes")?;
                set_benchmark_notes(&mut options.notes, notes)?;
                index += 2;
            }
            value if value.starts_with("--notes=") => {
                set_benchmark_notes(&mut options.notes, value.trim_start_matches("--notes="))?;
                index += 1;
            }
            value if value.starts_with("--note=") => {
                set_benchmark_notes(&mut options.notes, value.trim_start_matches("--note="))?;
                index += 1;
            }
            "--scorecard" => {
                options.include_scorecard = true;
                index += 1;
            }
            "--no-scorecard" => {
                options.include_scorecard = false;
                index += 1;
            }
            value => bail!("unsupported /benchmark record option `{value}`"),
        }
    }
    Ok(options)
}

fn parse_benchmark_label(raw: &str, name: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("--{name} requires a non-empty value");
    }
    Ok(redact_sensitive_text(trimmed))
}

fn set_benchmark_notes(notes: &mut Option<String>, raw: &str) -> Result<()> {
    if notes.is_some() {
        bail!("multiple notes were provided");
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("--notes requires a non-empty value");
    }
    *notes = Some(redact_sensitive_text(trimmed));
    Ok(())
}

fn build_benchmark_record_json(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    options: &BenchmarkRecordOptions,
    created_at: DateTime<Utc>,
    relative_path: &str,
) -> Value {
    let next_actions = benchmark_artifact_next_actions();
    let scorecard = if options.include_scorecard {
        let report = build_scorecard_report(workspace, config, registry);
        scorecard_summary_json(&report)
    } else {
        Value::Null
    };
    benchmark_value_with_action_checklist(json!({
        "schema": BENCHMARK_ARTIFACT_SCHEMA,
        "createdAt": created_at.to_rfc3339(),
        "artifactPath": relative_path,
        "workspace": {
            "name": workspace
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string()),
            "path": ".",
        },
        "version": {
            "package": "deepcli",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "suite": options.suite,
        "case": options.case_name,
        "notes": options.notes,
        "declaredCommands": options.commands,
        "execution": {
            "mode": "record_only",
            "ranByDeepcli": false,
            "status": "recorded",
            "reason": "benchmark record stores local evidence and declared commands; use /test, /preflight, or an explicit shell command to execute workloads",
        },
        "gitStatus": benchmark_git_status_json(workspace),
        "scorecard": scorecard,
        "nextActions": next_actions,
    }))
}

fn build_benchmark_run_json(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    options: &BenchmarkRunOptions,
    execution: &BenchmarkCommandExecution,
    created_at: DateTime<Utc>,
    relative_path: &str,
) -> Value {
    let next_actions = benchmark_artifact_next_actions();
    let scorecard = if options.include_scorecard {
        let report = build_scorecard_report(workspace, config, registry);
        scorecard_summary_json(&report)
    } else {
        Value::Null
    };
    benchmark_value_with_action_checklist(json!({
        "schema": BENCHMARK_ARTIFACT_SCHEMA,
        "createdAt": created_at.to_rfc3339(),
        "artifactPath": relative_path,
        "workspace": {
            "name": workspace
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string()),
            "path": ".",
        },
        "version": {
            "package": "deepcli",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "suite": options.suite,
        "case": options.case_name,
        "preset": options.preset,
        "notes": options.notes,
        "declaredCommands": [execution.command.clone()],
        "execution": {
            "mode": "command",
            "ranByDeepcli": true,
            "status": execution.status,
            "timeoutSeconds": options.timeout_seconds,
            "commandCount": 1,
            "commands": [{
                "command": execution.command,
                "status": execution.status,
                "exitCode": execution.exit_code,
                "timedOut": execution.timed_out,
                "durationMs": execution.duration_ms,
                "stdoutChars": execution.stdout_chars,
                "stderrChars": execution.stderr_chars,
                "stdoutSample": execution.stdout_sample,
                "stderrSample": execution.stderr_sample,
                "error": execution.error,
            }],
        },
        "gitStatus": benchmark_git_status_json(workspace),
        "scorecard": scorecard,
        "nextActions": next_actions,
    }))
}

fn benchmark_artifact_next_actions() -> Vec<String> {
    vec![
        "deepcli benchmark list --json".to_string(),
        "deepcli benchmark status --json".to_string(),
        "deepcli benchmark summary --json".to_string(),
        "deepcli benchmark show latest --json".to_string(),
        "deepcli scorecard --json".to_string(),
    ]
}

fn run_benchmark_shell_command(
    workspace: &Path,
    command: &str,
    timeout_seconds: u64,
) -> BenchmarkCommandExecution {
    let started = Instant::now();
    let sanitized_command = redact_sensitive_text(command);
    let mut child = match ProcessCommand::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return BenchmarkCommandExecution {
                command: sanitized_command,
                status: "failed",
                exit_code: None,
                timed_out: false,
                duration_ms: started.elapsed().as_millis(),
                stdout_chars: 0,
                stderr_chars: 0,
                stdout_sample: String::new(),
                stderr_sample: String::new(),
                error: Some(error.to_string()),
            };
        }
    };

    let timeout = Duration::from_secs(timeout_seconds);
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let output = child.wait_with_output();
                let duration_ms = started.elapsed().as_millis();
                return match output {
                    Ok(output) => benchmark_execution_from_output(
                        sanitized_command,
                        "timeout",
                        true,
                        duration_ms,
                        output,
                        None,
                    ),
                    Err(error) => BenchmarkCommandExecution {
                        command: sanitized_command,
                        status: "timeout",
                        exit_code: None,
                        timed_out: true,
                        duration_ms,
                        stdout_chars: 0,
                        stderr_chars: 0,
                        stdout_sample: String::new(),
                        stderr_sample: String::new(),
                        error: Some(error.to_string()),
                    },
                };
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                let _ = child.kill();
                let duration_ms = started.elapsed().as_millis();
                return BenchmarkCommandExecution {
                    command: sanitized_command,
                    status: "failed",
                    exit_code: None,
                    timed_out: false,
                    duration_ms,
                    stdout_chars: 0,
                    stderr_chars: 0,
                    stdout_sample: String::new(),
                    stderr_sample: String::new(),
                    error: Some(error.to_string()),
                };
            }
        }
    }

    let duration_ms = started.elapsed().as_millis();
    match child.wait_with_output() {
        Ok(output) => {
            let status = if output.status.success() {
                "passed"
            } else {
                "failed"
            };
            benchmark_execution_from_output(
                sanitized_command,
                status,
                false,
                duration_ms,
                output,
                None,
            )
        }
        Err(error) => BenchmarkCommandExecution {
            command: sanitized_command,
            status: "failed",
            exit_code: None,
            timed_out: false,
            duration_ms,
            stdout_chars: 0,
            stderr_chars: 0,
            stdout_sample: String::new(),
            stderr_sample: String::new(),
            error: Some(error.to_string()),
        },
    }
}

fn benchmark_execution_from_output(
    command: String,
    status: &'static str,
    timed_out: bool,
    duration_ms: u128,
    output: std::process::Output,
    error: Option<String>,
) -> BenchmarkCommandExecution {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    BenchmarkCommandExecution {
        command,
        status,
        exit_code: output.status.code(),
        timed_out,
        duration_ms,
        stdout_chars: stdout.chars().count(),
        stderr_chars: stderr.chars().count(),
        stdout_sample: truncate_benchmark_output(&redact_sensitive_text(&stdout)),
        stderr_sample: truncate_benchmark_output(&redact_sensitive_text(&stderr)),
        error,
    }
}

fn truncate_benchmark_output(value: &str) -> String {
    let count = value.chars().count();
    if count <= BENCHMARK_OUTPUT_SAMPLE_CHARS {
        return value.to_string();
    }
    let kept = value
        .chars()
        .take(BENCHMARK_OUTPUT_SAMPLE_CHARS)
        .collect::<String>();
    format!(
        "{kept}\n...[truncated {} chars]",
        count - BENCHMARK_OUTPUT_SAMPLE_CHARS
    )
}

fn benchmark_git_status_json(workspace: &Path) -> Value {
    match ProcessCommand::new("git")
        .arg("status")
        .arg("--short")
        .current_dir(workspace)
        .output()
    {
        Ok(output) if output.status.success() => {
            let raw = String::from_utf8_lossy(&output.stdout);
            let lines = raw.lines().map(redact_sensitive_text).collect::<Vec<_>>();
            let changed_paths = lines
                .iter()
                .filter(|line| !line.trim_start().starts_with("?? "))
                .count();
            let untracked_paths = lines
                .iter()
                .filter(|line| line.trim_start().starts_with("?? "))
                .count();
            json!({
                "available": true,
                "clean": lines.is_empty(),
                "changedPaths": changed_paths,
                "untrackedPaths": untracked_paths,
                "sample": lines.into_iter().take(20).collect::<Vec<_>>(),
            })
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            json!({
                "available": false,
                "clean": Value::Null,
                "error": redact_sensitive_text(stderr.trim()),
            })
        }
        Err(error) => json!({
            "available": false,
            "clean": Value::Null,
            "error": error.to_string(),
        }),
    }
}

fn unique_benchmark_artifact_path(
    workspace: &Path,
    created_at: DateTime<Utc>,
    suite: &str,
    case_name: &str,
) -> (PathBuf, String) {
    let timestamp = created_at.format("%Y%m%dT%H%M%SZ").to_string();
    let suite_slug = benchmark_slug(suite, DEFAULT_BENCHMARK_SUITE);
    let case_slug = benchmark_slug(case_name, DEFAULT_BENCHMARK_CASE);
    for suffix in 0..1000 {
        let file_name = if suffix == 0 {
            format!("{timestamp}-{suite_slug}-{case_slug}.json")
        } else {
            format!("{timestamp}-{suite_slug}-{case_slug}-{suffix}.json")
        };
        let relative_path = format!(".deepcli/benchmarks/{file_name}");
        let path = workspace.join(&relative_path);
        if !path.exists() {
            return (path, relative_path);
        }
    }
    let relative_path =
        format!(".deepcli/benchmarks/{timestamp}-{suite_slug}-{case_slug}-overflow.json");
    (workspace.join(&relative_path), relative_path)
}

pub(crate) fn benchmark_slug(raw: &str, fallback: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for character in raw.chars().flat_map(char::to_lowercase) {
        let next = if character.is_ascii_alphanumeric() {
            Some(character)
        } else if character == '-' || character == '_' || character.is_whitespace() {
            Some('-')
        } else {
            None
        };
        let Some(next) = next else {
            continue;
        };
        if next == '-' {
            if slug.is_empty() || last_was_dash {
                continue;
            }
            last_was_dash = true;
        } else {
            last_was_dash = false;
        }
        slug.push(next);
        if slug.len() >= 64 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    }
}
