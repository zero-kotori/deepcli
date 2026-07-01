use super::*;
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub(crate) const DEFAULT_BENCHMARK_CURRENT_BASELINE_TEMPLATE_ACTION: &str =
    "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json";
pub(crate) const DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION: &str =
    "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json";
pub(crate) const DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION: &str =
    "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json";

const DEFAULT_BENCHMARK_BASELINE_PATH: &str = ".deepcli/baselines/competitor.json";
const DEFAULT_BENCHMARK_CURRENT_BASELINE_PATH: &str = ".deepcli/baselines/current-main.json";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkBaselinesOptions {
    json_output: bool,
    output_path: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BenchmarkBaselineTemplateOptions {
    json_output: bool,
    output_path: Option<String>,
    name: String,
    from_current: bool,
}

impl Default for BenchmarkBaselineTemplateOptions {
    fn default() -> Self {
        Self {
            json_output: false,
            output_path: None,
            name: "competitor".to_string(),
            from_current: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BenchmarkBaselineReport {
    pub(crate) present: bool,
    pub(crate) name: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) cases: Vec<BenchmarkBaselineCase>,
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkBaselineCase {
    pub(crate) suite: String,
    pub(crate) case_name: String,
    pub(crate) status: Option<String>,
    pub(crate) duration_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct BenchmarkBaselineInventoryEntry {
    path: String,
    name: Option<String>,
    status: String,
    case_count: usize,
    missing_value_count: usize,
    ready_to_compare: bool,
    is_default: bool,
    error: Option<String>,
    cases: Vec<BenchmarkBaselineCase>,
}

#[derive(Debug, Clone)]
struct BenchmarkBaselineTemplateCapture {
    status: String,
    duration_ms: Option<u64>,
    artifact_path: String,
}

pub(crate) fn sota_baseline_next_actions(workspace: &Path) -> Vec<String> {
    if workspace.join(DEFAULT_BENCHMARK_BASELINE_PATH).is_file() {
        if benchmark_default_baseline_file_ready(workspace) {
            vec![DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION.to_string()]
        } else {
            vec!["deepcli benchmark baselines --json".to_string()]
        }
    } else if benchmark_current_baseline_file_ready(workspace) {
        vec![DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION.to_string()]
    } else if benchmark_current_baseline_ready(workspace) {
        vec![
            DEFAULT_BENCHMARK_CURRENT_BASELINE_TEMPLATE_ACTION.to_string(),
            DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION.to_string(),
        ]
    } else {
        vec![DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION.to_string()]
    }
}

fn benchmark_default_baseline_file_ready(workspace: &Path) -> bool {
    load_benchmark_baseline(workspace, Some(DEFAULT_BENCHMARK_BASELINE_PATH))
        .map(|baseline| benchmark_baseline_ready_for_required_presets(&baseline))
        .unwrap_or(false)
}

fn benchmark_current_baseline_file_ready(workspace: &Path) -> bool {
    load_benchmark_baseline(workspace, Some(DEFAULT_BENCHMARK_CURRENT_BASELINE_PATH))
        .map(|baseline| benchmark_baseline_ready_for_required_presets(&baseline))
        .unwrap_or(false)
}

fn benchmark_current_baseline_ready(workspace: &Path) -> bool {
    load_benchmark_artifacts(workspace)
        .map(|artifacts| {
            let captures = benchmark_current_baseline_captures(&artifacts);
            benchmark_baseline_template_status(Some(&captures)) == "ready"
        })
        .unwrap_or(false)
}

fn benchmark_baseline_ready_for_required_presets(baseline: &BenchmarkBaselineReport) -> bool {
    baseline.present
        && MEANINGFUL_BENCHMARK_PRESETS
            .iter()
            .filter_map(|preset_name| benchmark_preset_by_name(preset_name).ok())
            .all(|preset| {
                baseline.cases.iter().any(|case| {
                    case.suite == preset.suite
                        && case.case_name == preset.case_name
                        && case
                            .status
                            .as_deref()
                            .is_some_and(|status| !status.trim().is_empty() && status != "unknown")
                        && case.duration_ms.is_some()
                })
            })
}

pub(crate) fn handle_benchmark_baselines(workspace: &Path, args: &[String]) -> Result<String> {
    let options = parse_benchmark_baselines_options(args)?;
    let mut baselines = load_benchmark_baseline_inventory(workspace)?;
    if let Some(limit) = options.limit {
        baselines.truncate(limit);
    }
    let output = if options.json_output {
        format_benchmark_baselines_json(workspace, &baselines)?
    } else {
        format_benchmark_baselines_text(workspace, &baselines)
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_benchmark_baselines_options(args: &[String]) -> Result<BenchmarkBaselinesOptions> {
    let mut options = BenchmarkBaselinesOptions::default();
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
            value => bail!("unsupported /benchmark baselines option `{value}`"),
        }
    }
    Ok(options)
}

pub(crate) fn set_benchmark_baseline_path(target: &mut Option<String>, raw: &str) -> Result<()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("baseline path must not be empty");
    }
    if target.is_some() {
        bail!("multiple benchmark baseline paths were provided");
    }
    *target = Some(trimmed.to_string());
    Ok(())
}

pub(crate) fn handle_benchmark_baseline_template(
    workspace: &Path,
    args: &[String],
) -> Result<String> {
    let options = parse_benchmark_baseline_template_options(args)?;
    let target_path = benchmark_baseline_template_target_path(&options);
    let captures = if options.from_current {
        Some(benchmark_current_baseline_captures(
            &load_benchmark_artifacts(workspace)?,
        ))
    } else {
        None
    };
    let status = benchmark_baseline_template_status(captures.as_ref());
    let next_actions = benchmark_baseline_template_next_actions(
        &target_path,
        &options.name,
        status,
        options.from_current,
        options.output_path.is_some(),
    );
    let base_template =
        benchmark_baseline_template_value(&options.name, &next_actions, None, captures.as_ref());
    let base_template_json = serde_json::to_string_pretty(&base_template)?;
    let report = format_benchmark_baseline_template_text(
        workspace,
        &options,
        &base_template_json,
        status,
        &next_actions,
    );
    let template = benchmark_baseline_template_value(
        &options.name,
        &next_actions,
        Some(&report),
        captures.as_ref(),
    );
    let template_json = serde_json::to_string_pretty(&template)?;
    let output = if options.json_output {
        template_json.clone()
    } else {
        format_benchmark_baseline_template_text(
            workspace,
            &options,
            &template_json,
            status,
            &next_actions,
        )
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &template_json)?;
    }
    Ok(output)
}

fn parse_benchmark_baseline_template_options(
    args: &[String],
) -> Result<BenchmarkBaselineTemplateOptions> {
    let mut options = BenchmarkBaselineTemplateOptions::default();
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
            "--name" => {
                options.name =
                    parse_benchmark_baseline_name(required_arg(args, index + 1, "baseline name")?)?;
                index += 2;
            }
            value if value.starts_with("--name=") => {
                options.name = parse_benchmark_baseline_name(value.trim_start_matches("--name="))?;
                index += 1;
            }
            "--from-current" | "--from-latest" | "--capture-current" => {
                options.from_current = true;
                index += 1;
            }
            value => bail!("unsupported /benchmark baseline-template option `{value}`"),
        }
    }
    Ok(options)
}

