use super::*;
use crate::schema_ids;
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub(crate) const DEFAULT_ROUND_SCORE_THRESHOLD: u8 = 90;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RoundOptions {
    json_output: bool,
    output_path: Option<String>,
    fail_on_gaps: bool,
    fail_below: Option<u8>,
    run_benchmark_suite: bool,
    benchmark_suite: BenchmarkRunSuiteOptions,
}

#[derive(Debug, Clone)]
pub(crate) struct RoundGate {
    id: &'static str,
    title: &'static str,
    status: &'static str,
    summary: String,
    next_action: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RoundReport {
    report: String,
    pub(crate) status: &'static str,
    pub(crate) score_threshold: u8,
    pub(crate) scorecard: ScorecardReport,
    pub(crate) benchmark: BenchmarkStatusReport,
    pub(crate) benchmark_run: Option<RoundBenchmarkRun>,
    pub(crate) goal: Option<RoundGoalStatus>,
    pub(crate) gates: Vec<RoundGate>,
    pub(crate) gaps: Vec<String>,
    pub(crate) next_actions: Vec<String>,
    pub(crate) opportunities: Vec<ScorecardOpportunity>,
}

#[derive(Debug, Clone)]
pub(crate) struct RoundBenchmarkRun {
    requested_presets: Vec<String>,
    runs: Vec<BenchmarkRunArtifact>,
    stopped_early: bool,
    fail_fast: bool,
    fail_on_command: bool,
}

pub(crate) struct RoundTextInput<'a> {
    pub(crate) status: &'a str,
    pub(crate) score_threshold: u8,
    pub(crate) scorecard: &'a ScorecardReport,
    pub(crate) benchmark: &'a BenchmarkStatusReport,
    pub(crate) benchmark_run: Option<&'a RoundBenchmarkRun>,
    pub(crate) goal: Option<&'a RoundGoalStatus>,
    pub(crate) gates: &'a [RoundGate],
    pub(crate) gaps: &'a [String],
    pub(crate) next_actions: &'a [String],
    pub(crate) opportunities: &'a [ScorecardOpportunity],
}

