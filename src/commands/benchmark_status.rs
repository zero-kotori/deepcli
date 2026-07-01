use super::*;
use crate::schema_ids;
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub(crate) const BENCHMARK_STATUS_SCHEMA: &str = schema_ids::BENCHMARK_STATUS_V1;
pub(crate) const BENCHMARK_EVIDENCE_REFRESH_AFTER_DAYS: i64 = 1;
pub(crate) const BENCHMARK_EVIDENCE_STALE_AFTER_DAYS: i64 = 7;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkStatusOptions {
    json_output: bool,
    output_path: Option<String>,
    fail_on_not_ready: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkStatusReport {
    pub(crate) status: &'static str,
    pub(crate) artifact_count: usize,
    pub(crate) executable_count: usize,
    pub(crate) passed_count: usize,
    pub(crate) failed_count: usize,
    pub(crate) timeout_count: usize,
    pub(crate) recorded_count: usize,
    pub(crate) other_count: usize,
    pub(crate) smoke_count: usize,
    pub(crate) meaningful_count: usize,
    pub(crate) meaningful_executable_count: usize,
    pub(crate) meaningful_passed_count: usize,
    pub(crate) meaningful_failed_count: usize,
    pub(crate) meaningful_timeout_count: usize,
    pub(crate) latest_artifact: Option<BenchmarkStatusArtifact>,
    pub(crate) latest_meaningful: Option<BenchmarkStatusArtifact>,
    pub(crate) latest_meaningful_age_seconds: Option<i64>,
    pub(crate) seen_meaningful_presets: Vec<String>,
    pub(crate) missing_meaningful_presets: Vec<String>,
    pub(crate) required_preset_statuses: Vec<BenchmarkRequiredPresetStatus>,
    pub(crate) gaps: Vec<String>,
    pub(crate) next_actions: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkRequiredPresetStatus {
    pub(crate) preset: String,
    pub(crate) status: String,
    pub(crate) artifact: Option<BenchmarkStatusArtifact>,
    pub(crate) age_seconds: Option<i64>,
    pub(crate) gap: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkStatusArtifact {
    pub(crate) artifact_path: String,
    pub(crate) created_at: Option<DateTime<Utc>>,
    pub(crate) suite: String,
    pub(crate) case_name: String,
    pub(crate) preset: Option<String>,
    pub(crate) status: String,
    pub(crate) ran_by_deepcli: Option<bool>,
    pub(crate) duration_ms: Option<u64>,
}

pub(crate) fn handle_benchmark_status(workspace: &Path, args: &[String]) -> Result<String> {
    let options = parse_benchmark_status_options(args)?;
    let artifacts = load_benchmark_artifacts(workspace)?;
    let report = build_benchmark_status_report(workspace, &artifacts, Utc::now());
    let output = if options.json_output {
        format_benchmark_status_json(workspace, &report)?
    } else {
        format_benchmark_status_text(workspace, &report)
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_not_ready && report.status != "ready" {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

fn parse_benchmark_status_options(args: &[String]) -> Result<BenchmarkStatusOptions> {
    let mut options = BenchmarkStatusOptions::default();
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
            "--fail-on-not-ready" | "--fail-on-gaps" | "--strict" => {
                options.fail_on_not_ready = true;
                index += 1;
            }
            value => bail!("unsupported /benchmark status option `{value}`"),
        }
    }
    Ok(options)
}

pub(crate) fn build_benchmark_status_report(
    _workspace: &Path,
    artifacts: &[BenchmarkArtifact],
    now: DateTime<Utc>,
) -> BenchmarkStatusReport {
    let mut report = BenchmarkStatusReport {
        status: "missing",
        artifact_count: artifacts.len(),
        executable_count: 0,
        passed_count: 0,
        failed_count: 0,
        timeout_count: 0,
        recorded_count: 0,
        other_count: 0,
        smoke_count: 0,
        meaningful_count: 0,
        meaningful_executable_count: 0,
        meaningful_passed_count: 0,
        meaningful_failed_count: 0,
        meaningful_timeout_count: 0,
        latest_artifact: artifacts.first().map(benchmark_status_artifact),
        latest_meaningful: None,
        latest_meaningful_age_seconds: None,
        seen_meaningful_presets: benchmark_seen_meaningful_presets(artifacts),
        missing_meaningful_presets: Vec::new(),
        required_preset_statuses: Vec::new(),
        gaps: Vec::new(),
        next_actions: Vec::new(),
    };
    report.missing_meaningful_presets = MEANINGFUL_BENCHMARK_PRESETS
        .iter()
        .filter(|preset| {
            !report
                .seen_meaningful_presets
                .iter()
                .any(|seen| seen == **preset)
        })
        .map(|preset| (*preset).to_string())
        .collect();

    for artifact in artifacts {
        let status = benchmark_artifact_status(&artifact.value);
        match status {
            "passed" => {
                report.executable_count += 1;
                report.passed_count += 1;
            }
            "failed" => {
                report.executable_count += 1;
                report.failed_count += 1;
            }
            "timeout" => {
                report.executable_count += 1;
                report.timeout_count += 1;
            }
            "recorded" => report.recorded_count += 1,
            _ => report.other_count += 1,
        }

        if benchmark_artifact_is_smoke(&artifact.value) {
            report.smoke_count += 1;
        }

        if !benchmark_artifact_is_meaningful(&artifact.value) {
            continue;
        }
        report.meaningful_count += 1;
        match status {
            "passed" => {
                report.meaningful_executable_count += 1;
                report.meaningful_passed_count += 1;
            }
            "failed" => {
                report.meaningful_executable_count += 1;
                report.meaningful_failed_count += 1;
            }
            "timeout" => {
                report.meaningful_executable_count += 1;
                report.meaningful_timeout_count += 1;
            }
            _ => {}
        }
        if report.latest_meaningful.is_none() && matches!(status, "passed" | "failed" | "timeout") {
            report.latest_meaningful = Some(benchmark_status_artifact(artifact));
        }
    }

    if let Some(latest) = &report.latest_meaningful {
        report.latest_meaningful_age_seconds = latest
            .created_at
            .map(|created_at| now.signed_duration_since(created_at).num_seconds().max(0));
    }

    report.required_preset_statuses = benchmark_required_preset_statuses(artifacts, now);
    report.status = classify_benchmark_status(&report);
    report.gaps = benchmark_status_gaps(&report);
    report.next_actions = benchmark_status_next_actions(&report);
    report
}

fn benchmark_status_next_actions(report: &BenchmarkStatusReport) -> Vec<String> {
    let mut actions = benchmark_freshness_next_actions(report);
    actions.push("deepcli recipes sota --json".to_string());
    if report.status == "ready" {
        actions.push("deepcli benchmark baselines --json".to_string());
    }
    actions.extend([
        "deepcli benchmark presets --json".to_string(),
        "deepcli benchmark run-suite --json --fail-on-command".to_string(),
        "deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string(),
        "deepcli benchmark gate --json".to_string(),
        "deepcli benchmark summary --json".to_string(),
        "deepcli benchmark trends --json".to_string(),
    ]);
    if report.artifact_count > 0 {
        actions.push("deepcli benchmark clean --dry-run --json".to_string());
    }
    actions.push("deepcli scorecard --json".to_string());
    dedup_preserve_order(actions)
}

pub(crate) fn benchmark_freshness_next_actions(report: &BenchmarkStatusReport) -> Vec<String> {
    benchmark_freshness_refresh_action(report)
        .map(|action| vec![action.to_string()])
        .unwrap_or_default()
}

fn classify_benchmark_status(report: &BenchmarkStatusReport) -> &'static str {
    if report.artifact_count == 0 {
        return "missing";
    }
    if report.meaningful_executable_count == 0 {
        return "weak";
    }
    if report
        .required_preset_statuses
        .iter()
        .any(|preset| matches!(preset.status.as_str(), "failed" | "timeout"))
    {
        return "failing";
    }
    if report
        .required_preset_statuses
        .iter()
        .any(|preset| matches!(preset.status.as_str(), "missing" | "weak"))
    {
        return "incomplete";
    }
    if report
        .required_preset_statuses
        .iter()
        .any(|preset| preset.status == "stale")
    {
        return "stale";
    }
    if report
        .required_preset_statuses
        .iter()
        .all(|preset| preset.status == "passed")
    {
        "ready"
    } else {
        "weak"
    }
}

fn benchmark_status_gaps(report: &BenchmarkStatusReport) -> Vec<String> {
    let required_gaps = benchmark_required_preset_gaps(report);
    if matches!(report.status, "incomplete" | "failing" | "stale") && !required_gaps.is_empty() {
        return required_gaps;
    }
    match report.status {
        "missing" => vec![
            "no local benchmark artifact found under .deepcli/benchmarks".to_string(),
        ],
        "weak" if report.smoke_count == report.artifact_count && report.artifact_count > 0 => {
            vec![
                "only smoke benchmark artifacts found; smoke validates plumbing but not product capability"
                    .to_string(),
            ]
        }
        "weak" if report.meaningful_count > 0 => vec![
            "meaningful benchmark artifacts are record-only or unknown; no recent executable pass/fail evidence was found"
                .to_string(),
        ],
        "weak" => vec![
            "no meaningful benchmark preset evidence found; custom or record-only artifacts are not enough"
                .to_string(),
        ],
        "failing" => {
            let detail = report
                .latest_meaningful
                .as_ref()
                .map(|artifact| {
                    format!(
                        "{} status={}",
                        artifact.artifact_path, artifact.status
                    )
                })
                .unwrap_or_else(|| "<unknown>".to_string());
            vec![format!("latest meaningful benchmark failed or timed out: {detail}")]
        }
        "stale" => {
            let detail = report
                .latest_meaningful
                .as_ref()
                .map(|artifact| artifact.artifact_path.clone())
                .unwrap_or_else(|| "<unknown>".to_string());
            vec![format!(
                "latest meaningful benchmark is older than {} days: {detail}",
                BENCHMARK_EVIDENCE_STALE_AFTER_DAYS
            )]
        }
        _ => Vec::new(),
    }
}

fn benchmark_required_preset_gaps(report: &BenchmarkStatusReport) -> Vec<String> {
    report
        .required_preset_statuses
        .iter()
        .filter_map(|preset| preset.gap.clone())
        .collect()
}

fn benchmark_required_preset_statuses(
    artifacts: &[BenchmarkArtifact],
    now: DateTime<Utc>,
) -> Vec<BenchmarkRequiredPresetStatus> {
    MEANINGFUL_BENCHMARK_PRESETS
        .iter()
        .map(|preset_name| match benchmark_preset_by_name(preset_name) {
            Ok(preset) => benchmark_required_preset_status(artifacts, now, preset),
            Err(_) => BenchmarkRequiredPresetStatus {
                preset: (*preset_name).to_string(),
                status: "missing".to_string(),
                artifact: None,
                age_seconds: None,
                gap: Some(format!(
                    "required benchmark preset is not registered: {preset_name}"
                )),
            },
        })
        .collect()
}

fn benchmark_required_preset_status(
    artifacts: &[BenchmarkArtifact],
    now: DateTime<Utc>,
    preset: &BenchmarkPreset,
) -> BenchmarkRequiredPresetStatus {
    let latest_any = artifacts
        .iter()
        .find(|artifact| benchmark_artifact_matches_preset(&artifact.value, preset));
    let latest_executable = artifacts.iter().find(|artifact| {
        benchmark_artifact_matches_preset(&artifact.value, preset)
            && matches!(
                benchmark_artifact_status(&artifact.value),
                "passed" | "failed" | "timeout"
            )
    });
    let Some(artifact) = latest_executable else {
        return if let Some(artifact) = latest_any {
            BenchmarkRequiredPresetStatus {
                preset: preset.name.to_string(),
                status: "weak".to_string(),
                artifact: Some(benchmark_status_artifact(artifact)),
                age_seconds: artifact
                    .created_at
                    .map(|created_at| now.signed_duration_since(created_at).num_seconds().max(0)),
                gap: Some(format!(
                    "required benchmark preset `{}` has only record-only or unknown evidence; run `{}`",
                    preset.name, BENCHMARK_RUN_SUITE_REMEDIATION_ACTION
                )),
            }
        } else {
            BenchmarkRequiredPresetStatus {
                preset: preset.name.to_string(),
                status: "missing".to_string(),
                artifact: None,
                age_seconds: None,
                gap: Some(format!(
                    "missing required benchmark preset `{}`; run `{}`",
                    preset.name, BENCHMARK_RUN_SUITE_REMEDIATION_ACTION
                )),
            }
        };
    };

    let status = benchmark_artifact_status(&artifact.value);
    let age_seconds = artifact
        .created_at
        .map(|created_at| now.signed_duration_since(created_at).num_seconds().max(0));
    let stale_after_seconds = BENCHMARK_EVIDENCE_STALE_AFTER_DAYS * 24 * 60 * 60;
    let artifact_path = artifact.relative_path.clone();
    let (status, gap) = match status {
        "passed" if age_seconds.is_some_and(|age| age > stale_after_seconds) => (
            "stale".to_string(),
            Some(format!(
                "required benchmark preset `{}` is older than {} days: {}",
                preset.name, BENCHMARK_EVIDENCE_STALE_AFTER_DAYS, artifact_path
            )),
        ),
        "passed" => ("passed".to_string(), None),
        "failed" | "timeout" => (
            status.to_string(),
            Some(format!(
                "required benchmark preset `{}` latest executable artifact is {}: {}",
                preset.name, status, artifact_path
            )),
        ),
        _ => (
            "weak".to_string(),
            Some(format!(
                "required benchmark preset `{}` has non-executable latest evidence; run `{}`",
                preset.name, BENCHMARK_RUN_SUITE_REMEDIATION_ACTION
            )),
        ),
    };

    BenchmarkRequiredPresetStatus {
        preset: preset.name.to_string(),
        status,
        artifact: Some(benchmark_status_artifact(artifact)),
        age_seconds,
        gap,
    }
}

fn benchmark_status_artifact(artifact: &BenchmarkArtifact) -> BenchmarkStatusArtifact {
    BenchmarkStatusArtifact {
        artifact_path: artifact.relative_path.clone(),
        created_at: artifact.created_at,
        suite: artifact_string_field(&artifact.value, "suite", "<unknown>"),
        case_name: artifact_string_field(&artifact.value, "case", "<unknown>"),
        preset: benchmark_artifact_preset(&artifact.value).map(ToString::to_string),
        status: benchmark_artifact_status(&artifact.value).to_string(),
        ran_by_deepcli: benchmark_artifact_ran_by_deepcli(&artifact.value),
        duration_ms: benchmark_artifact_duration_ms(&artifact.value),
    }
}

pub(crate) fn benchmark_status_artifact_json(artifact: &Option<BenchmarkStatusArtifact>) -> Value {
    let Some(artifact) = artifact else {
        return Value::Null;
    };
    json!({
        "artifactPath": artifact.artifact_path,
        "createdAt": artifact.created_at.map(|time| time.to_rfc3339()),
        "suite": artifact.suite,
        "case": artifact.case_name,
        "preset": artifact.preset,
        "status": artifact.status,
        "ranByDeepcli": artifact.ran_by_deepcli,
        "durationMs": artifact.duration_ms,
    })
}

pub(crate) fn format_benchmark_status_json(
    workspace: &Path,
    report: &BenchmarkStatusReport,
) -> Result<String> {
    let next_actions = &report.next_actions;
    let checklist = benchmark_action_checklist(next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": BENCHMARK_STATUS_SCHEMA,
        "status": report.status,
        "ready": report.status == "ready",
        "hasGaps": !report.gaps.is_empty(),
        "workspace": workspace.display().to_string(),
        "summary": benchmark_status_summary_json(report, &checklist),
        "staleAfterDays": BENCHMARK_EVIDENCE_STALE_AFTER_DAYS,
        "artifactCount": report.artifact_count,
        "totals": {
            "executableCount": report.executable_count,
            "passedCount": report.passed_count,
            "failedCount": report.failed_count,
            "timeoutCount": report.timeout_count,
            "recordedCount": report.recorded_count,
            "otherCount": report.other_count,
            "smokeCount": report.smoke_count,
        },
        "meaningful": {
            "artifactCount": report.meaningful_count,
            "executableCount": report.meaningful_executable_count,
            "passedCount": report.meaningful_passed_count,
            "failedCount": report.meaningful_failed_count,
            "timeoutCount": report.meaningful_timeout_count,
        },
        "presetCoverage": {
            "required": MEANINGFUL_BENCHMARK_PRESETS,
            "seen": report.seen_meaningful_presets,
            "missing": report.missing_meaningful_presets,
            "requiredStatus": report.required_preset_statuses
                .iter()
                .map(benchmark_required_preset_status_json)
                .collect::<Vec<_>>(),
        },
        "latestArtifact": benchmark_status_artifact_json(&report.latest_artifact),
        "latestMeaningfulArtifact": benchmark_status_artifact_json(&report.latest_meaningful),
        "latestMeaningfulAgeSeconds": report.latest_meaningful_age_seconds,
        "freshness": benchmark_freshness_json(report),
        "gaps": report.gaps,
        "nextActions": next_actions,
        "checklist": checklist,
        "report": format_benchmark_status_text(workspace, report),
    }))?)
}

