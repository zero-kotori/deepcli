use super::*;
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const BENCHMARK_ARTIFACT_SCHEMA: &str = schema_ids::BENCHMARK_RECORD_V1;

const BENCHMARK_CARGO_TEST_REMEDIATION_ACTION: &str =
    "deepcli benchmark run --preset cargo-test --json --fail-on-command";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkListOptions {
    json_output: bool,
    output_path: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkCleanupOptions {
    json_output: bool,
    output_path: Option<String>,
    force: bool,
    keep: Option<usize>,
    older_than_days: Option<i64>,
    all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BenchmarkShowOptions {
    json_output: bool,
    output_path: Option<String>,
    target: String,
}

impl Default for BenchmarkShowOptions {
    fn default() -> Self {
        Self {
            json_output: false,
            output_path: None,
            target: "latest".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkArtifact {
    pub(crate) relative_path: String,
    pub(crate) created_at: Option<DateTime<Utc>>,
    pub(crate) modified_at: Option<DateTime<Utc>>,
    pub(crate) value: Value,
}

pub(crate) fn benchmark_value_with_action_checklist(mut value: Value) -> Value {
    let actions = value
        .get("nextActions")
        .and_then(Value::as_array)
        .map(|actions| {
            actions
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let checklist = benchmark_action_checklist(&actions);
    let summary = benchmark_artifact_detail_summary_json(&value, &checklist);
    value["checklist"] = Value::Array(checklist);
    value["summary"] = summary;
    value
}

fn benchmark_artifact_detail_summary_json(value: &Value, checklist: &[Value]) -> Value {
    let execution = value.get("execution");
    let command_count = execution
        .and_then(|execution| execution.get("commandCount"))
        .and_then(Value::as_u64)
        .or_else(|| {
            execution
                .and_then(|execution| execution.get("commands"))
                .and_then(Value::as_array)
                .map(|commands| commands.len() as u64)
        })
        .or_else(|| {
            value
                .get("declaredCommands")
                .and_then(Value::as_array)
                .map(|commands| commands.len() as u64)
        })
        .unwrap_or(0);
    let recommended_action = checklist
        .first()
        .and_then(|item| item.get("command"))
        .cloned()
        .unwrap_or(Value::Null);
    let recommended_action_label = checklist
        .first()
        .and_then(|item| item.get("label"))
        .cloned()
        .unwrap_or(Value::Null);

    json!({
        "status": benchmark_artifact_status(value),
        "artifactPath": value.get("artifactPath").cloned().unwrap_or(Value::Null),
        "createdAt": value.get("createdAt").cloned().unwrap_or(Value::Null),
        "suite": value.get("suite").cloned().unwrap_or(Value::Null),
        "case": value.get("case").cloned().unwrap_or(Value::Null),
        "preset": value.get("preset").cloned().unwrap_or(Value::Null),
        "mode": execution
            .and_then(|execution| execution.get("mode"))
            .cloned()
            .unwrap_or(Value::Null),
        "ranByDeepcli": execution
            .and_then(|execution| execution.get("ranByDeepcli"))
            .cloned()
            .unwrap_or(Value::Null),
        "commandCount": command_count,
        "durationMs": benchmark_artifact_duration_ms(value)
            .map(Value::from)
            .unwrap_or(Value::Null),
        "recommendedAction": recommended_action,
        "recommendedActionLabel": recommended_action_label,
    })
}

pub(crate) fn handle_benchmark_list(workspace: &Path, args: &[String]) -> Result<String> {
    let options = parse_benchmark_list_options(args)?;
    let mut artifacts = load_benchmark_artifacts(workspace)?;
    if let Some(limit) = options.limit {
        artifacts.truncate(limit);
    }
    let output = if options.json_output {
        format_benchmark_list_json(workspace, &artifacts)?
    } else {
        format_benchmark_list_text(workspace, &artifacts)
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_benchmark_list_options(args: &[String]) -> Result<BenchmarkListOptions> {
    let mut options = BenchmarkListOptions::default();
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
            "--limit" => {
                options.limit = Some(parse_positive_usize(
                    required_arg(args, index + 1, "limit")?,
                    "limit",
                )?);
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                options.limit = Some(parse_positive_usize(
                    value.trim_start_matches("--limit="),
                    "limit",
                )?);
                index += 1;
            }
            value => bail!("unsupported /benchmark list option `{value}`"),
        }
    }
    Ok(options)
}

pub(crate) fn handle_benchmark_cleanup(workspace: &Path, args: &[String]) -> Result<String> {
    let options = parse_benchmark_cleanup_options(args)?;
    let artifacts = load_benchmark_artifacts(workspace)?;
    let candidates = benchmark_cleanup_candidates(&artifacts, &options, Utc::now());
    let mut deleted = Vec::new();
    if options.force {
        for artifact in &candidates {
            let path = benchmark_artifact_workspace_path(workspace, artifact)?;
            if path.is_file() {
                fs::remove_file(&path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
                deleted.push(artifact.relative_path.clone());
            }
        }
    }
    let output = if options.json_output {
        format_benchmark_cleanup_json(workspace, &options, &artifacts, &candidates, &deleted)?
    } else {
        format_benchmark_cleanup_text(workspace, &options, &artifacts, &candidates, &deleted)
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_benchmark_cleanup_options(args: &[String]) -> Result<BenchmarkCleanupOptions> {
    let mut options = BenchmarkCleanupOptions::default();
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
            "--dry-run" => {
                options.force = false;
                index += 1;
            }
            "--force" | "--delete" => {
                options.force = true;
                index += 1;
            }
            "--keep" => {
                options.keep = Some(parse_nonnegative_usize(
                    required_arg(args, index + 1, "keep count")?,
                    "keep",
                )?);
                index += 2;
            }
            value if value.starts_with("--keep=") => {
                options.keep = Some(parse_nonnegative_usize(
                    value.trim_start_matches("--keep="),
                    "keep",
                )?);
                index += 1;
            }
            "--older-than-days" | "--older-than" => {
                options.older_than_days = Some(parse_nonnegative_i64(
                    required_arg(args, index + 1, "days")?,
                    "older-than-days",
                )?);
                index += 2;
            }
            value if value.starts_with("--older-than-days=") => {
                options.older_than_days = Some(parse_nonnegative_i64(
                    value.trim_start_matches("--older-than-days="),
                    "older-than-days",
                )?);
                index += 1;
            }
            value if value.starts_with("--older-than=") => {
                options.older_than_days = Some(parse_nonnegative_i64(
                    value.trim_start_matches("--older-than="),
                    "older-than-days",
                )?);
                index += 1;
            }
            "--all" => {
                options.all = true;
                index += 1;
            }
            value => bail!("unsupported /benchmark clean option `{value}`"),
        }
    }
    Ok(options)
}

fn parse_nonnegative_usize(raw: &str, label: &str) -> Result<usize> {
    let value = raw
        .parse::<usize>()
        .with_context(|| format!("invalid {label} `{raw}`"))?;
    Ok(value)
}

fn parse_nonnegative_i64(raw: &str, label: &str) -> Result<i64> {
    let value = raw
        .parse::<i64>()
        .with_context(|| format!("invalid {label} `{raw}`"))?;
    if value < 0 {
        bail!("{label} must be zero or greater");
    }
    Ok(value)
}

fn benchmark_cleanup_keep_count(options: &BenchmarkCleanupOptions) -> usize {
    if options.all && options.keep.is_none() {
        0
    } else {
        options.keep.unwrap_or(20)
    }
}

fn benchmark_cleanup_candidates<'a>(
    artifacts: &'a [BenchmarkArtifact],
    options: &BenchmarkCleanupOptions,
    now: DateTime<Utc>,
) -> Vec<&'a BenchmarkArtifact> {
    let keep = benchmark_cleanup_keep_count(options);
    artifacts
        .iter()
        .enumerate()
        .filter(|(index, artifact)| {
            if *index < keep {
                return false;
            }
            if let Some(days) = options.older_than_days {
                return artifact
                    .created_at
                    .map(|created_at| now.signed_duration_since(created_at).num_days() >= days)
                    .unwrap_or(false);
            }
            true
        })
        .map(|(_, artifact)| artifact)
        .collect()
}

fn benchmark_artifact_workspace_path(
    workspace: &Path,
    artifact: &BenchmarkArtifact,
) -> Result<PathBuf> {
    let relative = artifact.relative_path.as_str();
    if !relative.starts_with(".deepcli/benchmarks/")
        || relative.contains("..")
        || relative.contains('\\')
    {
        bail!("benchmark artifact path is outside .deepcli/benchmarks: {relative}");
    }
    Ok(workspace.join(relative))
}

pub(crate) fn handle_benchmark_show(workspace: &Path, args: &[String]) -> Result<String> {
    let options = parse_benchmark_show_options(args)?;
    let artifact = resolve_benchmark_artifact(workspace, &options.target)?;
    let output = if options.json_output {
        serde_json::to_string_pretty(&benchmark_value_with_action_checklist(
            artifact.value.clone(),
        ))?
    } else {
        format_benchmark_artifact_text(
            workspace,
            "deepcli benchmark artifact",
            &artifact.relative_path,
            &artifact.value,
        )
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_benchmark_show_options(args: &[String]) -> Result<BenchmarkShowOptions> {
    let mut options = BenchmarkShowOptions::default();
    let mut target_set = false;
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
            value if value.starts_with('-') => {
                bail!("unsupported /benchmark show option `{value}`");
            }
            value => {
                if target_set {
                    bail!("multiple benchmark artifact names were provided");
                }
                options.target = value.to_string();
                target_set = true;
                index += 1;
            }
        }
    }
    Ok(options)
}

pub(crate) fn load_benchmark_artifacts(workspace: &Path) -> Result<Vec<BenchmarkArtifact>> {
    let dir = workspace.join(".deepcli/benchmarks");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut artifacts = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("schema").and_then(Value::as_str) != Some(BENCHMARK_ARTIFACT_SCHEMA) {
            continue;
        }
        let relative_path = path
            .strip_prefix(workspace)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let modified_at = entry.metadata()?.modified().ok().map(DateTime::<Utc>::from);
        artifacts.push(BenchmarkArtifact {
            relative_path,
            created_at: benchmark_artifact_created_at(&value),
            value,
            modified_at,
        });
    }
    artifacts.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.modified_at.cmp(&left.modified_at))
            .then_with(|| right.relative_path.cmp(&left.relative_path))
    });
    Ok(artifacts)
}

fn benchmark_artifact_created_at(value: &Value) -> Option<DateTime<Utc>> {
    value
        .get("createdAt")
        .and_then(Value::as_str)
        .and_then(|created_at| DateTime::parse_from_rfc3339(created_at).ok())
        .map(|created_at| created_at.with_timezone(&Utc))
}

fn resolve_benchmark_artifact(workspace: &Path, target: &str) -> Result<BenchmarkArtifact> {
    let artifacts = load_benchmark_artifacts(workspace)?;
    if target == "latest" {
        return artifacts.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!(
                "no benchmark artifacts found under .deepcli/benchmarks; run `{}`, `{}`, or `deepcli benchmark record` first",
                BENCHMARK_RUN_SUITE_REMEDIATION_ACTION,
                BENCHMARK_CARGO_TEST_REMEDIATION_ACTION
            )
        });
    }
    let normalized = target.trim();
    if normalized.is_empty()
        || normalized.contains("..")
        || normalized.contains('/')
        || normalized.contains('\\')
    {
        bail!("benchmark artifact name must be a file name under .deepcli/benchmarks");
    }
    let wanted = if normalized.ends_with(".json") {
        format!(".deepcli/benchmarks/{normalized}")
    } else {
        format!(".deepcli/benchmarks/{normalized}.json")
    };
    artifacts
        .into_iter()
        .find(|artifact| artifact.relative_path == wanted)
        .ok_or_else(|| anyhow::anyhow!("benchmark artifact `{target}` was not found"))
}