fn parse_benchmark_baseline_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("baseline name must not be empty");
    }
    Ok(redact_sensitive_text(trimmed))
}

fn benchmark_baseline_template_value(
    name: &str,
    next_actions: &[String],
    report: Option<&str>,
    captures: Option<&BTreeMap<String, BenchmarkBaselineTemplateCapture>>,
) -> Value {
    let checklist = benchmark_action_checklist(next_actions);
    let cases = MEANINGFUL_BENCHMARK_PRESETS
        .iter()
        .filter_map(|preset_name| benchmark_preset_by_name(preset_name).ok())
        .map(|preset| {
            let capture = captures.and_then(|captures| captures.get(preset.name));
            json!({
                "suite": preset.suite,
                "case": preset.case_name,
                "preset": preset.name,
                "command": preset.command,
                "status": capture.map(|capture| Value::String(capture.status.clone())).unwrap_or(Value::Null),
                "durationMs": capture.and_then(|capture| capture.duration_ms).map(Value::from).unwrap_or(Value::Null),
                "notes": capture
                    .map(|capture| format!("captured from {}", capture.artifact_path))
                    .unwrap_or_else(|| "fill status with passed, failed, timeout, or recorded; fill durationMs when known".to_string()),
            })
        })
        .collect::<Vec<_>>();
    let status = benchmark_baseline_template_status(captures);
    json!({
        "schema": schema_ids::BENCHMARK_BASELINE_V1,
        "status": status,
        "name": name,
        "source": if captures.is_some() {
            "current_benchmark_artifacts"
        } else {
            "manual_template"
        },
        "cases": cases,
        "nextActions": next_actions,
        "checklist": checklist,
        "report": report,
    })
}

