use super::*;
use crate::schema_ids;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkSummaryOptions {
    json_output: bool,
    output_path: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkTrendOptions {
    json_output: bool,
    output_path: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkCompareOptions {
    json_output: bool,
    output_path: Option<String>,
    baseline_path: Option<String>,
    limit: Option<usize>,
}

pub(crate) fn handle_benchmark_summary(workspace: &Path, args: &[String]) -> Result<String> {
    let options = parse_benchmark_summary_options(args)?;
    let mut artifacts = load_benchmark_artifacts(workspace)?;
    if let Some(limit) = options.limit {
        artifacts.truncate(limit);
    }
    let summaries = build_benchmark_case_summaries(&artifacts);
    let output = if options.json_output {
        format_benchmark_summary_json(workspace, &artifacts, &summaries)?
    } else {
        format_benchmark_summary_text(workspace, artifacts.len(), &summaries)
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_benchmark_summary_options(args: &[String]) -> Result<BenchmarkSummaryOptions> {
    let mut options = BenchmarkSummaryOptions::default();
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
            value => bail!("unsupported /benchmark summary option `{value}`"),
        }
    }
    Ok(options)
}

pub(crate) fn handle_benchmark_trends(workspace: &Path, args: &[String]) -> Result<String> {
    let options = parse_benchmark_trend_options(args)?;
    let artifacts = load_benchmark_artifacts(workspace)?;
    let recent_limit = options.limit.unwrap_or(5);
    let trends = build_benchmark_case_trends(&artifacts, recent_limit);
    let output = if options.json_output {
        format_benchmark_trends_json(workspace, &artifacts, &trends, recent_limit)?
    } else {
        format_benchmark_trends_text(workspace, artifacts.len(), &trends, recent_limit)
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_benchmark_trend_options(args: &[String]) -> Result<BenchmarkTrendOptions> {
    let mut options = BenchmarkTrendOptions::default();
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
            value => bail!("unsupported /benchmark trends option `{value}`"),
        }
    }
    Ok(options)
}

pub(crate) fn handle_benchmark_compare(workspace: &Path, args: &[String]) -> Result<String> {
    let options = parse_benchmark_compare_options(args)?;
    let mut artifacts = load_benchmark_artifacts(workspace)?;
    if let Some(limit) = options.limit {
        artifacts.truncate(limit);
    }
    let trends = build_benchmark_case_trends(&artifacts, 1);
    let baseline = load_benchmark_baseline(workspace, options.baseline_path.as_deref())?;
    let comparisons = build_benchmark_comparisons(&trends, &baseline);
    let output = if options.json_output {
        format_benchmark_compare_json(workspace, &artifacts, &baseline, &comparisons)?
    } else {
        format_benchmark_compare_text(workspace, artifacts.len(), &baseline, &comparisons)
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_benchmark_compare_options(args: &[String]) -> Result<BenchmarkCompareOptions> {
    let mut options = BenchmarkCompareOptions::default();
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
            "--baseline" => {
                set_benchmark_baseline_path(
                    &mut options.baseline_path,
                    required_arg(args, index + 1, "baseline path")?,
                )?;
                index += 2;
            }
            value if value.starts_with("--baseline=") => {
                set_benchmark_baseline_path(
                    &mut options.baseline_path,
                    value.trim_start_matches("--baseline="),
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
            value => bail!("unsupported /benchmark compare option `{value}`"),
        }
    }
    Ok(options)
}

#[derive(Debug, Clone, Default)]
struct BenchmarkCaseSummary {
    suite: String,
    case_name: String,
    total: usize,
    executable: usize,
    passed: usize,
    failed: usize,
    timed_out: usize,
    recorded: usize,
    other: usize,
    latest_status: String,
    latest_artifact_path: String,
    latest_created_at: String,
    latest_duration_ms: Option<u64>,
    average_duration_ms: Option<u64>,
    min_duration_ms: Option<u64>,
    max_duration_ms: Option<u64>,
    duration_count: usize,
}

#[derive(Debug, Clone, Default)]
struct BenchmarkCaseAccumulator {
    summary: BenchmarkCaseSummary,
    duration_sum_ms: u128,
}

#[derive(Debug, Clone)]
struct BenchmarkTrendPoint {
    artifact_path: String,
    created_at: String,
    preset: Option<String>,
    status: String,
    duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BenchmarkCaseTrend {
    suite: String,
    case_name: String,
    total: usize,
    executable: usize,
    passed: usize,
    failed: usize,
    timed_out: usize,
    recorded: usize,
    other: usize,
    latest: Option<BenchmarkTrendPoint>,
    previous: Option<BenchmarkTrendPoint>,
    duration_delta_ms: Option<i64>,
    duration_trend: String,
    status_trend: String,
    recent: Vec<BenchmarkTrendPoint>,
}

#[derive(Debug, Clone)]
struct BenchmarkCaseComparison {
    suite: String,
    case_name: String,
    current: Option<BenchmarkTrendPoint>,
    baseline: Option<BenchmarkBaselineCase>,
    status_comparison: String,
    duration_delta_ms: Option<i64>,
    duration_comparison: String,
}

fn build_benchmark_case_summaries(artifacts: &[BenchmarkArtifact]) -> Vec<BenchmarkCaseSummary> {
    let mut cases: BTreeMap<(String, String), BenchmarkCaseAccumulator> = BTreeMap::new();
    for artifact in artifacts {
        let suite = artifact_string_field(&artifact.value, "suite", "<unknown>");
        let case_name = artifact_string_field(&artifact.value, "case", "<unknown>");
        let status = benchmark_artifact_status(&artifact.value).to_string();
        let duration_ms = benchmark_artifact_duration_ms(&artifact.value);
        let created_at = artifact_string_field(&artifact.value, "createdAt", "<unknown>");
        let entry = cases
            .entry((suite.clone(), case_name.clone()))
            .or_insert_with(|| BenchmarkCaseAccumulator {
                summary: BenchmarkCaseSummary {
                    suite: suite.clone(),
                    case_name: case_name.clone(),
                    latest_status: status.clone(),
                    latest_artifact_path: artifact.relative_path.clone(),
                    latest_created_at: created_at.clone(),
                    latest_duration_ms: duration_ms,
                    ..BenchmarkCaseSummary::default()
                },
                duration_sum_ms: 0,
            });
        entry.summary.total += 1;
        match status.as_str() {
            "passed" => {
                entry.summary.executable += 1;
                entry.summary.passed += 1;
            }
            "failed" => {
                entry.summary.executable += 1;
                entry.summary.failed += 1;
            }
            "timeout" => {
                entry.summary.executable += 1;
                entry.summary.timed_out += 1;
            }
            "recorded" => {
                entry.summary.recorded += 1;
            }
            _ => {
                entry.summary.other += 1;
            }
        }
        if let Some(duration_ms) = duration_ms {
            entry.summary.duration_count += 1;
            entry.duration_sum_ms += duration_ms as u128;
            entry.summary.min_duration_ms = Some(
                entry
                    .summary
                    .min_duration_ms
                    .map_or(duration_ms, |current| current.min(duration_ms)),
            );
            entry.summary.max_duration_ms = Some(
                entry
                    .summary
                    .max_duration_ms
                    .map_or(duration_ms, |current| current.max(duration_ms)),
            );
        }
    }
    cases
        .into_values()
        .map(|mut case| {
            if case.summary.duration_count > 0 {
                case.summary.average_duration_ms =
                    Some((case.duration_sum_ms / case.summary.duration_count as u128) as u64);
            }
            case.summary
        })
        .collect()
}

pub(crate) fn build_benchmark_case_trends(
    artifacts: &[BenchmarkArtifact],
    recent_limit: usize,
) -> Vec<BenchmarkCaseTrend> {
    let mut cases: BTreeMap<(String, String), Vec<&BenchmarkArtifact>> = BTreeMap::new();
    for artifact in artifacts {
        let suite = artifact_string_field(&artifact.value, "suite", "<unknown>");
        let case_name = artifact_string_field(&artifact.value, "case", "<unknown>");
        cases.entry((suite, case_name)).or_default().push(artifact);
    }

    cases
        .into_iter()
        .map(|((suite, case_name), artifacts)| {
            let latest = artifacts
                .first()
                .map(|artifact| benchmark_trend_point(artifact));
            let previous = artifacts
                .get(1)
                .map(|artifact| benchmark_trend_point(artifact));
            let mut trend = BenchmarkCaseTrend {
                suite,
                case_name,
                total: artifacts.len(),
                latest: latest.clone(),
                previous: previous.clone(),
                duration_delta_ms: benchmark_duration_delta_ms(&latest, &previous),
                duration_trend: benchmark_duration_trend(&latest, &previous).to_string(),
                status_trend: benchmark_status_trend(&latest, &previous).to_string(),
                recent: artifacts
                    .iter()
                    .take(recent_limit)
                    .map(|artifact| benchmark_trend_point(artifact))
                    .collect(),
                ..BenchmarkCaseTrend::default()
            };
            for artifact in artifacts {
                match benchmark_artifact_status(&artifact.value) {
                    "passed" => {
                        trend.executable += 1;
                        trend.passed += 1;
                    }
                    "failed" => {
                        trend.executable += 1;
                        trend.failed += 1;
                    }
                    "timeout" => {
                        trend.executable += 1;
                        trend.timed_out += 1;
                    }
                    "recorded" => {
                        trend.recorded += 1;
                    }
                    _ => {
                        trend.other += 1;
                    }
                }
            }
            trend
        })
        .collect()
}

fn build_benchmark_comparisons(
    trends: &[BenchmarkCaseTrend],
    baseline: &BenchmarkBaselineReport,
) -> Vec<BenchmarkCaseComparison> {
    let mut cases: BTreeMap<
        (String, String),
        (Option<BenchmarkTrendPoint>, Option<BenchmarkBaselineCase>),
    > = BTreeMap::new();
    for trend in trends {
        cases.insert(
            (trend.suite.clone(), trend.case_name.clone()),
            (trend.latest.clone(), None),
        );
    }
    for baseline_case in &baseline.cases {
        cases
            .entry((baseline_case.suite.clone(), baseline_case.case_name.clone()))
            .and_modify(|(_, baseline)| *baseline = Some(baseline_case.clone()))
            .or_insert_with(|| (None, Some(baseline_case.clone())));
    }
    cases
        .into_iter()
        .map(|((suite, case_name), (current, baseline))| {
            let status_comparison = if baseline.as_ref().is_some_and(|case| case.status.is_none()) {
                "missing_baseline_status".to_string()
            } else {
                benchmark_status_comparison(
                    current.as_ref().map(|point| point.status.as_str()),
                    baseline.as_ref().and_then(|case| case.status.as_deref()),
                )
                .to_string()
            };
            let duration_delta_ms = benchmark_compare_duration_delta_ms(&current, &baseline);
            BenchmarkCaseComparison {
                suite,
                case_name,
                current,
                baseline,
                status_comparison,
                duration_delta_ms,
                duration_comparison: benchmark_compare_duration_comparison(duration_delta_ms)
                    .to_string(),
            }
        })
        .collect()
}

fn benchmark_status_comparison(
    current_status: Option<&str>,
    baseline_status: Option<&str>,
) -> &'static str {
    match (current_status, baseline_status) {
        (None, None) => "unknown",
        (None, Some(_)) => "missing_current",
        (Some(_), None) => "missing_baseline",
        (Some(current), Some(baseline)) if current == baseline => match current {
            "passed" => "same_pass",
            status if benchmark_problem_status(status) => "same_problem",
            _ => "same",
        },
        (Some(current), Some("passed")) if benchmark_problem_status(current) => "regressed",
        (Some("passed"), Some(baseline)) if benchmark_problem_status(baseline) => "recovered",
        (Some(_), Some(_)) => "changed",
    }
}

fn benchmark_compare_duration_delta_ms(
    current: &Option<BenchmarkTrendPoint>,
    baseline: &Option<BenchmarkBaselineCase>,
) -> Option<i64> {
    Some(current.as_ref()?.duration_ms? as i64 - baseline.as_ref()?.duration_ms? as i64)
}

fn benchmark_compare_duration_comparison(delta_ms: Option<i64>) -> &'static str {
    match delta_ms {
        Some(delta) if delta < 0 => "faster",
        Some(delta) if delta > 0 => "slower",
        Some(_) => "flat",
        None => "unknown",
    }
}

fn benchmark_trend_point(artifact: &BenchmarkArtifact) -> BenchmarkTrendPoint {
    BenchmarkTrendPoint {
        artifact_path: artifact.relative_path.clone(),
        created_at: artifact_string_field(&artifact.value, "createdAt", "<unknown>"),
        preset: benchmark_artifact_preset(&artifact.value).map(ToString::to_string),
        status: benchmark_artifact_status(&artifact.value).to_string(),
        duration_ms: benchmark_artifact_duration_ms(&artifact.value),
    }
}

fn benchmark_duration_delta_ms(
    latest: &Option<BenchmarkTrendPoint>,
    previous: &Option<BenchmarkTrendPoint>,
) -> Option<i64> {
    Some(latest.as_ref()?.duration_ms? as i64 - previous.as_ref()?.duration_ms? as i64)
}

fn benchmark_duration_trend(
    latest: &Option<BenchmarkTrendPoint>,
    previous: &Option<BenchmarkTrendPoint>,
) -> &'static str {
    match benchmark_duration_delta_ms(latest, previous) {
        Some(delta) if delta > 0 => "slower",
        Some(delta) if delta < 0 => "faster",
        Some(_) => "flat",
        None => "unknown",
    }
}

fn benchmark_status_trend(
    latest: &Option<BenchmarkTrendPoint>,
    previous: &Option<BenchmarkTrendPoint>,
) -> &'static str {
    let Some(latest) = latest else {
        return "none";
    };
    let Some(previous) = previous else {
        return "new";
    };
    if latest.status == previous.status {
        return match latest.status.as_str() {
            "passed" => "stable_pass",
            "failed" | "timeout" => "stable_problem",
            _ => "stable",
        };
    }
    if latest.status == "passed" && benchmark_problem_status(&previous.status) {
        "recovered"
    } else if benchmark_problem_status(&latest.status) && previous.status == "passed" {
        "regressed"
    } else {
        "changed"
    }
}

fn benchmark_problem_status(status: &str) -> bool {
    matches!(status, "failed" | "timeout")
}

fn format_benchmark_compare_json(
    workspace: &Path,
    artifacts: &[BenchmarkArtifact],
    baseline: &BenchmarkBaselineReport,
    comparisons: &[BenchmarkCaseComparison],
) -> Result<String> {
    let next_actions = benchmark_compare_next_actions(baseline, artifacts.is_empty());
    let checklist = benchmark_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::BENCHMARK_COMPARE_V1,
        "status": benchmark_compare_status(artifacts.len(), baseline, comparisons),
        "workspace": workspace.display().to_string(),
        "artifactCount": artifacts.len(),
        "baseline": benchmark_baseline_report_json(baseline),
        "comparisonCount": comparisons.len(),
        "comparisons": comparisons
            .iter()
            .map(benchmark_case_comparison_json)
            .collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": format_benchmark_compare_text(workspace, artifacts.len(), baseline, comparisons),
    }))?)
}