pub(crate) fn handle_round(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_round_options(&args)?;
    let score_threshold = options.fail_below.unwrap_or(DEFAULT_ROUND_SCORE_THRESHOLD);
    let benchmark_run = if options.run_benchmark_suite {
        Some(run_round_benchmark_suite(
            workspace,
            config,
            registry,
            &options.benchmark_suite,
        )?)
    } else {
        None
    };
    let report = build_round_report(workspace, config, registry, score_threshold, benchmark_run);
    let output = if options.json_output {
        format_round_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if report
        .benchmark_run
        .as_ref()
        .is_some_and(|run| run.fail_on_command && benchmark_run_suite_status(&run.runs) != "passed")
    {
        return Err(CommandExit::new(output, 1).into());
    }
    if options.fail_on_gaps && report.status != "ready" {
        return Err(CommandExit::new(output, 1).into());
    }
    if options.fail_below.is_some() && report.scorecard.percent < score_threshold {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

fn parse_round_options(args: &[String]) -> Result<RoundOptions> {
    let mut options = RoundOptions::default();
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
            "--fail-on-gaps" | "--fail-on-not-ready" | "--strict" => {
                options.fail_on_gaps = true;
                index += 1;
            }
            "--run-benchmark" | "--run-benchmarks" | "--run-suite" => {
                options.run_benchmark_suite = true;
                index += 1;
            }
            "--preset" => {
                let name = required_arg(args, index + 1, "benchmark preset")?;
                push_benchmark_run_suite_preset(&mut options.benchmark_suite.presets, name)?;
                options.run_benchmark_suite = true;
                index += 2;
            }
            value if value.starts_with("--preset=") => {
                push_benchmark_run_suite_preset(
                    &mut options.benchmark_suite.presets,
                    value.trim_start_matches("--preset="),
                )?;
                options.run_benchmark_suite = true;
                index += 1;
            }
            "--presets" => {
                let raw = required_arg(args, index + 1, "benchmark presets")?;
                push_benchmark_run_suite_presets(&mut options.benchmark_suite.presets, raw)?;
                options.run_benchmark_suite = true;
                index += 2;
            }
            value if value.starts_with("--presets=") => {
                push_benchmark_run_suite_presets(
                    &mut options.benchmark_suite.presets,
                    value.trim_start_matches("--presets="),
                )?;
                options.run_benchmark_suite = true;
                index += 1;
            }
            "--fail-on-command" => {
                options.benchmark_suite.fail_on_command = true;
                options.run_benchmark_suite = true;
                index += 1;
            }
            "--fail-fast" => {
                options.benchmark_suite.fail_fast = true;
                options.run_benchmark_suite = true;
                index += 1;
            }
            "--fail-below" | "--min-score" => {
                let raw = required_arg(args, index + 1, "score percent")?;
                options.fail_below = Some(parse_scorecard_threshold(raw)?);
                index += 2;
            }
            value if value.starts_with("--fail-below=") => {
                options.fail_below = Some(parse_scorecard_threshold(
                    value.trim_start_matches("--fail-below="),
                )?);
                index += 1;
            }
            value if value.starts_with("--min-score=") => {
                options.fail_below = Some(parse_scorecard_threshold(
                    value.trim_start_matches("--min-score="),
                )?);
                index += 1;
            }
            value => bail!("unsupported /round option `{value}`"),
        }
    }
    Ok(options)
}

fn run_round_benchmark_suite(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    options: &BenchmarkRunSuiteOptions,
) -> Result<RoundBenchmarkRun> {
    let requested_presets = benchmark_run_suite_preset_names(options)?;
    let mut runs = Vec::new();
    let mut stopped_early = false;

    for preset_name in &requested_presets {
        let run_options = benchmark_run_options_for_suite_preset(preset_name, options)?;
        let run = execute_benchmark_run_artifact(workspace, config, registry, &run_options)?;
        let failed = run.execution.status != "passed";
        runs.push(run);
        if failed && options.fail_fast {
            stopped_early = true;
            break;
        }
    }

    Ok(RoundBenchmarkRun {
        requested_presets,
        runs,
        stopped_early,
        fail_fast: options.fail_fast,
        fail_on_command: options.fail_on_command,
    })
}

pub(crate) fn build_round_report(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    score_threshold: u8,
    benchmark_run: Option<RoundBenchmarkRun>,
) -> RoundReport {
    let scorecard = build_scorecard_report(workspace, config, registry);
    let benchmark_artifacts = load_benchmark_artifacts(workspace).unwrap_or_default();
    let benchmark = build_benchmark_status_report(workspace, &benchmark_artifacts, Utc::now());
    let benchmark_trends = build_benchmark_case_trends(&benchmark_artifacts, 2);
    let benchmark_trends_status =
        benchmark_trends_status(benchmark_artifacts.len(), &benchmark_trends);
    let goal = build_round_goal_status(workspace);
    let scorecard_threshold_ready = scorecard.percent >= score_threshold;
    let benchmark_ready = benchmark.status == "ready";
    let benchmark_trends_ready =
        !benchmark_ready || !round_benchmark_trends_needs_attention(benchmark_trends_status);
    let goal_ready = goal.as_ref().is_none_or(|goal| goal.ready);

    let mut gaps = Vec::new();
    if scorecard.percent < score_threshold {
        gaps.push(format!(
            "scorecard: score {}% is below round threshold {}%",
            scorecard.percent, score_threshold
        ));
    }
    gaps.extend(scorecard.gaps.iter().cloned());
    gaps.extend(
        benchmark
            .gaps
            .iter()
            .map(|gap| format!("benchmark_evidence: {gap}")),
    );
    if benchmark_ready {
        if let Some(gap) = round_benchmark_trends_gap(benchmark_trends_status) {
            gaps.push(gap);
        }
    }
    if let Some(goal) = &goal {
        gaps.extend(
            goal.blockers
                .iter()
                .map(|blocker| format!("goal_readiness: {blocker}")),
        );
    }
    let gaps = dedup_preserve_order(gaps);

    let mut gates = vec![
        RoundGate {
            id: "scorecard",
            title: "Product Capability Scorecard",
            status: if scorecard_threshold_ready {
                "passed"
            } else {
                "failed"
            },
            summary: if scorecard_threshold_ready {
                if scorecard.gaps.is_empty() {
                    format!(
                        "scorecard is {}% and meets the {}% round threshold with no gaps",
                        scorecard.percent, score_threshold
                    )
                } else {
                    format!(
                        "scorecard is {}% and meets the {}% round threshold; {} gap(s) are reported separately",
                        scorecard.percent,
                        score_threshold,
                        scorecard.gaps.len()
                    )
                }
            } else {
                format!(
                    "scorecard is {}% with {} gap(s); threshold is {}%",
                    scorecard.percent,
                    scorecard.gaps.len(),
                    score_threshold
                )
            },
            next_action: if scorecard_threshold_ready {
                None
            } else {
                Some("deepcli scorecard --json".to_string())
            },
        },
        RoundGate {
            id: "benchmark_evidence",
            title: "Benchmark Evidence",
            status: if benchmark_ready { "passed" } else { "failed" },
            summary: round_benchmark_gate_summary(&benchmark),
            next_action: if let Some(action) = benchmark_freshness_refresh_action(&benchmark) {
                Some(action.to_string())
            } else if benchmark_ready {
                Some("deepcli benchmark summary --json".to_string())
            } else {
                Some("deepcli round --json --run-benchmark --fail-on-command".to_string())
            },
        },
    ];
    if benchmark_ready {
        let trend_needs_attention = round_benchmark_trends_needs_attention(benchmark_trends_status);
        gates.push(RoundGate {
            id: "benchmark_trends",
            title: "Benchmark Trend History",
            status: if trend_needs_attention {
                "failed"
            } else {
                "passed"
            },
            summary: round_benchmark_trends_gate_summary(
                benchmark_trends_status,
                benchmark_trends.len(),
            ),
            next_action: Some(round_benchmark_trends_next_action(benchmark_trends_status)),
        });
    }
    if let Some(goal) = &goal {
        gates.push(RoundGate {
            id: "goal_readiness",
            title: "Goal Readiness",
            status: if goal.ready { "passed" } else { "failed" },
            summary: if goal.ready {
                format!(
                    "goal session {} is ready with no blocker(s)",
                    short_id(&goal.session.id)
                )
            } else {
                format!(
                    "goal session {} has {} blocker(s)",
                    short_id(&goal.session.id),
                    goal.blockers.len()
                )
            },
            next_action: if goal.ready {
                Some("deepcli goal status --json".to_string())
            } else {
                Some("deepcli goal gate --json".to_string())
            },
        });
    }

    let status = if scorecard_threshold_ready
        && benchmark_ready
        && benchmark_trends_ready
        && goal_ready
        && gaps.is_empty()
    {
        "ready"
    } else {
        "needs_attention"
    };

    let mut next_actions = Vec::new();
    if benchmark_ready {
        next_actions.extend(benchmark_freshness_next_actions(&benchmark));
    }
    if !scorecard_threshold_ready || scorecard_has_standalone_round_gaps(&scorecard) {
        next_actions.push("deepcli scorecard --json".to_string());
    }
    if !benchmark_ready {
        next_actions.push("deepcli round --json --run-benchmark --fail-on-command".to_string());
        next_actions.push("deepcli recipes sota --json".to_string());
        next_actions.push("deepcli benchmark run-suite --json --fail-on-command".to_string());
        next_actions.push("deepcli benchmark presets --json".to_string());
        next_actions
            .push("deepcli benchmark run --preset cargo-test --json --fail-on-command".to_string());
        next_actions.push("deepcli benchmark status --json".to_string());
        next_actions.push("deepcli benchmark gate --json".to_string());
        next_actions.push("deepcli benchmark trends --json".to_string());
    }
    if benchmark_ready && round_benchmark_trends_needs_attention(benchmark_trends_status) {
        next_actions.push(round_benchmark_trends_next_action(benchmark_trends_status));
        next_actions.push("deepcli benchmark trends --json".to_string());
        next_actions.push("deepcli benchmark status --json".to_string());
        if benchmark_trends_status == "regression" {
            next_actions.push(
                "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
                    .to_string(),
            );
        }
    }
    if goal.as_ref().is_some_and(|goal| !goal.ready) {
        next_actions.push("deepcli goal status --json".to_string());
        next_actions.push("deepcli goal gate --json".to_string());
    }
    next_actions.push("deepcli preflight --json".to_string());
    next_actions.push("deepcli gate --json".to_string());
    if status == "ready" {
        next_actions.push("deepcli recipes sota --json".to_string());
        next_actions.push(SCORECARD_OPPORTUNITIES_ACTION.to_string());
        next_actions.extend(opportunity_baseline_next_actions(
            sota_baseline_next_actions(workspace),
        ));
    }
    let next_actions = dedup_preserve_order(next_actions);
    let opportunities = if status == "ready" {
        scorecard.opportunities.clone()
    } else {
        Vec::new()
    };
    let report = format_round_text(
        workspace,
        RoundTextInput {
            status,
            score_threshold,
            scorecard: &scorecard,
            benchmark: &benchmark,
            benchmark_run: benchmark_run.as_ref(),
            goal: goal.as_ref(),
            gates: &gates,
            gaps: &gaps,
            next_actions: &next_actions,
            opportunities: &opportunities,
        },
    );

    RoundReport {
        report,
        status,
        score_threshold,
        scorecard,
        benchmark,
        benchmark_run,
        goal,
        gates,
        gaps,
        next_actions,
        opportunities,
    }
}

fn scorecard_has_standalone_round_gaps(scorecard: &ScorecardReport) -> bool {
    scorecard
        .gaps
        .iter()
        .any(|gap| !gap.starts_with("benchmark_evidence:"))
}

pub(crate) fn format_round_text(workspace: &Path, input: RoundTextInput<'_>) -> String {
    let mut lines = vec![
        "deepcli product round".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("status: {}", input.status),
        format!("score threshold: {}%", input.score_threshold),
        format!(
            "scorecard: raw score {}/{} points; normalized score {}/100 ({}, tier={})",
            input.scorecard.score,
            input.scorecard.max_score,
            input.scorecard.percent,
            input.scorecard.status,
            input.scorecard.tier
        ),
        format!(
            "benchmark: status={} ready={} artifacts={}{}",
            input.benchmark.status,
            input.benchmark.status == "ready",
            input.benchmark.artifact_count,
            format_round_benchmark_freshness_suffix(input.benchmark)
        ),
    ];
    if let Some(goal) = input.goal {
        lines.push(format!(
            "goal: ready={} session={} source={} blockers={}",
            goal.ready,
            short_id(&goal.session.id),
            goal.source.as_str(),
            goal.blockers.len()
        ));
    } else {
        lines.push("goal: none".to_string());
    }
    if let Some(run) = input.benchmark_run {
        lines.push(format!(
            "benchmark run: status={} presets={} passed={} failed={} timeout={} stoppedEarly={}",
            benchmark_run_suite_status(&run.runs),
            run.runs.len(),
            run.runs
                .iter()
                .filter(|item| item.execution.status == "passed")
                .count(),
            run.runs
                .iter()
                .filter(|item| item.execution.status == "failed")
                .count(),
            run.runs
                .iter()
                .filter(|item| item.execution.status == "timeout")
                .count(),
            run.stopped_early
        ));
        if !run.runs.is_empty() {
            lines.push("benchmark artifacts:".to_string());
            for item in &run.runs {
                lines.push(format!(
                    "  - {}: {} artifact={}",
                    item.artifact
                        .get("preset")
                        .and_then(Value::as_str)
                        .unwrap_or("<none>"),
                    item.execution.status,
                    item.relative_path
                ));
            }
        }
    }
    lines.push("gates:".to_string());
    for gate in input.gates {
        lines.push(format!(
            "  - {}: {} - {}",
            gate.id, gate.status, gate.summary
        ));
        if let Some(next_action) = &gate.next_action {
            lines.push(format!("    next: {next_action}"));
        }
    }
    if !input.gaps.is_empty() {
        lines.push("gaps:".to_string());
        lines.extend(input.gaps.iter().map(|gap| format!("  - {gap}")));
    }
    if !input.opportunities.is_empty() {
        lines.extend(scorecard_opportunity_summary_text(input.opportunities));
        lines.push("opportunities:".to_string());
        for opportunity in input.opportunities {
            lines.push(format!(
                "  - {}: {} ({})",
                opportunity.id, opportunity.title, opportunity.status
            ));
            lines.push(format!("    summary: {}", opportunity.summary));
            lines.push(format!("    impact: {}", opportunity.impact));
            lines.push(format!("    priority: {}", opportunity.priority));
            lines.push(format!("    effort: {}", opportunity.effort));
            lines.push("    next actions:".to_string());
            lines.extend(
                opportunity
                    .next_actions
                    .iter()
                    .map(|action| format!("      - {action}")),
            );
        }
    }
    lines.push("next actions:".to_string());
    lines.extend(
        input
            .next_actions
            .iter()
            .map(|action| format!("  - {action}")),
    );
    lines.join("\n")
}

fn format_round_json(workspace: &Path, report: &RoundReport) -> Result<String> {
    let gates = report
        .gates
        .iter()
        .map(|gate| {
            json!({
                "id": gate.id,
                "title": gate.title,
                "status": gate.status,
                "summary": &gate.summary,
                "nextAction": gate.next_action.as_deref(),
                "checklist": round_gate_checklist(gate),
            })
        })
        .collect::<Vec<_>>();
    let checklist = scorecard_action_checklist(&report.next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::ROUND_V1,
        "status": report.status,
        "ready": report.status == "ready",
        "summary": round_summary_json(report, &gates, &checklist),
        "workspace": workspace.display().to_string(),
        "version": {
            "package": "deepcli",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "scoreThreshold": report.score_threshold,
        "scorecard": scorecard_summary_json(&report.scorecard),
        "benchmarkStatus": round_benchmark_status_json(&report.benchmark),
        "benchmarkRun": report.benchmark_run.as_ref().map(round_benchmark_run_json),
        "goalStatus": report.goal.as_ref().map(round_goal_status_json),
        "gates": gates,
        "gaps": &report.gaps,
        "nextActions": &report.next_actions,
        "checklist": checklist,
        "recommendedOpportunity": scorecard_recommended_opportunity_json(&report.opportunities),
        "opportunityPriorityCounts": scorecard_opportunity_priority_counts_json(&report.opportunities),
        "opportunityEffortCounts": scorecard_opportunity_effort_counts_json(&report.opportunities),
        "opportunities": scorecard_opportunities_json(&report.opportunities),
        "report": &report.report,
    }))?)
}