fn format_benchmark_list_json(workspace: &Path, artifacts: &[BenchmarkArtifact]) -> Result<String> {
    let next_actions = benchmark_list_next_actions(workspace);
    let checklist = benchmark_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::BENCHMARK_LIST_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "artifactCount": artifacts.len(),
        "summary": benchmark_list_summary_json(artifacts, &checklist),
        "artifacts": artifacts.iter().map(benchmark_artifact_summary_json).collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": checklist,
    }))?)
}

fn benchmark_list_summary_json(artifacts: &[BenchmarkArtifact], checklist: &[Value]) -> Value {
    let latest = artifacts.first();
    let recommended_action = checklist
        .first()
        .and_then(|item| item.get("command"))
        .and_then(Value::as_str)
        .map(Value::from)
        .unwrap_or(Value::Null);
    let recommended_action_label = checklist
        .first()
        .and_then(|item| item.get("label"))
        .and_then(Value::as_str)
        .map(Value::from)
        .unwrap_or(Value::Null);

    json!({
        "status": "ok",
        "artifactCount": artifacts.len(),
        "latestArtifactPath": latest
            .map(|artifact| Value::from(artifact.relative_path.clone()))
            .unwrap_or(Value::Null),
        "latestCreatedAt": latest
            .and_then(|artifact| artifact.value.get("createdAt"))
            .cloned()
            .unwrap_or(Value::Null),
        "latestSuite": latest
            .and_then(|artifact| artifact.value.get("suite"))
            .cloned()
            .unwrap_or(Value::Null),
        "latestCase": latest
            .and_then(|artifact| artifact.value.get("case"))
            .cloned()
            .unwrap_or(Value::Null),
        "latestPreset": latest
            .and_then(|artifact| artifact.value.get("preset"))
            .cloned()
            .unwrap_or(Value::Null),
        "latestStatus": latest
            .map(|artifact| Value::from(benchmark_artifact_status(&artifact.value)))
            .unwrap_or(Value::Null),
        "recommendedAction": recommended_action,
        "recommendedActionLabel": recommended_action_label,
    })
}