fn benchmark_compare_status(
    artifact_count: usize,
    baseline: &BenchmarkBaselineReport,
    comparisons: &[BenchmarkCaseComparison],
) -> &'static str {
    if artifact_count == 0 && comparisons.is_empty() {
        return "empty";
    }
    if !baseline.present {
        return "needs_baseline";
    }
    if comparisons
        .iter()
        .any(|case| case.status_comparison == "regressed")
    {
        return "regression";
    }
    if comparisons.iter().any(|case| {
        matches!(
            case.status_comparison.as_str(),
            "missing_current" | "missing_baseline" | "missing_baseline_status"
        )
    }) {
        return "incomplete";
    }
    if benchmark_baseline_needs_values(baseline) {
        return "incomplete";
    }
    "ok"
}

fn benchmark_case_comparison_json(case: &BenchmarkCaseComparison) -> Value {
    json!({
        "suite": case.suite,
        "case": case.case_name,
        "current": benchmark_trend_point_json(&case.current),
        "baseline": case
            .baseline
            .as_ref()
            .map(benchmark_baseline_case_json)
            .unwrap_or(Value::Null),
        "statusComparison": case.status_comparison,
        "durationDeltaMs": case.duration_delta_ms,
        "durationComparison": case.duration_comparison,
    })
}