fn benchmark_current_baseline_captures(
    artifacts: &[BenchmarkArtifact],
) -> BTreeMap<String, BenchmarkBaselineTemplateCapture> {
    MEANINGFUL_BENCHMARK_PRESETS
        .iter()
        .filter_map(|preset_name| {
            let preset = benchmark_preset_by_name(preset_name).ok()?;
            let artifact = artifacts
                .iter()
                .find(|artifact| benchmark_artifact_matches_preset(&artifact.value, preset))?;
            Some((
                preset.name.to_string(),
                BenchmarkBaselineTemplateCapture {
                    status: benchmark_artifact_status(&artifact.value).to_string(),
                    duration_ms: benchmark_artifact_duration_ms(&artifact.value),
                    artifact_path: artifact.relative_path.clone(),
                },
            ))
        })
        .collect()
}

fn benchmark_baseline_template_status(
    captures: Option<&BTreeMap<String, BenchmarkBaselineTemplateCapture>>,
) -> &'static str {
    let Some(captures) = captures else {
        return "needs_values";
    };
    let ready = MEANINGFUL_BENCHMARK_PRESETS.iter().all(|preset| {
        captures
            .get(*preset)
            .is_some_and(|capture| capture.status != "unknown" && capture.duration_ms.is_some())
    });
    if ready {
        "ready"
    } else {
        "needs_values"
    }
}

fn benchmark_baseline_template_target_path(options: &BenchmarkBaselineTemplateOptions) -> String {
    let default_target_path = format!(
        ".deepcli/baselines/{}.json",
        benchmark_slug(&options.name, "competitor")
    );
    options.output_path.clone().unwrap_or(default_target_path)
}

fn benchmark_baseline_template_next_actions(
    target_path: &str,
    name: &str,
    status: &str,
    from_current: bool,
    writes_output: bool,
) -> Vec<String> {
    if !writes_output {
        let mut actions = Vec::new();
        if from_current && status != "ready" {
            actions.push("deepcli benchmark run-suite --json --fail-on-command".to_string());
        }
        actions.push(benchmark_baseline_template_persist_action(
            target_path,
            name,
            from_current,
        ));
        actions.extend([
            "deepcli benchmark trends --json".to_string(),
            "deepcli benchmark status --json".to_string(),
        ]);
        return actions;
    }
    if status == "ready" {
        return vec![
            format!("deepcli benchmark compare --baseline {target_path} --json"),
            "deepcli benchmark trends --json".to_string(),
            "deepcli benchmark status --json".to_string(),
        ];
    }
    let mut actions = Vec::new();
    if from_current {
        actions.push("deepcli benchmark run-suite --json --fail-on-command".to_string());
    }
    actions.push(format!(
        "edit status and durationMs values in {target_path}"
    ));
    actions.push(format!(
        "deepcli benchmark compare --baseline {target_path} --json"
    ));
    actions
}

fn benchmark_baseline_template_persist_action(
    target_path: &str,
    name: &str,
    from_current: bool,
) -> String {
    let mut action = "deepcli benchmark baseline-template".to_string();
    if from_current {
        action.push_str(" --from-current");
    }
    if name != "competitor" {
        action.push_str(" --name ");
        action.push_str(shell_words::quote(name).as_ref());
    }
    action.push_str(" --output ");
    action.push_str(shell_words::quote(target_path).as_ref());
    action.push_str(" --json");
    action
}