fn benchmark_list_next_actions(workspace: &Path) -> Vec<String> {
    let mut actions = vec![
        "deepcli benchmark presets --json".to_string(),
        "deepcli benchmark run-suite --json --fail-on-command".to_string(),
        "deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string(),
        "deepcli benchmark record --json".to_string(),
        "deepcli benchmark status --json".to_string(),
        "deepcli benchmark summary --json".to_string(),
        "deepcli benchmark trends --json".to_string(),
    ];
    actions.extend(sota_baseline_next_actions(workspace));
    actions.extend([
        "deepcli benchmark show latest --json".to_string(),
        "deepcli benchmark clean --dry-run --json".to_string(),
        "deepcli scorecard --json".to_string(),
    ]);
    actions
}

fn format_benchmark_cleanup_json(
    workspace: &Path,
    options: &BenchmarkCleanupOptions,
    artifacts: &[BenchmarkArtifact],
    candidates: &[&BenchmarkArtifact],
    deleted: &[String],
) -> Result<String> {
    let next_actions = benchmark_cleanup_next_actions(options, candidates.is_empty());
    let checklist = benchmark_action_checklist(&next_actions);
    let status = benchmark_cleanup_status(options, candidates, deleted);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::BENCHMARK_CLEANUP_V1,
        "status": status,
        "workspace": workspace.display().to_string(),
        "dryRun": !options.force,
        "force": options.force,
        "keep": benchmark_cleanup_keep_count(options),
        "olderThanDays": options.older_than_days,
        "all": options.all,
        "artifactCount": artifacts.len(),
        "candidateCount": candidates.len(),
        "deletedCount": deleted.len(),
        "summary": benchmark_cleanup_summary_json(
            options,
            artifacts.len(),
            candidates.len(),
            deleted.len(),
            status,
            &checklist,
        ),
        "candidates": candidates
            .iter()
            .map(|artifact| benchmark_artifact_summary_json(artifact))
            .collect::<Vec<_>>(),
        "deleted": deleted,
        "nextActions": next_actions,
        "checklist": checklist,
        "report": format_benchmark_cleanup_text(workspace, options, artifacts, candidates, deleted),
    }))?)
}