fn benchmark_compare_next_actions(baseline: &BenchmarkBaselineReport, empty: bool) -> Vec<String> {
    let mut actions = Vec::new();
    if !baseline.present {
        actions.push(
            "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
                .to_string(),
        );
        actions.push(
            "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
                .to_string(),
        );
    } else if benchmark_baseline_needs_values(baseline) {
        let path = baseline
            .path
            .as_deref()
            .unwrap_or(".deepcli/baselines/competitor.json");
        actions.push(format!("edit status and durationMs values in {path}"));
        actions.push(format!(
            "deepcli benchmark compare --baseline {path} --json"
        ));
    }
    if empty {
        actions.push("deepcli benchmark presets --json".to_string());
        actions.push("deepcli benchmark run-suite --json --fail-on-command".to_string());
        actions
            .push("deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string());
    } else {
        actions.push("deepcli benchmark summary --json".to_string());
        actions.push("deepcli benchmark trends --json".to_string());
        actions.push("deepcli benchmark status --json".to_string());
        actions.push("deepcli benchmark list --json".to_string());
    }
    actions.push("deepcli scorecard --json".to_string());
    actions
}

fn format_benchmark_summary_json(
    workspace: &Path,
    artifacts: &[BenchmarkArtifact],
    summaries: &[BenchmarkCaseSummary],
) -> Result<String> {
    let next_actions = benchmark_summary_next_actions(workspace);
    let checklist = benchmark_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::BENCHMARK_SUMMARY_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "summary": benchmark_summary_summary_json(artifacts.len(), summaries, &checklist),
        "artifactCount": artifacts.len(),
        "caseCount": summaries.len(),
        "totals": benchmark_summary_totals_json(summaries),
        "cases": summaries.iter().map(benchmark_case_summary_json).collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": format_benchmark_summary_text(workspace, artifacts.len(), summaries),
    }))?)
}