pub(crate) fn benchmark_status_summary_json(
    report: &BenchmarkStatusReport,
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
        "status": report.status,
        "ready": report.status == "ready",
        "artifactCount": report.artifact_count,
        "meaningfulArtifactCount": report.meaningful_count,
        "meaningfulExecutableCount": report.meaningful_executable_count,
        "meaningfulPassedCount": report.meaningful_passed_count,
        "meaningfulFailedCount": report.meaningful_failed_count,
        "meaningfulTimeoutCount": report.meaningful_timeout_count,
        "freshnessStatus": benchmark_freshness_status(report),
        "freshnessAgeSeconds": benchmark_freshness_age_seconds(report),
        "freshnessAge": format_benchmark_age(benchmark_freshness_age_seconds(report)),
        "refreshRecommended": benchmark_freshness_refresh_recommended(report),
        "refreshAction": benchmark_freshness_refresh_action(report),
        "requiredPresetCount": MEANINGFUL_BENCHMARK_PRESETS.len(),
        "requiredReadyCount": report
            .required_preset_statuses
            .iter()
            .filter(|preset| preset.status == "passed")
            .count(),
        "requiredMissingCount": report
            .required_preset_statuses
            .iter()
            .filter(|preset| preset.status == "missing")
            .count(),
        "requiredProblemCount": report
            .required_preset_statuses
            .iter()
            .filter(|preset| matches!(preset.status.as_str(), "failed" | "timeout"))
            .count(),
        "gapCount": report.gaps.len(),
        "recommendedAction": recommended_action,
        "recommendedActionLabel": recommended_action_label,
    })
}