fn benchmark_cleanup_summary_json(
    options: &BenchmarkCleanupOptions,
    artifact_count: usize,
    candidate_count: usize,
    deleted_count: usize,
    status: &str,
    checklist: &[Value],
) -> Value {
    let recommended_action = checklist
        .first()
        .and_then(|item| item.get("command"))
        .and_then(Value::as_str)
        .map(Value::from)
        .unwrap_or(Value::Null);
    let recommended_action_label = checklist
        .first()
        .and_then(|item| item.get("label"))
        .and_then(Value::as_str)
        .map(Value::from)
        .unwrap_or(Value::Null);

    json!({
        "status": status,
        "dryRun": !options.force,
        "force": options.force,
        "artifactCount": artifact_count,
        "candidateCount": candidate_count,
        "deletedCount": deleted_count,
        "keep": benchmark_cleanup_keep_count(options),
        "olderThanDays": options.older_than_days,
        "all": options.all,
        "willDelete": options.force && candidate_count > 0,
        "recommendedAction": recommended_action,
        "recommendedActionLabel": recommended_action_label,
    })
}

fn benchmark_cleanup_status(
    options: &BenchmarkCleanupOptions,
    candidates: &[&BenchmarkArtifact],
    deleted: &[String],
) -> &'static str {
    if candidates.is_empty() {
        "empty"
    } else if options.force {
        if deleted.is_empty() {
            "unchanged"
        } else {
            "deleted"
        }
    } else {
        "planned"
    }
}