fn benchmark_summary_next_actions(workspace: &Path) -> Vec<String> {
    let mut actions = vec![
        "deepcli benchmark presets --json".to_string(),
        "deepcli benchmark run-suite --json --fail-on-command".to_string(),
        "deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string(),
        "deepcli benchmark status --json".to_string(),
        "deepcli benchmark trends --json".to_string(),
    ];
    actions.extend(sota_baseline_next_actions(workspace));
    actions.extend([
        "deepcli benchmark list --json".to_string(),
        "deepcli benchmark show latest --json".to_string(),
        "deepcli benchmark clean --dry-run --json".to_string(),
        "deepcli scorecard --json".to_string(),
    ]);
    actions
}

fn benchmark_summary_summary_json(
    artifact_count: usize,
    summaries: &[BenchmarkCaseSummary],
    checklist: &[Value],
) -> Value {
    let total = summaries.iter().map(|case| case.total).sum::<usize>();
    let executable = summaries.iter().map(|case| case.executable).sum::<usize>();
    let passed = summaries.iter().map(|case| case.passed).sum::<usize>();
    let failed = summaries.iter().map(|case| case.failed).sum::<usize>();
    let timed_out = summaries.iter().map(|case| case.timed_out).sum::<usize>();
    let recorded = summaries.iter().map(|case| case.recorded).sum::<usize>();
    let other = summaries.iter().map(|case| case.other).sum::<usize>();
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
        "artifactCount": artifact_count,
        "caseCount": summaries.len(),
        "total": total,
        "executableCount": executable,
        "passedCount": passed,
        "failedCount": failed,
        "timeoutCount": timed_out,
        "recordedCount": recorded,
        "otherCount": other,
        "passRatePercent": benchmark_pass_rate_percent(passed, executable),
        "recommendedAction": recommended_action,
        "recommendedActionLabel": recommended_action_label,
    })
}

