use super::*;
use crate::schema_ids;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) const MEANINGFUL_BENCHMARK_PRESETS: &[&str] =
    &["cargo-test", "preflight-quick", "selftest", "scorecard"];
pub(crate) const DEFAULT_BENCHMARK_RUN_SUITE_PRESETS: &[&str] =
    &["cargo-test", "preflight-quick", "selftest", "scorecard"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkPresetsOptions {
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BenchmarkPreset {
    pub(crate) name: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) title: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) suite: &'static str,
    pub(crate) case_name: &'static str,
    pub(crate) command: &'static str,
    pub(crate) timeout_seconds: u64,
}

pub(crate) const BENCHMARK_PRESETS: &[BenchmarkPreset] = &[
    BenchmarkPreset {
        name: "cargo-test",
        aliases: &["cargo", "test", "tests"],
        title: "Cargo Test",
        summary: "Run the Rust test suite as product benchmark evidence.",
        suite: "product",
        case_name: "cargo-test",
        command: "cargo test",
        timeout_seconds: 600,
    },
    BenchmarkPreset {
        name: "preflight-quick",
        aliases: &["quick", "preflight"],
        title: "Quick Preflight",
        summary: "Run a quicker local release-readiness check without the slow gate.",
        suite: "product",
        case_name: "preflight-quick",
        command: "deepcli preflight --quick --json",
        timeout_seconds: 300,
    },
    BenchmarkPreset {
        name: "selftest",
        aliases: &["install", "readiness"],
        title: "Selftest",
        summary: "Run deepcli's local install and command-surface readiness test.",
        suite: "product",
        case_name: "selftest",
        command: "deepcli selftest --json",
        timeout_seconds: 120,
    },
    BenchmarkPreset {
        name: "scorecard",
        aliases: &["sota", "rubric"],
        title: "Scorecard",
        summary: "Run the local product capability scorecard as benchmark evidence.",
        suite: "product",
        case_name: "scorecard",
        command: "deepcli scorecard --json",
        timeout_seconds: 120,
    },
    BenchmarkPreset {
        name: "smoke",
        aliases: &["hello"],
        title: "Smoke",
        summary: "Run a tiny command to validate benchmark artifact plumbing.",
        suite: "product",
        case_name: "smoke",
        command: "printf deepcli-benchmark-smoke",
        timeout_seconds: 30,
    },
];

pub(crate) fn handle_benchmark_presets(workspace: &Path, args: &[String]) -> Result<String> {
    let options = parse_benchmark_presets_options(args)?;
    let output = if options.json_output {
        format_benchmark_presets_json(workspace)?
    } else {
        format_benchmark_presets_text(workspace)
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_benchmark_presets_options(args: &[String]) -> Result<BenchmarkPresetsOptions> {
    let mut options = BenchmarkPresetsOptions::default();
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
            value => bail!("unsupported /benchmark presets option `{value}`"),
        }
    }
    Ok(options)
}

pub(crate) fn benchmark_preset_by_name(name: &str) -> Result<&'static BenchmarkPreset> {
    let normalized = name.trim().to_ascii_lowercase();
    BENCHMARK_PRESETS
        .iter()
        .find(|preset| {
            preset.name == normalized || preset.aliases.iter().any(|alias| *alias == normalized)
        })
        .ok_or_else(|| {
            let names = BENCHMARK_PRESETS
                .iter()
                .map(|preset| preset.name)
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::anyhow!("unknown benchmark preset `{name}`; expected one of: {names}")
        })
}

fn format_benchmark_presets_json(workspace: &Path) -> Result<String> {
    let next_actions = benchmark_presets_next_actions(workspace);
    let checklist = benchmark_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::BENCHMARK_PRESETS_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "presetCount": BENCHMARK_PRESETS.len(),
        "summary": benchmark_presets_summary_json(&checklist),
        "presets": BENCHMARK_PRESETS
            .iter()
            .map(benchmark_preset_json)
            .collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": checklist,
    }))?)
}

fn benchmark_presets_next_actions(workspace: &Path) -> Vec<String> {
    let mut actions = vec![
        "deepcli benchmark run-suite --json --fail-on-command".to_string(),
        "deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string(),
        "deepcli benchmark run --preset preflight-quick --json --fail-on-command".to_string(),
        "deepcli benchmark status --json".to_string(),
        "deepcli benchmark summary --json".to_string(),
        "deepcli benchmark trends --json".to_string(),
    ];
    actions.extend(sota_baseline_next_actions(workspace));
    actions.extend([
        "deepcli benchmark clean --dry-run --json".to_string(),
        "deepcli scorecard --json".to_string(),
    ]);
    actions
}

fn benchmark_presets_summary_json(checklist: &[Value]) -> Value {
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
    let optional_preset_count = BENCHMARK_PRESETS
        .iter()
        .filter(|preset| !MEANINGFUL_BENCHMARK_PRESETS.contains(&preset.name))
        .count();

    json!({
        "status": "ok",
        "presetCount": BENCHMARK_PRESETS.len(),
        "defaultSuitePresetCount": DEFAULT_BENCHMARK_RUN_SUITE_PRESETS.len(),
        "requiredEvidencePresetCount": MEANINGFUL_BENCHMARK_PRESETS.len(),
        "optionalPresetCount": optional_preset_count,
        "defaultSuiteAction": "deepcli benchmark run-suite --json --fail-on-command",
        "defaultSuitePresets": DEFAULT_BENCHMARK_RUN_SUITE_PRESETS,
        "requiredEvidencePresets": MEANINGFUL_BENCHMARK_PRESETS,
        "recommendedAction": recommended_action,
        "recommendedActionLabel": recommended_action_label,
    })
}

fn benchmark_preset_json(preset: &BenchmarkPreset) -> Value {
    json!({
        "name": preset.name,
        "aliases": preset.aliases,
        "title": preset.title,
        "summary": preset.summary,
        "suite": preset.suite,
        "case": preset.case_name,
        "command": preset.command,
        "timeoutSeconds": preset.timeout_seconds,
        "defaultSuite": DEFAULT_BENCHMARK_RUN_SUITE_PRESETS.contains(&preset.name),
        "requiredEvidence": MEANINGFUL_BENCHMARK_PRESETS.contains(&preset.name),
    })
}

fn format_benchmark_presets_text(workspace: &Path) -> String {
    let mut lines = vec![
        "deepcli benchmark presets".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("count: {}", BENCHMARK_PRESETS.len()),
        "presets:".to_string(),
    ];
    for preset in BENCHMARK_PRESETS {
        lines.push(format!("  - {} ({})", preset.name, preset.title));
        lines.push(format!(
            "    suite={} case={} timeout={}s",
            preset.suite, preset.case_name, preset.timeout_seconds
        ));
        lines.push(format!("    summary: {}", preset.summary));
        lines.push(format!("    command: {}", preset.command));
    }
    lines.push("next actions:".to_string());
    lines.push("  - deepcli benchmark run-suite --json --fail-on-command".to_string());
    lines
        .push("  - deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string());
    lines.push("  - deepcli benchmark status --json".to_string());
    lines.push("  - deepcli benchmark summary --json".to_string());
    lines.push("  - deepcli benchmark trends --json".to_string());
    lines.extend(
        sota_baseline_next_actions(workspace)
            .into_iter()
            .map(|action| format!("  - {action}")),
    );
    lines.push("  - deepcli benchmark clean --dry-run --json".to_string());
    lines.join("\n")
}