fn benchmark_artifact_summary_json(artifact: &BenchmarkArtifact) -> Value {
    json!({
        "artifactPath": artifact.relative_path,
        "createdAt": artifact.value.get("createdAt").cloned().unwrap_or(Value::Null),
        "modifiedAt": artifact.modified_at.map(|time| time.to_rfc3339()),
        "suite": artifact.value.get("suite").cloned().unwrap_or(Value::Null),
        "case": artifact.value.get("case").cloned().unwrap_or(Value::Null),
        "preset": artifact.value.get("preset").cloned().unwrap_or(Value::Null),
        "execution": benchmark_artifact_execution_summary_json(&artifact.value),
        "scorecard": artifact.value.get("scorecard").cloned().unwrap_or(Value::Null),
    })
}

fn benchmark_artifact_execution_summary_json(value: &Value) -> Value {
    let execution = value.get("execution").unwrap_or(&Value::Null);
    if !execution.is_object() {
        return Value::Null;
    }
    json!({
        "mode": execution.get("mode").cloned().unwrap_or(Value::Null),
        "status": execution.get("status").cloned().unwrap_or(Value::Null),
        "ranByDeepcli": execution.get("ranByDeepcli").cloned().unwrap_or(Value::Null),
        "durationMs": benchmark_artifact_duration_ms(value),
    })
}

