use super::*;
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ScorecardOptions {
    json_output: bool,
    output_path: Option<String>,
    fail_below: Option<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScorecardCategory {
    id: &'static str,
    title: &'static str,
    summary: &'static str,
    score: u16,
    max_score: u16,
    evidence: Vec<String>,
    gaps: Vec<String>,
    next_actions: Vec<String>,
    priority_next_actions: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScorecardReport {
    pub(crate) report: String,
    pub(crate) status: &'static str,
    pub(crate) tier: &'static str,
    pub(crate) score: u16,
    pub(crate) max_score: u16,
    pub(crate) percent: u8,
    pub(crate) categories: Vec<ScorecardCategory>,
    pub(crate) gaps: Vec<String>,
    pub(crate) next_actions: Vec<String>,
    pub(crate) opportunities: Vec<ScorecardOpportunity>,
}

pub(crate) const SCORECARD_BENCHMARK_REMEDIATION_ACTION: &str =
    "deepcli round --json --run-benchmark --fail-on-command";

pub(crate) fn handle_scorecard(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_scorecard_options(&args)?;
    let report = build_scorecard_report(workspace, config, registry);
    let output = if options.json_output {
        format_scorecard_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if options
        .fail_below
        .is_some_and(|threshold| report.percent < threshold)
    {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

fn parse_scorecard_options(args: &[String]) -> Result<ScorecardOptions> {
    let mut options = ScorecardOptions::default();
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
            value => bail!("unsupported /scorecard option `{value}`"),
        }
    }
    Ok(options)
}

pub(crate) fn parse_scorecard_threshold(raw: &str) -> Result<u8> {
    let value = raw
        .parse::<u8>()
        .with_context(|| format!("invalid score threshold `{raw}`"))?;
    if value > 100 {
        bail!("score threshold must be between 0 and 100");
    }
    Ok(value)
}

pub(crate) fn build_scorecard_report(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
) -> ScorecardReport {
    let command_names = CommandRouter::command_names();
    let docs_present = workspace.join("docs/ai/REQUIREMENTS.md").exists()
        && workspace.join("docs/ai/TECHNICAL_PLAN.md").exists()
        && workspace.join("docs/FEATURES.md").exists();
    let test_count = discover_tests_in(workspace)
        .map(|tests| tests.len())
        .unwrap_or_default();
    let project_config_present = project_config_path(workspace).exists();
    let git_identity = build_git_identity_report(workspace, &config.project.git_identity);
    let provider_model = active_default_model(config);
    let exported_scorecard_present = workspace.join(".deepcli/exports/scorecard.json").is_file();
    let benchmark_artifacts = load_benchmark_artifacts(workspace).unwrap_or_default();
    let benchmark_status =
        build_benchmark_status_report(workspace, &benchmark_artifacts, Utc::now());
    let benchmark_trends = build_benchmark_case_trends(&benchmark_artifacts, 2);
    let benchmark_trends_status =
        benchmark_trends_status(benchmark_artifacts.len(), &benchmark_trends);

    let mut categories = vec![
        scorecard_command_category(
            "command_discovery",
            "Command Discovery",
            "Users can discover the right entrypoint without memorizing the entire command surface.",
            &command_names,
            &[
                "/help",
                "/quickstart",
                "/recipes",
                "/scorecard",
                "/completion",
                "/version",
            ],
            &[
                "deepcli quickstart --json",
                "deepcli recipes",
                "deepcli completion json",
            ],
        ),
        scorecard_command_category(
            "agent_workflow",
            "Agent Workflow",
            "The product exposes a complete code, inspect, review, and iterate loop.",
            &command_names,
            &[
                "/status", "/usage", "/trace", "/plan", "/diff", "/review", "/test", "/prompt",
                "/skill", "/agent", "/git", "/web",
            ],
            &[
                "deepcli status --json",
                "deepcli review",
                "deepcli test discover --json",
            ],
        ),
        scorecard_command_category(
            "session_continuity",
            "Session Continuity",
            "Long tasks can be named, inspected, resumed, and cleaned up.",
            &command_names,
            &[
                "/resume", "/session", "/cleanup", "/rename", "/stop", "/quit",
            ],
            &[
                "deepcli resume",
                "deepcli sessions --all --limit 20",
                "deepcli session next --json",
            ],
        ),
        scorecard_command_category(
            "verification_delivery",
            "Verification And Delivery",
            "Changes can be tested, reviewed, gated, and handed off with structured evidence.",
            &command_names,
            &[
                "/test",
                "/accept",
                "/gate",
                "/verify",
                "/handoff",
                "/preflight",
                "/privacy",
            ],
            &[
                "deepcli preflight --json",
                "deepcli gate --json",
                "deepcli handoff --pr",
            ],
        ),
        scorecard_command_category(
            "safety_privacy",
            "Safety And Privacy",
            "Local-first safety, credentials, permissions, privacy, and health checks are visible.",
            &command_names,
            &[
                "/permissions",
                "/credentials",
                "/privacy",
                "/doctor",
                "/selftest",
                "/logs",
            ],
            &[
                "deepcli privacy --json --fail-on-findings",
                "deepcli permissions show --json",
                "deepcli credentials status --json",
            ],
        ),
        scorecard_command_category(
            "provider_model_ops",
            "Provider And Model Ops",
            "Provider/model switching, inspection, and timeout tuning are local and scriptable.",
            &command_names,
            &["/model", "/timeout"],
            &[
                "deepcli model list --json",
                "deepcli model set deepseek deepseek-v4-pro",
            ],
        ),
        scorecard_command_category(
            "support_operability",
            "Support Operability",
            "Users can collect redacted diagnostics and support artifacts without provider calls.",
            &command_names,
            &[
                "/diagnose",
                "/support",
                "/logs",
                "/trace",
                "/version",
            ],
            &["deepcli diagnose --json", "deepcli support", "deepcli version --json"],
        ),
        scorecard_command_category(
            "benchmark_evidence",
            "Benchmark Evidence",
            "The product can assess itself and point to missing SOTA validation evidence.",
            &command_names,
            &[
                "/scorecard",
                "/round",
                "/benchmark",
                "/preflight",
                "/gate",
                "/handoff",
            ],
            &[
                SCORECARD_ROUND_REPORT_ACTION,
                "deepcli preflight --json",
                "deepcli benchmark presets --json",
                "deepcli round --json --run-benchmark --fail-on-command",
                "deepcli benchmark run-suite --json --fail-on-command",
                "deepcli benchmark gate --json",
                "deepcli benchmark trends --json",
                "deepcli benchmark run --preset cargo-test --json --fail-on-command",
            ],
        ),
    ];

    if docs_present {
        scorecard_add_evidence(
            &mut categories[0],
            2,
            "docs/ai requirements, technical plan, and feature guide are present",
        );
    } else {
        scorecard_add_gap(
            &mut categories[0],
            "missing product requirement or feature documentation",
            "restore docs/ai/REQUIREMENTS.md, docs/ai/TECHNICAL_PLAN.md, and docs/FEATURES.md",
        );
    }

    if registry.declarations().len() >= 20 {
        scorecard_add_evidence(
            &mut categories[1],
            2,
            &format!(
                "tool registry exposes {} tools",
                registry.declarations().len()
            ),
        );
    } else {
        scorecard_add_gap(
            &mut categories[1],
            "tool registry exposes too few tools for broad coding tasks",
            "inspect `/selftest --json` and expand the tool registry before benchmarking",
        );
    }

    if workspace.join("src/ui.rs").exists() {
        scorecard_add_evidence(
            &mut categories[2],
            2,
            "native terminal implementation is present",
        );
    } else {
        scorecard_add_gap(
            &mut categories[2],
            "native terminal source is missing, so interactive sessions cannot start",
            "restore src/ui/native_terminal.rs and verify `deepcli` interactive startup",
        );
    }

    if test_count > 0 {
        scorecard_add_evidence(
            &mut categories[3],
            2,
            &format!("{test_count} project test command(s) discovered"),
        );
    } else {
        scorecard_add_gap(
            &mut categories[3],
            "no project tests were discovered for acceptance evidence",
            "add tests or run `/test discover --json` after configuring test commands",
        );
    }

    if config.sandbox.enabled_by_default
        && config.sandbox.allow_read_within_workspace
        && !config.sandbox.allow_dangerous_commands
        && !config.sandbox.allow_system_write
    {
        scorecard_add_evidence(
            &mut categories[4],
            2,
            "sandbox defaults allow workspace read while blocking system writes and dangerous commands",
        );
    } else {
        scorecard_add_gap(
            &mut categories[4],
            "sandbox defaults are weaker than the local-first safety target",
            "inspect `/permissions show --json` and restore safe sandbox defaults",
        );
    }
    if project_config_present && git_identity.issues.is_empty() {
        scorecard_add_evidence(
            &mut categories[4],
            2,
            "project config and expected Git identity are healthy",
        );
    } else {
        scorecard_add_gap(
            &mut categories[4],
            "project config or expected Git identity is missing or mismatched",
            "run `/doctor --quick --json` and apply the suggested git config fix",
        );
    }

    if config.providers.len() >= 2 {
        scorecard_add_evidence(
            &mut categories[5],
            2,
            &format!("{} providers configured", config.providers.len()),
        );
    } else {
        scorecard_add_gap(
            &mut categories[5],
            "only one provider is configured",
            "add a second provider configuration before provider-comparison benchmarks",
        );
    }
    if provider_model != "<unset>" {
        scorecard_add_evidence(
            &mut categories[5],
            1,
            &format!("default model is configured as {provider_model}"),
        );
    } else {
        scorecard_add_gap(
            &mut categories[5],
            "default provider model is not configured",
            "run `/model list --json` and `/model set <provider> <model>`",
        );
    }

    if docs_present && project_config_present {
        scorecard_add_evidence(
            &mut categories[6],
            2,
            "support docs and project config are available for issue triage",
        );
    } else {
        scorecard_add_gap(
            &mut categories[6],
            "support artifacts lack either docs or project config evidence",
            "run `/support` after restoring docs and project config",
        );
    }

    if docs_present {
        scorecard_add_evidence(
            &mut categories[7],
            2,
            "benchmark rubric is anchored to docs/ai product requirements",
        );
    }
    match benchmark_status.status {
        "ready" => {
            scorecard_add_evidence(
                &mut categories[7],
                3,
                &format!(
                    "recent meaningful benchmark evidence is present: {}",
                    benchmark_status
                        .latest_meaningful
                        .as_ref()
                        .map(|artifact| artifact.artifact_path.as_str())
                        .unwrap_or("<unknown>")
                ),
            );
        }
        "failing" | "stale" => {
            categories[7].score = categories[7].score.saturating_sub(2);
            scorecard_add_evidence(
                &mut categories[7],
                0,
                &format!(
                    "benchmark status is {} with {} artifact(s)",
                    benchmark_status.status, benchmark_status.artifact_count
                ),
            );
            for gap in &benchmark_status.gaps {
                scorecard_add_gap(
                    &mut categories[7],
                    gap,
                    SCORECARD_BENCHMARK_REMEDIATION_ACTION,
                );
            }
        }
        "weak" => {
            categories[7].score = categories[7].score.saturating_sub(3);
            scorecard_add_evidence(
                &mut categories[7],
                0,
                &format!(
                    "benchmark artifacts exist but evidence is weak: {} artifact(s), {} smoke artifact(s)",
                    benchmark_status.artifact_count, benchmark_status.smoke_count
                ),
            );
            for gap in &benchmark_status.gaps {
                scorecard_add_gap(
                    &mut categories[7],
                    gap,
                    SCORECARD_BENCHMARK_REMEDIATION_ACTION,
                );
            }
        }
        _ => {
            categories[7].score = categories[7].score.saturating_sub(3);
            if exported_scorecard_present {
                scorecard_add_evidence(
                    &mut categories[7],
                    0,
                    "an exported scorecard exists, but no benchmark run evidence was found",
                );
            }
            let gap = if exported_scorecard_present {
                "no local benchmark run artifact found under .deepcli/benchmarks; exported scorecard alone is weak benchmark evidence"
            } else {
                "no local benchmark artifact found under .deepcli/benchmarks"
            };
            scorecard_add_gap(
                &mut categories[7],
                gap,
                SCORECARD_BENCHMARK_REMEDIATION_ACTION,
            );
        }
    }

    for category in &mut categories {
        if category.score > category.max_score {
            category.score = category.max_score;
        }
        scorecard_prioritize_category_next_actions(category);
    }

    let score = categories
        .iter()
        .map(|category| category.score)
        .sum::<u16>();
    let max_score = categories
        .iter()
        .map(|category| category.max_score)
        .sum::<u16>();
    let percent = scorecard_percent(score, max_score);
    let gaps = categories
        .iter()
        .flat_map(|category| {
            category
                .gaps
                .iter()
                .map(|gap| format!("{}: {gap}", category.id))
        })
        .collect::<Vec<_>>();
    let tier = if gaps.is_empty() {
        scorecard_tier(percent)
    } else if percent >= 75 {
        "competitive_with_gaps"
    } else {
        scorecard_tier(percent)
    };
    let status = if gaps.is_empty() && percent >= 75 {
        "ok"
    } else {
        "needs_attention"
    };
    let has_gaps = !gaps.is_empty();
    let next_actions = scorecard_global_next_actions(
        workspace,
        &categories,
        has_gaps,
        &benchmark_status,
        benchmark_trends_status,
    );
    let opportunities =
        scorecard_product_opportunities(workspace, status, &gaps, &benchmark_status);
    let report = format_scorecard_text(
        workspace,
        ScorecardTextInput {
            status,
            tier,
            score,
            max_score,
            percent,
            categories: &categories,
            gaps: &gaps,
            next_actions: &next_actions,
            opportunities: &opportunities,
        },
    );

    ScorecardReport {
        report,
        status,
        tier,
        score,
        max_score,
        percent,
        categories,
        gaps,
        next_actions,
        opportunities,
    }
}

fn scorecard_command_category(
    id: &'static str,
    title: &'static str,
    summary: &'static str,
    command_names: &[&'static str],
    required_commands: &[&'static str],
    next_actions: &[&'static str],
) -> ScorecardCategory {
    let present = required_commands
        .iter()
        .filter(|command| command_names.contains(command))
        .count();
    let command_points = ((present as u16) * 8) / required_commands.len() as u16;
    let missing = required_commands
        .iter()
        .filter(|command| !command_names.contains(command))
        .map(|command| (*command).to_string())
        .collect::<Vec<_>>();
    let mut category = ScorecardCategory {
        id,
        title,
        summary,
        score: command_points,
        max_score: 10,
        evidence: vec![format!(
            "registered commands: {present}/{}",
            required_commands.len()
        )],
        gaps: Vec::new(),
        next_actions: Vec::new(),
        priority_next_actions: Vec::new(),
    };
    if !missing.is_empty() {
        category
            .gaps
            .push(format!("missing command(s): {}", missing.join(", ")));
    }
    category
        .next_actions
        .extend(next_actions.iter().map(|action| (*action).to_string()));
    category
}

fn scorecard_add_evidence(category: &mut ScorecardCategory, points: u16, evidence: &str) {
    category.score += points;
    category.evidence.push(evidence.to_string());
}

fn scorecard_add_gap(category: &mut ScorecardCategory, gap: &str, next_action: &str) {
    category.gaps.push(gap.to_string());
    category.priority_next_actions.push(next_action.to_string());
    category.next_actions.push(next_action.to_string());
}

fn scorecard_global_next_actions(
    workspace: &Path,
    categories: &[ScorecardCategory],
    has_gaps: bool,
    benchmark_status: &BenchmarkStatusReport,
    benchmark_trends_status: &str,
) -> Vec<String> {
    if !has_gaps {
        if benchmark_status.status == "ready"
            && round_benchmark_trends_needs_attention(benchmark_trends_status)
        {
            let mut actions = benchmark_freshness_next_actions(benchmark_status);
            actions.extend([
                round_benchmark_trends_next_action(benchmark_trends_status),
                "deepcli recipes sota --json".to_string(),
                SCORECARD_OPPORTUNITIES_ACTION.to_string(),
                "deepcli benchmark trends --json".to_string(),
                "deepcli benchmark status --json".to_string(),
                "deepcli preflight --json".to_string(),
                "deepcli gate --json".to_string(),
            ]);
            actions.extend(opportunity_baseline_next_actions(
                sota_baseline_next_actions(workspace),
            ));
            return dedup_preserve_order(actions);
        }
        let mut actions = benchmark_freshness_next_actions(benchmark_status);
        actions.extend([
            SCORECARD_ROUND_REPORT_ACTION.to_string(),
            "deepcli preflight --json".to_string(),
            "deepcli gate --json".to_string(),
            "deepcli recipes sota --json".to_string(),
            SCORECARD_OPPORTUNITIES_ACTION.to_string(),
            "deepcli benchmark trends --json".to_string(),
            "deepcli benchmark status --json".to_string(),
        ]);
        actions.extend(opportunity_baseline_next_actions(
            sota_baseline_next_actions(workspace),
        ));
        return dedup_preserve_order(actions);
    }

    let mut next_actions = benchmark_freshness_next_actions(benchmark_status);
    next_actions.extend(
        categories
            .iter()
            .flat_map(|category| category.priority_next_actions.clone())
            .collect::<Vec<_>>(),
    );
    next_actions.extend(
        categories
            .iter()
            .filter(|category| !category.gaps.is_empty())
            .flat_map(|category| category.next_actions.clone()),
    );
    next_actions.push(SCORECARD_BENCHMARK_REMEDIATION_ACTION.to_string());
    next_actions.push("deepcli recipes sota --json".to_string());
    next_actions.push("deepcli benchmark run-suite --json --fail-on-command".to_string());
    next_actions.push("deepcli benchmark status --json".to_string());
    next_actions.push("deepcli benchmark trends --json".to_string());
    next_actions.push("deepcli benchmark gate --json".to_string());
    next_actions.push("deepcli preflight --json".to_string());
    dedup_preserve_order(next_actions)
}

fn scorecard_prioritize_category_next_actions(category: &mut ScorecardCategory) {
    let mut actions = category.priority_next_actions.clone();
    actions.extend(
        category
            .next_actions
            .clone()
            .into_iter()
            .filter(|action| category.gaps.is_empty() || action != SCORECARD_ROUND_REPORT_ACTION),
    );
    category.next_actions = dedup_preserve_order(actions);
}

fn scorecard_category_checklist(category: &ScorecardCategory) -> Vec<Value> {
    scorecard_action_checklist(&category.next_actions)
}

fn scorecard_percent(score: u16, max_score: u16) -> u8 {
    if max_score == 0 {
        return 0;
    }
    (((score as u32) * 100 + (max_score as u32 / 2)) / max_score as u32) as u8
}

fn scorecard_tier(percent: u8) -> &'static str {
    match percent {
        90..=100 => "sota_candidate",
        75..=89 => "competitive",
        60..=74 => "foundation",
        _ => "needs_attention",
    }
}

fn scorecard_category_status(category: &ScorecardCategory) -> &'static str {
    match scorecard_percent(category.score, category.max_score) {
        90..=100 => "strong",
        75..=89 => "ready",
        60..=74 => "partial",
        _ => "weak",
    }
}

fn scorecard_score_scale_json() -> Value {
    json!({
        "score": "raw_points",
        "maxScore": "raw_points_max",
        "percent": "percent_0_100",
        "normalizedScore": "percent_0_100",
        "display": "normalizedScore",
    })
}

struct ScorecardTextInput<'a> {
    status: &'a str,
    tier: &'a str,
    score: u16,
    max_score: u16,
    percent: u8,
    categories: &'a [ScorecardCategory],
    gaps: &'a [String],
    next_actions: &'a [String],
    opportunities: &'a [ScorecardOpportunity],
}

fn format_scorecard_text(workspace: &Path, input: ScorecardTextInput<'_>) -> String {
    let mut lines = vec![
        "deepcli scorecard".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("status: {}", input.status),
        format!("tier: {}", input.tier),
        format!("raw score: {}/{} points", input.score, input.max_score),
        format!("normalized score: {}/100", input.percent),
        "categories:".to_string(),
    ];
    for category in input.categories {
        lines.push(format!(
            "  - {}: {}/{} ({}%, {})",
            category.id,
            category.score,
            category.max_score,
            scorecard_percent(category.score, category.max_score),
            scorecard_category_status(category)
        ));
        lines.push(format!("    title: {}", category.title));
        lines.push(format!("    summary: {}", category.summary));
        lines.push("    evidence:".to_string());
        lines.extend(
            category
                .evidence
                .iter()
                .map(|evidence| format!("      - {evidence}")),
        );
        if !category.gaps.is_empty() {
            lines.push("    gaps:".to_string());
            lines.extend(category.gaps.iter().map(|gap| format!("      - {gap}")));
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

fn format_scorecard_json(workspace: &Path, report: &ScorecardReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SCORECARD_V1,
        "status": report.status,
        "tier": report.tier,
        "score": report.score,
        "maxScore": report.max_score,
        "percent": report.percent,
        "normalizedScore": report.percent,
        "scoreScale": scorecard_score_scale_json(),
        "workspace": workspace.display().to_string(),
        "version": {
            "package": "deepcli",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "categories": report.categories.iter().map(|category| json!({
            "id": category.id,
            "title": category.title,
            "summary": category.summary,
            "status": scorecard_category_status(category),
            "score": category.score,
            "maxScore": category.max_score,
            "percent": scorecard_percent(category.score, category.max_score),
            "normalizedScore": scorecard_percent(category.score, category.max_score),
            "evidence": category.evidence,
            "gaps": category.gaps,
            "nextActions": category.next_actions,
            "checklist": scorecard_category_checklist(category),
        })).collect::<Vec<_>>(),
        "gaps": report.gaps,
        "nextActions": report.next_actions,
        "checklist": scorecard_action_checklist(&report.next_actions),
        "recommendedOpportunity": scorecard_recommended_opportunity_json(&report.opportunities),
        "opportunityPriorityCounts": scorecard_opportunity_priority_counts_json(&report.opportunities),
        "opportunityEffortCounts": scorecard_opportunity_effort_counts_json(&report.opportunities),
        "opportunities": scorecard_opportunities_json(&report.opportunities),
        "report": report.report,
    }))?)
}

pub(crate) fn scorecard_summary_json(report: &ScorecardReport) -> Value {
    json!({
        "schema": schema_ids::SCORECARD_SUMMARY_V1,
        "status": report.status,
        "tier": report.tier,
        "score": report.score,
        "maxScore": report.max_score,
        "percent": report.percent,
        "normalizedScore": report.percent,
        "scoreScale": scorecard_score_scale_json(),
        "gaps": report.gaps,
        "recommendedOpportunity": scorecard_recommended_opportunity_json(&report.opportunities),
        "opportunityPriorityCounts": scorecard_opportunity_priority_counts_json(&report.opportunities),
        "opportunityEffortCounts": scorecard_opportunity_effort_counts_json(&report.opportunities),
        "opportunities": scorecard_opportunities_json(&report.opportunities),
        "categories": report.categories.iter().map(|category| json!({
            "id": category.id,
            "status": scorecard_category_status(category),
            "score": category.score,
            "maxScore": category.max_score,
            "percent": scorecard_percent(category.score, category.max_score),
            "normalizedScore": scorecard_percent(category.score, category.max_score),
            "gaps": category.gaps,
            "nextActions": category.next_actions,
            "checklist": scorecard_category_checklist(category),
        })).collect::<Vec<_>>(),
    })
}