fn format_benchmark_baseline_template_text(
    workspace: &Path,
    options: &BenchmarkBaselineTemplateOptions,
    template_json: &str,
    status: &str,
    next_actions: &[String],
) -> String {
    let mut lines = vec![
        "deepcli benchmark baseline-template".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("status: {status}"),
        format!(
            "source: {}",
            if options.from_current {
                "current benchmark artifacts"
            } else {
                "manual template"
            }
        ),
        format!("name: {}", options.name),
        format!("case count: {}", MEANINGFUL_BENCHMARK_PRESETS.len()),
    ];
    if let Some(output_path) = &options.output_path {
        lines.push(format!("wrote baseline template: {output_path}"));
    }
    lines.push("template:".to_string());
    lines.push(template_json.to_string());
    lines.push("next actions:".to_string());
    lines.extend(next_actions.iter().map(|action| format!("  - {action}")));
    lines.join("\n")
}

pub(crate) fn load_benchmark_baseline(
    workspace: &Path,
    raw_path: Option<&str>,
) -> Result<BenchmarkBaselineReport> {
    let Some(raw_path) = raw_path else {
        return Ok(BenchmarkBaselineReport::default());
    };
    let path = resolve_workspace_path(workspace, raw_path)?;
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse benchmark baseline {}", path.display()))?;
    let cases_value = if let Some(cases) = value.get("cases").and_then(Value::as_array) {
        cases
    } else if let Some(cases) = value.as_array() {
        cases
    } else {
        bail!("benchmark baseline must be a JSON object with a cases array or an array of cases");
    };
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(redact_sensitive_text)
        });
    let cases = cases_value
        .iter()
        .filter_map(benchmark_baseline_case_from_value)
        .collect::<Vec<_>>();
    Ok(BenchmarkBaselineReport {
        present: true,
        name,
        path: Some(workspace_relative_display(workspace, &path).replace('\\', "/")),
        cases,
    })
}

fn load_benchmark_baseline_inventory(
    workspace: &Path,
) -> Result<Vec<BenchmarkBaselineInventoryEntry>> {
    let baselines_dir = workspace.join(".deepcli/baselines");
    if !baselines_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&baselines_dir)
        .with_context(|| format!("failed to read {}", baselines_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            paths.push(path);
        }
    }
    paths.sort_by(|left, right| {
        workspace_relative_display(workspace, left)
            .cmp(&workspace_relative_display(workspace, right))
    });
    paths
        .into_iter()
        .map(|path| benchmark_baseline_inventory_entry(workspace, &path))
        .collect()
}

fn benchmark_baseline_inventory_entry(
    workspace: &Path,
    path: &Path,
) -> Result<BenchmarkBaselineInventoryEntry> {
    let relative_path = workspace_relative_display(workspace, path).replace('\\', "/");
    let is_default = relative_path == DEFAULT_BENCHMARK_BASELINE_PATH;
    match load_benchmark_baseline(workspace, Some(&relative_path)) {
        Ok(report) => {
            let missing_value_count = benchmark_baseline_missing_value_count(&report);
            let ready_to_compare = !report.cases.is_empty() && missing_value_count == 0;
            Ok(BenchmarkBaselineInventoryEntry {
                path: relative_path,
                name: report.name,
                status: if ready_to_compare {
                    "ready".to_string()
                } else {
                    "needs_values".to_string()
                },
                case_count: report.cases.len(),
                missing_value_count,
                ready_to_compare,
                is_default,
                error: None,
                cases: report.cases,
            })
        }
        Err(error) => Ok(BenchmarkBaselineInventoryEntry {
            path: relative_path,
            name: path
                .file_stem()
                .and_then(|name| name.to_str())
                .map(redact_sensitive_text),
            status: "invalid".to_string(),
            case_count: 0,
            missing_value_count: 0,
            ready_to_compare: false,
            is_default,
            error: Some(redact_sensitive_text(&error.to_string())),
            cases: Vec::new(),
        }),
    }
}

fn benchmark_baseline_missing_value_count(baseline: &BenchmarkBaselineReport) -> usize {
    baseline
        .cases
        .iter()
        .filter(|case| case.status.is_none() || case.duration_ms.is_none())
        .count()
}

fn benchmark_baseline_case_from_value(value: &Value) -> Option<BenchmarkBaselineCase> {
    if !value.is_object() {
        return None;
    }
    let suite = value
        .get("suite")
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .unwrap_or_else(|| "<unknown>".to_string());
    let case_name = value
        .get("case")
        .or_else(|| value.get("caseName"))
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .unwrap_or_else(|| "<unknown>".to_string());
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .map(redact_sensitive_text);
    let duration_ms = value
        .get("durationMs")
        .or_else(|| value.get("duration_ms"))
        .and_then(Value::as_u64);
    Some(BenchmarkBaselineCase {
        suite,
        case_name,
        status,
        duration_ms,
    })
}