pub(crate) fn artifact_string_field(value: &Value, key: &str, fallback: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn benchmark_artifact_status(value: &Value) -> &str {
    value
        .get("execution")
        .and_then(|execution| execution.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

pub(crate) fn benchmark_artifact_duration_ms(value: &Value) -> Option<u64> {
    let commands = value
        .get("execution")
        .and_then(|execution| execution.get("commands"))
        .and_then(Value::as_array)?;
    let mut total = 0u64;
    let mut found = false;
    for command in commands {
        if let Some(duration) = command.get("durationMs").and_then(Value::as_u64) {
            total = total.saturating_add(duration);
            found = true;
        }
    }
    found.then_some(total)
}

fn format_benchmark_list_text(workspace: &Path, artifacts: &[BenchmarkArtifact]) -> String {
    let mut lines = vec![
        "deepcli benchmark artifacts".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("count: {}", artifacts.len()),
    ];
    if artifacts.is_empty() {
        lines.push("artifacts: none".to_string());
        lines.push("next actions:".to_string());
        lines.extend(
            benchmark_list_next_actions(workspace)
                .into_iter()
                .map(|action| format!("  - {action}")),
        );
        return lines.join("\n");
    }
    lines.push("artifacts:".to_string());
    for artifact in artifacts {
        lines.push(format!("  - {}", artifact.relative_path));
        lines.push(format!(
            "    created: {}",
            artifact
                .value
                .get("createdAt")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>")
        ));
        lines.push(format!(
            "    suite: {}",
            artifact
                .value
                .get("suite")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>")
        ));
        lines.push(format!(
            "    case: {}",
            artifact
                .value
                .get("case")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>")
        ));
    }
    lines.push("next actions:".to_string());
    lines.extend(
        benchmark_list_next_actions(workspace)
            .into_iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn format_benchmark_cleanup_text(
    workspace: &Path,
    options: &BenchmarkCleanupOptions,
    artifacts: &[BenchmarkArtifact],
    candidates: &[&BenchmarkArtifact],
    deleted: &[String],
) -> String {
    let mut lines = vec![
        "deepcli benchmark cleanup".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("mode: {}", if options.force { "delete" } else { "dry-run" }),
        format!("artifacts: {}", artifacts.len()),
        format!("keep: {}", benchmark_cleanup_keep_count(options)),
        format!("candidates: {}", candidates.len()),
        format!("deleted: {}", deleted.len()),
    ];
    if let Some(days) = options.older_than_days {
        lines.push(format!("older-than-days: {days}"));
    }
    if candidates.is_empty() {
        lines.push("candidate artifacts: none".to_string());
    } else {
        lines.push("candidate artifacts:".to_string());
        for artifact in candidates {
            lines.push(format!("  - {}", artifact.relative_path));
            lines.push(format!(
                "    created: {} status={} preset={}",
                artifact
                    .value
                    .get("createdAt")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>"),
                benchmark_artifact_status(&artifact.value),
                benchmark_artifact_preset(&artifact.value).unwrap_or("<none>")
            ));
        }
    }
    if !deleted.is_empty() {
        lines.push("deleted artifacts:".to_string());
        lines.extend(deleted.iter().map(|path| format!("  - {path}")));
    }
    lines.push("next actions:".to_string());
    lines.extend(
        benchmark_cleanup_next_actions(options, candidates.is_empty())
            .into_iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn benchmark_cleanup_next_actions(options: &BenchmarkCleanupOptions, empty: bool) -> Vec<String> {
    if empty {
        return vec![
            "deepcli benchmark run-suite --json --fail-on-command".to_string(),
            "deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string(),
            "deepcli benchmark status --json".to_string(),
        ];
    }
    if options.force {
        vec![
            "deepcli benchmark status --json".to_string(),
            "deepcli benchmark summary --json".to_string(),
            "deepcli round --json".to_string(),
        ]
    } else {
        vec![
            benchmark_cleanup_force_command(options),
            "deepcli benchmark list --json".to_string(),
            "deepcli benchmark status --json".to_string(),
        ]
    }
}

fn benchmark_cleanup_force_command(options: &BenchmarkCleanupOptions) -> String {
    let mut command = vec![
        "deepcli".to_string(),
        "benchmark".to_string(),
        "clean".to_string(),
        "--force".to_string(),
    ];
    if options.all && options.keep.is_none() {
        command.push("--all".to_string());
    } else {
        command.push("--keep".to_string());
        command.push(benchmark_cleanup_keep_count(options).to_string());
    }
    if let Some(days) = options.older_than_days {
        command.push("--older-than-days".to_string());
        command.push(days.to_string());
    }
    command.join(" ")
}

pub(crate) fn format_benchmark_artifact_text(
    workspace: &Path,
    title: &str,
    relative_path: &str,
    artifact: &Value,
) -> String {
    let scorecard = artifact.get("scorecard").unwrap_or(&Value::Null);
    let mut lines = vec![
        title.to_string(),
        format!("workspace: {}", workspace.display()),
        format!("artifact: {relative_path}"),
        format!(
            "created: {}",
            artifact
                .get("createdAt")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>")
        ),
        format!(
            "suite: {}",
            artifact
                .get("suite")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>")
        ),
        format!(
            "case: {}",
            artifact
                .get("case")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>")
        ),
    ];
    if let Some(notes) = artifact.get("notes").and_then(Value::as_str) {
        lines.push(format!("notes: {notes}"));
    }
    if let Some(commands) = artifact.get("declaredCommands").and_then(Value::as_array) {
        if !commands.is_empty() {
            lines.push("declared commands:".to_string());
            for command in commands.iter().filter_map(Value::as_str) {
                lines.push(format!("  - {command}"));
            }
        }
    }
    if let Some(execution) = artifact.get("execution").and_then(Value::as_object) {
        let mode = execution
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let status = execution
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let ran_by_deepcli = execution
            .get("ranByDeepcli")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        lines.push(format!(
            "execution: mode={mode} status={status} ran_by_deepcli={ran_by_deepcli}"
        ));
        if let Some(commands) = execution.get("commands").and_then(Value::as_array) {
            for command in commands {
                let command_text = command
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                let command_status = command
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                let duration_ms = command
                    .get("durationMs")
                    .and_then(Value::as_u64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "?".to_string());
                let exit_code = command
                    .get("exitCode")
                    .and_then(Value::as_i64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "null".to_string());
                lines.push(format!(
                    "  - {command_status}: exit={exit_code} duration={duration_ms}ms command={command_text}"
                ));
            }
        }
    }
    if scorecard.is_object() {
        lines.push(format!(
            "scorecard: {} / {} ({}, {}%)",
            scorecard
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>"),
            scorecard
                .get("tier")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>"),
            scorecard
                .get("score")
                .and_then(Value::as_u64)
                .map(|score| score.to_string())
                .unwrap_or_else(|| "?".to_string()),
            scorecard
                .get("percent")
                .and_then(Value::as_u64)
                .map(|percent| percent.to_string())
                .unwrap_or_else(|| "?".to_string()),
        ));
    }
    lines.push("next actions:".to_string());
    lines.push("  - deepcli benchmark presets --json".to_string());
    lines.push("  - deepcli benchmark run-suite --json --fail-on-command".to_string());
    lines
        .push("  - deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string());
    lines.push("  - deepcli benchmark list --json".to_string());
    lines.push("  - deepcli benchmark status --json".to_string());
    lines.push("  - deepcli benchmark summary --json".to_string());
    lines.push("  - deepcli benchmark show latest --json".to_string());
    lines.push("  - deepcli scorecard --json".to_string());
    lines.join("\n")
}