fn format_benchmark_trends_json(
    workspace: &Path,
    artifacts: &[BenchmarkArtifact],
    trends: &[BenchmarkCaseTrend],
    recent_limit: usize,
) -> Result<String> {
    let status = benchmark_trends_status(artifacts.len(), trends);
    let next_actions = benchmark_trends_next_actions(workspace, status);
    let checklist = benchmark_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::BENCHMARK_TRENDS_V1,
        "status": status,
        "workspace": workspace.display().to_string(),
        "artifactCount": artifacts.len(),
        "caseCount": trends.len(),
        "recentLimit": recent_limit,
        "summary": benchmark_trends_summary_json(
            status,
            artifacts.len(),
            trends,
            &checklist,
        ),
        "trends": trends.iter().map(benchmark_case_trend_json).collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": format_benchmark_trends_text(workspace, artifacts.len(), trends, recent_limit),
    }))?)
}

fn benchmark_trends_summary_json(
    status: &str,
    artifact_count: usize,
    trends: &[BenchmarkCaseTrend],
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
        "artifactCount": artifact_count,
        "caseCount": trends.len(),
        "regressionCount": trends
            .iter()
            .filter(|trend| benchmark_trend_has_regression(trend))
            .count(),
        "recoveredCount": trends
            .iter()
            .filter(|trend| trend.status_trend == "recovered")
            .count(),
        "stablePassCount": trends
            .iter()
            .filter(|trend| trend.status_trend == "stable_pass")
            .count(),
        "slowerCount": trends
            .iter()
            .filter(|trend| trend.duration_trend == "slower")
            .count(),
        "fasterCount": trends
            .iter()
            .filter(|trend| trend.duration_trend == "faster")
            .count(),
        "flatCount": trends
            .iter()
            .filter(|trend| trend.duration_trend == "flat")
            .count(),
        "unknownDurationCount": trends
            .iter()
            .filter(|trend| trend.duration_trend == "unknown")
            .count(),
        "recommendedAction": recommended_action,
        "recommendedActionLabel": recommended_action_label,
    })
}