pub(crate) fn benchmark_baseline_report_json(baseline: &BenchmarkBaselineReport) -> Value {
    json!({
        "present": baseline.present,
        "name": baseline.name,
        "path": baseline.path,
        "caseCount": baseline.cases.len(),
        "cases": baseline
            .cases
            .iter()
            .map(benchmark_baseline_case_json)
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn benchmark_baseline_case_json(case: &BenchmarkBaselineCase) -> Value {
    json!({
        "suite": case.suite,
        "case": case.case_name,
        "status": case.status,
        "durationMs": case.duration_ms,
    })
}

fn format_benchmark_baselines_json(
    workspace: &Path,
    baselines: &[BenchmarkBaselineInventoryEntry],
) -> Result<String> {
    let status = benchmark_baselines_status(baselines);
    let baseline_count = baselines.len();
    let ready_count = baselines
        .iter()
        .filter(|baseline| baseline.status == "ready")
        .count();
    let needs_values_count = baselines
        .iter()
        .filter(|baseline| baseline.status == "needs_values")
        .count();
    let invalid_count = baselines
        .iter()
        .filter(|baseline| baseline.status == "invalid")
        .count();
    let default_baseline = benchmark_default_baseline_json(baselines);
    let next_actions = benchmark_baselines_next_actions(workspace, baselines);
    let checklist = benchmark_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::BENCHMARK_BASELINES_V1,
        "status": status,
        "workspace": workspace.display().to_string(),
        "summary": benchmark_baselines_summary_json(
            status,
            baselines,
            &default_baseline,
            &checklist,
        ),
        "baselineCount": baseline_count,
        "readyCount": ready_count,
        "needsValuesCount": needs_values_count,
        "invalidCount": invalid_count,
        "defaultBaseline": default_baseline,
        "baselines": baselines
            .iter()
            .map(benchmark_baseline_inventory_entry_json)
            .collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": format_benchmark_baselines_text(workspace, baselines),
    }))?)
}

fn benchmark_baselines_summary_json(
    status: &str,
    baselines: &[BenchmarkBaselineInventoryEntry],
    default_baseline: &Value,
    checklist: &[Value],
) -> Value {
    let baseline_count = baselines.len();
    let ready_count = baselines
        .iter()
        .filter(|baseline| baseline.status == "ready")
        .count();
    let needs_values_count = baselines
        .iter()
        .filter(|baseline| baseline.status == "needs_values")
        .count();
    let invalid_count = baselines
        .iter()
        .filter(|baseline| baseline.status == "invalid")
        .count();
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
        "baselineCount": baseline_count,
        "readyCount": ready_count,
        "needsValuesCount": needs_values_count,
        "invalidCount": invalid_count,
        "compareReady": default_baseline
            .get("readyToCompare")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "compareReadyCount": baselines
            .iter()
            .filter(|baseline| baseline.ready_to_compare)
            .count(),
        "defaultBaselineStatus": default_baseline
            .get("status")
            .cloned()
            .unwrap_or(Value::Null),
        "defaultBaselinePath": default_baseline
            .get("path")
            .cloned()
            .unwrap_or(Value::Null),
        "recommendedAction": recommended_action,
        "recommendedActionLabel": recommended_action_label,
    })
}

fn benchmark_baselines_status(baselines: &[BenchmarkBaselineInventoryEntry]) -> &'static str {
    if baselines.is_empty() {
        return "empty";
    }
    if !baselines.iter().any(|baseline| baseline.is_default) {
        return "needs_default";
    }
    let ready_count = baselines
        .iter()
        .filter(|baseline| baseline.status == "ready")
        .count();
    if ready_count == baselines.len() {
        "ready"
    } else if ready_count > 0 {
        "mixed"
    } else if baselines
        .iter()
        .any(|baseline| baseline.status == "needs_values")
    {
        "needs_values"
    } else {
        "invalid"
    }
}