fn round_summary_json(report: &RoundReport, gates: &[Value], checklist: &[Value]) -> Value {
    let failed_gate_count = gates
        .iter()
        .filter(|gate| gate.get("status").and_then(Value::as_str) == Some("failed"))
        .count();
    let passed_gate_count = gates
        .iter()
        .filter(|gate| gate.get("status").and_then(Value::as_str) == Some("passed"))
        .count();
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
        "status": report.status,
        "ready": report.status == "ready",
        "scoreThreshold": report.score_threshold,
        "scorecardPercent": report.scorecard.percent,
        "benchmarkStatus": report.benchmark.status,
        "benchmarkFreshnessStatus": benchmark_freshness_status(&report.benchmark),
        "benchmarkFreshnessAgeSeconds": benchmark_freshness_age_seconds(&report.benchmark),
        "benchmarkFreshnessAge": format_benchmark_age(benchmark_freshness_age_seconds(&report.benchmark)),
        "benchmarkRefreshRecommended": benchmark_freshness_refresh_recommended(&report.benchmark),
        "gateCount": gates.len(),
        "passedGateCount": passed_gate_count,
        "failedGateCount": failed_gate_count,
        "gapCount": report.gaps.len(),
        "opportunityCount": report.opportunities.len(),
        "recommendedOpportunityId": report
            .opportunities
            .first()
            .map(|opportunity| Value::from(opportunity.id))
            .unwrap_or(Value::Null),
        "recommendedAction": recommended_action,
        "recommendedActionLabel": recommended_action_label,
    })
}

fn round_gate_checklist(gate: &RoundGate) -> Vec<Value> {
    gate.next_action
        .as_ref()
        .filter(|action| action.starts_with("deepcli ") && !action.contains('<'))
        .map(|command| scorecard_action_checklist(std::slice::from_ref(command)))
        .unwrap_or_default()
}

fn round_benchmark_run_json(run: &RoundBenchmarkRun) -> Value {
    json!({
        "schema": BENCHMARK_SUITE_SCHEMA,
        "status": benchmark_run_suite_status(&run.runs),
        "requestedPresets": &run.requested_presets,
        "presetCount": run.runs.len(),
        "passedCount": run.runs.iter().filter(|item| item.execution.status == "passed").count(),
        "failedCount": run.runs.iter().filter(|item| item.execution.status == "failed").count(),
        "timeoutCount": run.runs.iter().filter(|item| item.execution.status == "timeout").count(),
        "stoppedEarly": run.stopped_early,
        "failFast": run.fail_fast,
        "failOnCommand": run.fail_on_command,
        "artifacts": run.runs.iter().map(benchmark_run_suite_artifact_json).collect::<Vec<_>>(),
    })
}