pub(crate) fn benchmark_trends_status(
    artifact_count: usize,
    trends: &[BenchmarkCaseTrend],
) -> &'static str {
    if artifact_count == 0 {
        return "empty";
    }
    if trends.iter().any(benchmark_trend_has_regression) {
        return "regression";
    }
    if !trends.iter().any(|trend| trend.previous.is_some()) {
        "insufficient_history"
    } else {
        "ok"
    }
}

fn benchmark_trend_has_regression(trend: &BenchmarkCaseTrend) -> bool {
    trend.status_trend == "regressed"
        || trend
            .latest
            .as_ref()
            .is_some_and(|latest| benchmark_problem_status(&latest.status))
}

fn benchmark_case_trend_json(trend: &BenchmarkCaseTrend) -> Value {
    json!({
        "suite": trend.suite,
        "case": trend.case_name,
        "total": trend.total,
        "executableCount": trend.executable,
        "passedCount": trend.passed,
        "failedCount": trend.failed,
        "timeoutCount": trend.timed_out,
        "recordedCount": trend.recorded,
        "otherCount": trend.other,
        "passRatePercent": benchmark_pass_rate_percent(trend.passed, trend.executable),
        "statusTrend": trend.status_trend,
        "durationTrend": trend.duration_trend,
        "durationDeltaMs": trend.duration_delta_ms,
        "latest": benchmark_trend_point_json(&trend.latest),
        "previous": benchmark_trend_point_json(&trend.previous),
        "recent": trend
            .recent
            .iter()
            .map(benchmark_trend_point_value)
            .collect::<Vec<_>>(),
    })
}

fn benchmark_trend_point_json(point: &Option<BenchmarkTrendPoint>) -> Value {
    let Some(point) = point else {
        return Value::Null;
    };
    benchmark_trend_point_value(point)
}

fn benchmark_trend_point_value(point: &BenchmarkTrendPoint) -> Value {
    json!({
        "artifactPath": point.artifact_path,
        "createdAt": point.created_at,
        "preset": point.preset,
        "status": point.status,
        "durationMs": point.duration_ms,
    })
}

fn benchmark_trends_next_actions(workspace: &Path, status: &str) -> Vec<String> {
    match status {
        "empty" => vec![
            "deepcli benchmark presets --json".to_string(),
            "deepcli benchmark run-suite --json --fail-on-command".to_string(),
            "deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string(),
            "deepcli benchmark status --json".to_string(),
        ],
        "insufficient_history" => {
            let mut actions = vec![
                "deepcli round --json --run-benchmark --fail-on-command".to_string(),
                "deepcli benchmark run-suite --json --fail-on-command".to_string(),
                "deepcli benchmark status --json".to_string(),
                "deepcli benchmark summary --json".to_string(),
            ];
            actions.extend(sota_baseline_next_actions(workspace));
            actions.extend([
                "deepcli benchmark list --json".to_string(),
                "deepcli benchmark clean --dry-run --json".to_string(),
            ]);
            actions
        }
        _ => {
            let mut actions = vec![
                "deepcli benchmark status --json".to_string(),
                "deepcli benchmark summary --json".to_string(),
            ];
            actions.extend(sota_baseline_next_actions(workspace));
            actions.extend([
                "deepcli benchmark list --json".to_string(),
                "deepcli benchmark clean --dry-run --json".to_string(),
                "deepcli round --json".to_string(),
            ]);
            actions
        }
    }
}

fn benchmark_summary_totals_json(summaries: &[BenchmarkCaseSummary]) -> Value {
    let total = summaries.iter().map(|case| case.total).sum::<usize>();
    let executable = summaries.iter().map(|case| case.executable).sum::<usize>();
    let passed = summaries.iter().map(|case| case.passed).sum::<usize>();
    let failed = summaries.iter().map(|case| case.failed).sum::<usize>();
    let timed_out = summaries.iter().map(|case| case.timed_out).sum::<usize>();
    let recorded = summaries.iter().map(|case| case.recorded).sum::<usize>();
    let other = summaries.iter().map(|case| case.other).sum::<usize>();
    json!({
        "total": total,
        "executableCount": executable,
        "passedCount": passed,
        "failedCount": failed,
        "timeoutCount": timed_out,
        "recordedCount": recorded,
        "otherCount": other,
        "passRatePercent": benchmark_pass_rate_percent(passed, executable),
    })
}