pub(crate) fn benchmark_required_preset_status_json(
    status: &BenchmarkRequiredPresetStatus,
) -> Value {
    json!({
        "preset": status.preset,
        "status": status.status,
        "artifact": benchmark_status_artifact_json(&status.artifact),
        "ageSeconds": status.age_seconds,
        "gap": status.gap,
    })
}

pub(crate) fn benchmark_freshness_json(report: &BenchmarkStatusReport) -> Value {
    json!({
        "status": benchmark_freshness_status(report),
        "ageSeconds": benchmark_freshness_age_seconds(report),
        "age": format_benchmark_age(benchmark_freshness_age_seconds(report)),
        "latestMeaningfulAgeSeconds": report.latest_meaningful_age_seconds,
        "latestMeaningfulAge": format_benchmark_age(report.latest_meaningful_age_seconds),
        "oldestRequiredAgeSeconds": benchmark_oldest_required_age_seconds(report),
        "oldestRequiredAge": format_benchmark_age(benchmark_oldest_required_age_seconds(report)),
        "refreshAfterDays": BENCHMARK_EVIDENCE_REFRESH_AFTER_DAYS,
        "staleAfterDays": BENCHMARK_EVIDENCE_STALE_AFTER_DAYS,
        "refreshRecommended": benchmark_freshness_refresh_recommended(report),
        "refreshAction": benchmark_freshness_refresh_action(report),
    })
}