fn benchmark_default_baseline_json(baselines: &[BenchmarkBaselineInventoryEntry]) -> Value {
    if let Some(baseline) = baselines.iter().find(|baseline| baseline.is_default) {
        json!({
            "present": true,
            "path": baseline.path,
            "name": baseline.name,
            "status": baseline.status,
            "readyToCompare": baseline.ready_to_compare,
            "caseCount": baseline.case_count,
            "missingValueCount": baseline.missing_value_count,
            "error": baseline.error,
        })
    } else {
        json!({
            "present": false,
            "path": DEFAULT_BENCHMARK_BASELINE_PATH,
            "name": "competitor",
            "status": "missing",
            "readyToCompare": false,
            "caseCount": 0,
            "missingValueCount": 0,
            "error": Value::Null,
        })
    }
}

fn benchmark_baseline_inventory_entry_json(entry: &BenchmarkBaselineInventoryEntry) -> Value {
    json!({
        "path": entry.path,
        "name": entry.name,
        "status": entry.status,
        "caseCount": entry.case_count,
        "missingValueCount": entry.missing_value_count,
        "readyToCompare": entry.ready_to_compare,
        "default": entry.is_default,
        "error": entry.error,
        "cases": entry.cases.iter().map(benchmark_baseline_case_json).collect::<Vec<_>>(),
    })
}

fn benchmark_baselines_next_actions(
    workspace: &Path,
    baselines: &[BenchmarkBaselineInventoryEntry],
) -> Vec<String> {
    let mut actions = Vec::new();
    if baselines.is_empty() {
        actions.extend(sota_baseline_next_actions(workspace));
    } else {
        if !baselines.iter().any(|baseline| baseline.is_default) {
            actions.push(DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION.to_string());
        }
        if baselines
            .iter()
            .any(|baseline| baseline.is_default && baseline.status == "invalid")
        {
            actions.push(DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION.to_string());
        }
        for baseline in baselines
            .iter()
            .filter(|baseline| baseline.ready_to_compare)
        {
            actions.push(format!(
                "deepcli benchmark compare --baseline {} --json",
                baseline.path
            ));
        }
        for baseline in baselines
            .iter()
            .filter(|baseline| baseline.status == "needs_values")
        {
            actions.push(format!(
                "edit status and durationMs values in {}",
                baseline.path
            ));
            actions.push(format!(
                "deepcli benchmark compare --baseline {} --json",
                baseline.path
            ));
        }
        actions.extend(sota_baseline_next_actions(workspace));
    }
    actions.push("deepcli benchmark trends --json".to_string());
    actions.push("deepcli benchmark status --json".to_string());
    actions.push("deepcli scorecard --json".to_string());
    dedup_preserve_order(actions)
}

fn format_benchmark_baselines_text(
    workspace: &Path,
    baselines: &[BenchmarkBaselineInventoryEntry],
) -> String {
    let status = benchmark_baselines_status(baselines);
    let mut lines = vec![
        "deepcli benchmark baselines".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("status: {status}"),
        format!("baseline count: {}", baselines.len()),
    ];
    let default = baselines.iter().find(|baseline| baseline.is_default);
    if let Some(default) = default {
        lines.push(format!(
            "default baseline: {} status={}",
            default.path, default.status
        ));
    } else {
        lines.push(format!(
            "default baseline: {} status=missing",
            DEFAULT_BENCHMARK_BASELINE_PATH
        ));
    }
    if baselines.is_empty() {
        lines.push("baselines: none".to_string());
    } else {
        lines.push("baselines:".to_string());
        for baseline in baselines {
            lines.push(format!(
                "  - {}: status={} name={} cases={} missing_values={} ready_to_compare={} default={}",
                baseline.path,
                baseline.status,
                baseline.name.as_deref().unwrap_or("none"),
                baseline.case_count,
                baseline.missing_value_count,
                baseline.ready_to_compare,
                baseline.is_default
            ));
            if let Some(error) = &baseline.error {
                lines.push(format!("    error: {error}"));
            }
        }
    }
    lines.push("next actions:".to_string());
    lines.extend(
        benchmark_baselines_next_actions(workspace, baselines)
            .into_iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

pub(crate) fn benchmark_baseline_needs_values(baseline: &BenchmarkBaselineReport) -> bool {
    baseline
        .cases
        .iter()
        .any(|case| case.status.is_none() || case.duration_ms.is_none())
}