fn benchmark_case_summary_json(summary: &BenchmarkCaseSummary) -> Value {
    json!({
        "suite": summary.suite,
        "case": summary.case_name,
        "total": summary.total,
        "executableCount": summary.executable,
        "passedCount": summary.passed,
        "failedCount": summary.failed,
        "timeoutCount": summary.timed_out,
        "recordedCount": summary.recorded,
        "otherCount": summary.other,
        "passRatePercent": benchmark_pass_rate_percent(summary.passed, summary.executable),
        "latest": {
            "status": summary.latest_status,
            "artifactPath": summary.latest_artifact_path,
            "createdAt": summary.latest_created_at,
            "durationMs": summary.latest_duration_ms,
        },
        "duration": {
            "count": summary.duration_count,
            "averageMs": summary.average_duration_ms,
            "minMs": summary.min_duration_ms,
            "maxMs": summary.max_duration_ms,
        },
    })
}

fn benchmark_pass_rate_percent(passed: usize, executable: usize) -> Value {
    if executable == 0 {
        Value::Null
    } else {
        json!(((passed as u64) * 100 + (executable as u64 / 2)) / executable as u64)
    }
}

fn format_benchmark_summary_text(
    workspace: &Path,
    artifact_count: usize,
    summaries: &[BenchmarkCaseSummary],
) -> String {
    let mut lines = vec![
        "deepcli benchmark summary".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("artifacts: {artifact_count}"),
        format!("case count: {}", summaries.len()),
    ];
    if summaries.is_empty() {
        lines.push("history: none".to_string());
        lines.push("next actions:".to_string());
        lines.push("  - deepcli benchmark presets --json".to_string());
        lines.push("  - deepcli benchmark run-suite --json --fail-on-command".to_string());
        lines.push(
            "  - deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string(),
        );
        lines.push("  - deepcli benchmark list --json".to_string());
        return lines.join("\n");
    }
    lines.push("cases:".to_string());
    for summary in summaries {
        let pass_rate = benchmark_pass_rate_percent(summary.passed, summary.executable)
            .as_u64()
            .map(|rate| format!("{rate}%"))
            .unwrap_or_else(|| "n/a".to_string());
        lines.push(format!("  - {}/{}", summary.suite, summary.case_name));
        lines.push(format!(
            "    total={} executable={} passed={} failed={} timeout={} recorded={} pass_rate={}",
            summary.total,
            summary.executable,
            summary.passed,
            summary.failed,
            summary.timed_out,
            summary.recorded,
            pass_rate
        ));
        lines.push(format!(
            "    latest={} duration={}ms artifact={}",
            summary.latest_status,
            summary
                .latest_duration_ms
                .map(|duration| duration.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            summary.latest_artifact_path
        ));
        if summary.duration_count > 0 {
            lines.push(format!(
                "    duration_avg={}ms min={}ms max={}ms samples={}",
                summary
                    .average_duration_ms
                    .map(|duration| duration.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
                summary
                    .min_duration_ms
                    .map(|duration| duration.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
                summary
                    .max_duration_ms
                    .map(|duration| duration.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
                summary.duration_count
            ));
        }
    }
    lines.push("next actions:".to_string());
    lines.extend(
        benchmark_summary_next_actions(workspace)
            .into_iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn format_benchmark_compare_text(
    workspace: &Path,
    artifact_count: usize,
    baseline: &BenchmarkBaselineReport,
    comparisons: &[BenchmarkCaseComparison],
) -> String {
    let status = benchmark_compare_status(artifact_count, baseline, comparisons);
    let baseline_name = baseline.name.as_deref().unwrap_or("none");
    let baseline_path = baseline.path.as_deref().unwrap_or("none");
    let mut lines = vec![
        "deepcli benchmark compare".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("status: {status}"),
        format!("artifacts: {artifact_count}"),
        format!("baseline: {baseline_name} ({baseline_path})"),
        format!("comparison count: {}", comparisons.len()),
    ];
    if comparisons.is_empty() {
        lines.push("comparisons: none".to_string());
        lines.push("next actions:".to_string());
        lines.extend(
            benchmark_compare_next_actions(baseline, artifact_count == 0)
                .into_iter()
                .map(|action| format!("  - {action}")),
        );
        return lines.join("\n");
    }

    lines.push("comparisons:".to_string());
    for comparison in comparisons {
        let current_status = comparison
            .current
            .as_ref()
            .map(|point| point.status.as_str())
            .unwrap_or("none");
        let baseline_status = comparison
            .baseline
            .as_ref()
            .and_then(|case| case.status.as_deref())
            .unwrap_or("none");
        let current_duration = comparison
            .current
            .as_ref()
            .and_then(|point| point.duration_ms)
            .map(|duration| format!("{duration}ms"))
            .unwrap_or_else(|| "n/a".to_string());
        let baseline_duration = comparison
            .baseline
            .as_ref()
            .and_then(|case| case.duration_ms)
            .map(|duration| format!("{duration}ms"))
            .unwrap_or_else(|| "n/a".to_string());
        let duration_delta = comparison
            .duration_delta_ms
            .map(|delta| format!("{delta}ms"))
            .unwrap_or_else(|| "n/a".to_string());
        lines.push(format!(
            "  - {}/{}: current={} baseline={} status_comparison={} duration_delta={} duration_comparison={}",
            comparison.suite,
            comparison.case_name,
            current_status,
            baseline_status,
            comparison.status_comparison,
            duration_delta,
            comparison.duration_comparison
        ));
        lines.push(format!(
            "    current_duration={} baseline_duration={}",
            current_duration, baseline_duration
        ));
        if let Some(current) = &comparison.current {
            lines.push(format!("    current_artifact={}", current.artifact_path));
        }
    }
    lines.push("next actions:".to_string());
    lines.extend(
        benchmark_compare_next_actions(baseline, artifact_count == 0)
            .into_iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn format_benchmark_trends_text(
    workspace: &Path,
    artifact_count: usize,
    trends: &[BenchmarkCaseTrend],
    recent_limit: usize,
) -> String {
    let status = benchmark_trends_status(artifact_count, trends);
    let mut lines = vec![
        "deepcli benchmark trends".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("status: {status}"),
        format!("artifacts: {artifact_count}"),
        format!("case count: {}", trends.len()),
        format!("recent limit: {recent_limit}"),
    ];
    if trends.is_empty() {
        lines.push("trends: none".to_string());
        lines.push("next actions:".to_string());
        lines.extend(
            benchmark_trends_next_actions(workspace, status)
                .into_iter()
                .map(|action| format!("  - {action}")),
        );
        return lines.join("\n");
    }

    lines.push("cases:".to_string());
    for trend in trends {
        let pass_rate = benchmark_pass_rate_percent(trend.passed, trend.executable)
            .as_u64()
            .map(|rate| format!("{rate}%"))
            .unwrap_or_else(|| "n/a".to_string());
        let latest = trend
            .latest
            .as_ref()
            .map(|point| point.status.as_str())
            .unwrap_or("none");
        let previous = trend
            .previous
            .as_ref()
            .map(|point| point.status.as_str())
            .unwrap_or("none");
        let duration_delta = trend
            .duration_delta_ms
            .map(|delta| format!("{delta}ms"))
            .unwrap_or_else(|| "n/a".to_string());
        lines.push(format!(
            "  - {}/{}: status_trend={} duration_trend={} pass_rate={}",
            trend.suite, trend.case_name, trend.status_trend, trend.duration_trend, pass_rate
        ));
        lines.push(format!(
            "    total={} executable={} passed={} failed={} timeout={} recorded={}",
            trend.total,
            trend.executable,
            trend.passed,
            trend.failed,
            trend.timed_out,
            trend.recorded
        ));
        lines.push(format!(
            "    latest={} previous={} duration_delta={}",
            latest, previous, duration_delta
        ));
        if !trend.recent.is_empty() {
            lines.push("    recent:".to_string());
            for point in &trend.recent {
                lines.push(format!(
                    "      - [{}] {} duration={}ms artifact={}",
                    point.status,
                    point.created_at,
                    point
                        .duration_ms
                        .map(|duration| duration.to_string())
                        .unwrap_or_else(|| "n/a".to_string()),
                    point.artifact_path
                ));
            }
        }
    }
    lines.push("next actions:".to_string());
    lines.extend(
        benchmark_trends_next_actions(workspace, status)
            .into_iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}