pub(crate) fn benchmark_freshness_status(report: &BenchmarkStatusReport) -> &'static str {
    let Some(age_seconds) = benchmark_freshness_age_seconds(report) else {
        return "missing";
    };
    if report.status == "stale" || age_seconds > benchmark_stale_after_seconds() {
        "stale"
    } else if age_seconds >= benchmark_refresh_after_seconds() {
        "aging"
    } else {
        "fresh"
    }
}

pub(crate) fn benchmark_freshness_refresh_recommended(report: &BenchmarkStatusReport) -> bool {
    matches!(benchmark_freshness_status(report), "aging" | "stale")
}

pub(crate) fn benchmark_freshness_refresh_action(
    report: &BenchmarkStatusReport,
) -> Option<&'static str> {
    benchmark_freshness_refresh_recommended(report)
        .then_some(SCORECARD_BENCHMARK_REMEDIATION_ACTION)
}

pub(crate) fn benchmark_freshness_age_seconds(report: &BenchmarkStatusReport) -> Option<i64> {
    benchmark_oldest_required_age_seconds(report).or(report.latest_meaningful_age_seconds)
}

fn benchmark_oldest_required_age_seconds(report: &BenchmarkStatusReport) -> Option<i64> {
    report
        .required_preset_statuses
        .iter()
        .filter_map(|preset| preset.age_seconds)
        .max()
}

