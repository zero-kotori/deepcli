use super::*;
use serde_json::{json, Value};

pub(crate) fn round_benchmark_trends_needs_attention(status: &str) -> bool {
    matches!(status, "insufficient_history" | "regression")
}

pub(crate) fn round_benchmark_trends_gate_summary(status: &str, case_count: usize) -> String {
    match status {
        "ok" => format!("benchmark trends status is ok with {case_count} case(s)"),
        "insufficient_history" => format!(
            "benchmark trends status is insufficient_history with {case_count} case(s); run round with the benchmark suite to create comparable history"
        ),
        "regression" => format!(
            "benchmark trends status is regression with {case_count} case(s); inspect benchmark trends before marking the round ready"
        ),
        other => format!("benchmark trends status is {other} with {case_count} case(s)"),
    }
}

pub(crate) fn round_benchmark_trends_gap(status: &str) -> Option<String> {
    match status {
        "insufficient_history" => Some(
            "benchmark_trends: benchmark trends have insufficient history; run `deepcli round --json --run-benchmark --fail-on-command` to create a comparable sample and re-check the round"
                .to_string(),
        ),
        "regression" => Some(
            "benchmark_trends: benchmark trends report a regression; inspect `deepcli benchmark trends --json` before marking the round ready"
                .to_string(),
        ),
        _ => None,
    }
}

pub(crate) fn round_benchmark_trends_next_action(status: &str) -> String {
    match status {
        "insufficient_history" => {
            "deepcli round --json --run-benchmark --fail-on-command".to_string()
        }
        _ => "deepcli benchmark trends --json".to_string(),
    }
}

pub(crate) fn round_benchmark_gate_summary(benchmark: &BenchmarkStatusReport) -> String {
    let mut parts = vec![format!(
        "benchmark status is {} with {} artifact(s)",
        benchmark.status, benchmark.artifact_count
    )];
    if benchmark.latest_meaningful_age_seconds.is_some() {
        parts.push(format!(
            "freshness={} age={}",
            benchmark_freshness_status(benchmark),
            format_benchmark_age(benchmark_freshness_age_seconds(benchmark))
        ));
    }
    for (status, label) in [
        ("missing", "missing presets"),
        ("failed", "failed presets"),
        ("timeout", "timeout presets"),
        ("stale", "stale presets"),
        ("weak", "weak presets"),
    ] {
        let presets = benchmark
            .required_preset_statuses
            .iter()
            .filter(|preset| preset.status == status)
            .map(|preset| preset.preset.as_str())
            .collect::<Vec<_>>();
        if !presets.is_empty() {
            parts.push(format!(
                "{label}: {}",
                format_round_benchmark_preset_names(&presets, 4)
            ));
        }
    }
    parts.join("; ")
}

fn format_round_benchmark_preset_names(names: &[&str], limit: usize) -> String {
    let shown = names.iter().take(limit).copied().collect::<Vec<_>>();
    let mut text = shown.join(", ");
    if names.len() > limit {
        if !text.is_empty() {
            text.push_str(", ");
        }
        text.push_str(&format!("+{} more", names.len() - limit));
    }
    text
}

pub(crate) fn round_benchmark_status_json(report: &BenchmarkStatusReport) -> Value {
    let checklist = benchmark_action_checklist(&report.next_actions);
    json!({
        "schema": BENCHMARK_STATUS_SCHEMA,
        "status": report.status,
        "ready": report.status == "ready",
        "summary": benchmark_status_summary_json(report, &checklist),
        "artifactCount": report.artifact_count,
        "meaningfulArtifactCount": report.meaningful_count,
        "meaningfulExecutableCount": report.meaningful_executable_count,
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
        "gaps": &report.gaps,
        "nextActions": &report.next_actions,
    })
}

pub(crate) fn format_round_benchmark_freshness_suffix(report: &BenchmarkStatusReport) -> String {
    if report.latest_meaningful_age_seconds.is_none() {
        return String::new();
    }
    format!(
        " freshness={} age={}",
        benchmark_freshness_status(report),
        format_benchmark_age(benchmark_freshness_age_seconds(report))
    )
}