fn benchmark_refresh_after_seconds() -> i64 {
    BENCHMARK_EVIDENCE_REFRESH_AFTER_DAYS * 24 * 60 * 60
}

fn benchmark_stale_after_seconds() -> i64 {
    BENCHMARK_EVIDENCE_STALE_AFTER_DAYS * 24 * 60 * 60
}

pub(crate) fn format_benchmark_age(age_seconds: Option<i64>) -> String {
    let Some(age_seconds) = age_seconds else {
        return "unknown".to_string();
    };
    let age_seconds = age_seconds.max(0);
    if age_seconds < 60 {
        return format!("{age_seconds}s");
    }
    let minutes = age_seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        let remaining_minutes = minutes % 60;
        return if remaining_minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {remaining_minutes}m")
        };
    }
    let days = hours / 24;
    let remaining_hours = hours % 24;
    if remaining_hours == 0 {
        format!("{days}d")
    } else {
        format!("{days}d {remaining_hours}h")
    }
}

pub(crate) fn format_benchmark_status_text(
    workspace: &Path,
    report: &BenchmarkStatusReport,
) -> String {
    let mut lines = vec![
        "deepcli benchmark status".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("status: {}", report.status),
        format!("ready: {}", report.status == "ready"),
        format!("artifacts: {}", report.artifact_count),
        format!(
            "totals: executable={} passed={} failed={} timeout={} recorded={} smoke={}",
            report.executable_count,
            report.passed_count,
            report.failed_count,
            report.timeout_count,
            report.recorded_count,
            report.smoke_count
        ),
        format!(
            "meaningful: artifacts={} executable={} passed={} failed={} timeout={}",
            report.meaningful_count,
            report.meaningful_executable_count,
            report.meaningful_passed_count,
            report.meaningful_failed_count,
            report.meaningful_timeout_count
        ),
    ];
    if report.latest_meaningful_age_seconds.is_some() {
        lines.push(format!(
            "freshness: {} age={} refreshRecommended={} refreshAfter={}d staleAfter={}d",
            benchmark_freshness_status(report),
            format_benchmark_age(benchmark_freshness_age_seconds(report)),
            benchmark_freshness_refresh_recommended(report),
            BENCHMARK_EVIDENCE_REFRESH_AFTER_DAYS,
            BENCHMARK_EVIDENCE_STALE_AFTER_DAYS
        ));
    }
    if let Some(latest) = &report.latest_artifact {
        lines.push(format!(
            "latest artifact: {} status={} preset={} case={}",
            latest.artifact_path,
            latest.status,
            latest.preset.as_deref().unwrap_or("n/a"),
            latest.case_name
        ));
    }
    if let Some(latest) = &report.latest_meaningful {
        let age = report
            .latest_meaningful_age_seconds
            .map(|value| format!("{} ({value}s)", format_benchmark_age(Some(value))))
            .unwrap_or_else(|| "unknown".to_string());
        lines.push(format!(
            "latest meaningful: {} status={} age={}",
            latest.artifact_path, latest.status, age
        ));
    }
    lines.push(format!(
        "meaningful presets seen: {}",
        if report.seen_meaningful_presets.is_empty() {
            "none".to_string()
        } else {
            report.seen_meaningful_presets.join(", ")
        }
    ));
    if !report.required_preset_statuses.is_empty() {
        lines.push("required presets:".to_string());
        for preset in &report.required_preset_statuses {
            let artifact = preset
                .artifact
                .as_ref()
                .map(|artifact| artifact.artifact_path.as_str())
                .unwrap_or("none");
            lines.push(format!(
                "  - {}: status={} artifact={}",
                preset.preset, preset.status, artifact
            ));
        }
    }
    if !report.gaps.is_empty() {
        lines.push("gaps:".to_string());
        lines.extend(report.gaps.iter().map(|gap| format!("  - {gap}")));
    }
    lines.push("next actions:".to_string());
    lines.extend(
        report
            .next_actions
            .iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn benchmark_seen_meaningful_presets(artifacts: &[BenchmarkArtifact]) -> Vec<String> {
    MEANINGFUL_BENCHMARK_PRESETS
        .iter()
        .filter(|preset| {
            artifacts.iter().any(|artifact| {
                benchmark_artifact_canonical_preset(&artifact.value)
                    .is_some_and(|seen| seen == **preset)
            })
        })
        .map(|preset| (*preset).to_string())
        .collect()
}

fn benchmark_artifact_is_smoke(value: &Value) -> bool {
    benchmark_artifact_canonical_preset(value) == Some("smoke")
        || artifact_string_field(value, "case", "").eq_ignore_ascii_case("smoke")
}

fn benchmark_artifact_is_meaningful(value: &Value) -> bool {
    if let Some(preset) = benchmark_artifact_canonical_preset(value) {
        return MEANINGFUL_BENCHMARK_PRESETS.contains(&preset);
    }
    benchmark_artifact_matches_meaningful_command(value)
}

fn benchmark_artifact_matches_meaningful_command(value: &Value) -> bool {
    BENCHMARK_PRESETS
        .iter()
        .filter(|preset| MEANINGFUL_BENCHMARK_PRESETS.contains(&preset.name))
        .any(|preset| benchmark_artifact_matches_preset_command(value, preset))
}

pub(crate) fn benchmark_artifact_matches_preset(value: &Value, preset: &BenchmarkPreset) -> bool {
    benchmark_artifact_canonical_preset(value) == Some(preset.name)
        || benchmark_artifact_matches_preset_command(value, preset)
}

fn benchmark_artifact_matches_preset_command(value: &Value, preset: &BenchmarkPreset) -> bool {
    let case_name = artifact_string_field(value, "case", "");
    let commands = benchmark_artifact_declared_commands(value);
    case_name == preset.case_name
        && commands
            .iter()
            .any(|command| command.trim() == preset.command)
}

fn benchmark_artifact_declared_commands(value: &Value) -> Vec<String> {
    value
        .get("declaredCommands")
        .and_then(Value::as_array)
        .map(|commands| {
            commands
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn benchmark_artifact_preset(value: &Value) -> Option<&str> {
    value.get("preset").and_then(Value::as_str)
}

fn benchmark_artifact_canonical_preset(value: &Value) -> Option<&'static str> {
    let preset = benchmark_artifact_preset(value)?;
    benchmark_preset_by_name(preset)
        .ok()
        .map(|preset| preset.name)
}

fn benchmark_artifact_ran_by_deepcli(value: &Value) -> Option<bool> {
    value
        .get("execution")
        .and_then(|execution| execution.get("ranByDeepcli"))
        .and_then(Value::as_bool)
}
