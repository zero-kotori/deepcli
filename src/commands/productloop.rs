use super::*;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ScorecardOptions {
    json_output: bool,
    output_path: Option<String>,
    fail_below: Option<u8>,
}

#[derive(Debug, Clone)]
struct ScorecardCategory {
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
    report: String,
    status: &'static str,
    tier: &'static str,
    score: u16,
    max_score: u16,
    percent: u8,
    categories: Vec<ScorecardCategory>,
    gaps: Vec<String>,
    next_actions: Vec<String>,
    opportunities: Vec<ScorecardOpportunity>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScorecardOpportunity {
    pub(crate) id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) summary: String,
    pub(crate) impact: &'static str,
    pub(crate) priority: &'static str,
    pub(crate) effort: &'static str,
    pub(crate) status: &'static str,
    pub(crate) next_actions: Vec<String>,
}

pub(crate) const SCORECARD_BENCHMARK_REMEDIATION_ACTION: &str =
    "deepcli round --json --run-benchmark --fail-on-command";
const SCORECARD_ROUND_REPORT_ACTION: &str = "deepcli round --json";
const SCORECARD_OPPORTUNITIES_ACTION: &str = "deepcli opportunities --json";
const BENCHMARK_RUN_SUITE_REMEDIATION_ACTION: &str =
    "deepcli benchmark run-suite --json --fail-on-command";
const BENCHMARK_CARGO_TEST_REMEDIATION_ACTION: &str =
    "deepcli benchmark run --preset cargo-test --json --fail-on-command";
const DEFAULT_BENCHMARK_BASELINE_PATH: &str = ".deepcli/baselines/competitor.json";
const DEFAULT_BENCHMARK_CURRENT_BASELINE_PATH: &str = ".deepcli/baselines/current-main.json";
pub(crate) const DEFAULT_BENCHMARK_CURRENT_BASELINE_TEMPLATE_ACTION: &str =
    "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json";
pub(crate) const DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION: &str =
    "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json";
pub(crate) const DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION: &str =
    "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json";

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

fn parse_scorecard_threshold(raw: &str) -> Result<u8> {
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
                "/about",
            ],
            &["deepcli quickstart --json", "deepcli recipes", "deepcli completion json"],
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
            &["deepcli status --json", "deepcli review", "deepcli test discover --json"],
        ),
        scorecard_command_category(
            "session_continuity",
            "Session Continuity",
            "Long tasks can be named, inspected, resumed, and cleaned up.",
            &command_names,
            &[
                "/resume", "/session", "/history", "/cleanup", "/next", "/rename", "/stop",
                "/quit",
            ],
            &["deepcli resume", "deepcli sessions --all --limit 20", "deepcli next --json"],
        ),
        scorecard_command_category(
            "verification_delivery",
            "Verification And Delivery",
            "Changes can be tested, reviewed, gated, and handed off with structured evidence.",
            &command_names,
            &[
                "/test",
                "/env",
                "/accept",
                "/gate",
                "/verify",
                "/handoff",
                "/preflight",
                "/privacy",
            ],
            &["deepcli preflight --json", "deepcli gate --json", "deepcli handoff --pr"],
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
            &[
                "/model",
                "/provider",
                "/use",
                "/switch",
                "/models",
                "/providers",
                "/timeout",
            ],
            &["deepcli model list --json", "deepcli use deepseek deepseek-v4-pro"],
        ),
        scorecard_command_category(
            "support_operability",
            "Support Operability",
            "Users can collect redacted diagnostics and support artifacts without provider calls.",
            &command_names,
            &[
                "/diagnose",
                "/support",
                "/health",
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
        scorecard_add_evidence(&mut categories[2], 2, "TUI implementation is present");
    } else {
        scorecard_add_gap(
            &mut categories[2],
            "TUI source is missing, so session continuity cannot be inspected in-app",
            "restore or implement the TUI session picker and task monitor",
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

fn scorecard_product_opportunities(
    workspace: &Path,
    status: &str,
    gaps: &[String],
    benchmark_status: &BenchmarkStatusReport,
) -> Vec<ScorecardOpportunity> {
    if status != "ok" || !gaps.is_empty() {
        return Vec::new();
    }

    let mut opportunities = Vec::new();
    if let Some(refresh_action) = benchmark_freshness_refresh_action(benchmark_status) {
        let freshness = benchmark_freshness_status(benchmark_status);
        let age = format_benchmark_age(benchmark_freshness_age_seconds(benchmark_status));
        opportunities.push(ScorecardOpportunity {
            id: "benchmark_freshness",
            title: "Refresh Benchmark Evidence",
            summary: format!(
                "Benchmark evidence is ready but {freshness} at age {age}; refresh it before relying on SOTA claims."
            ),
            impact: "keeps ready benchmark evidence fresh enough for SOTA decisions",
            priority: "high",
            effort: "low",
            status: "available",
            next_actions: vec![
                refresh_action.to_string(),
                "deepcli benchmark status --json".to_string(),
            ],
        });
    }

    let baseline_actions = opportunity_baseline_next_actions(sota_baseline_next_actions(workspace));
    let baseline_ready = baseline_actions
        .iter()
        .any(|action| action.starts_with("deepcli benchmark compare --baseline "));
    let baseline = if baseline_ready {
        ScorecardOpportunity {
            id: "competitor_baseline",
            title: "Compare Competitor Baseline",
            summary:
                "A ready competitor baseline is available; compare current evidence against it."
                    .to_string(),
            impact: "keeps SOTA claims grounded in a local competitor benchmark",
            priority: "high",
            effort: "low",
            status: "available",
            next_actions: baseline_actions,
        }
    } else {
        ScorecardOpportunity {
            id: "competitor_baseline",
            title: "Prepare Competitor Baseline",
            summary:
                "Capture a current baseline and prepare a competitor baseline before the next SOTA comparison."
                    .to_string(),
            impact: "turns ready product evidence into comparable benchmark evidence",
            priority: "high",
            effort: "medium",
            status: "available",
            next_actions: baseline_actions,
        }
    };

    opportunities.push(baseline);
    opportunities.push(ScorecardOpportunity {
        id: "product_loop_experience",
        title: "Exercise Product Loop Experience",
        summary: "Review the ready round and SOTA recipe as the next product-design entrypoint."
            .to_string(),
        impact: "keeps the designer-engineer loop discoverable after all gates pass",
        priority: "medium",
        effort: "low",
        status: "available",
        next_actions: vec![
            SCORECARD_ROUND_REPORT_ACTION.to_string(),
            "deepcli recipes sota --json".to_string(),
        ],
    });
    opportunities
}

fn opportunity_baseline_next_actions(actions: Vec<String>) -> Vec<String> {
    let mut next_actions = vec!["deepcli benchmark baselines --json".to_string()];
    next_actions.extend(actions);
    dedup_preserve_order(next_actions)
}

fn scorecard_category_checklist(category: &ScorecardCategory) -> Vec<Value> {
    scorecard_action_checklist(&category.next_actions)
}

pub(crate) fn scorecard_opportunities_json(opportunities: &[ScorecardOpportunity]) -> Vec<Value> {
    opportunities
        .iter()
        .map(scorecard_opportunity_json)
        .collect()
}

fn scorecard_opportunity_json(opportunity: &ScorecardOpportunity) -> Value {
    json!({
        "id": opportunity.id,
        "title": opportunity.title,
        "summary": opportunity.summary,
        "impact": opportunity.impact,
        "priority": opportunity.priority,
        "effort": opportunity.effort,
        "status": opportunity.status,
        "nextActions": opportunity.next_actions,
        "checklist": scorecard_action_checklist(&opportunity.next_actions),
    })
}

pub(crate) fn scorecard_recommended_opportunity_json(
    opportunities: &[ScorecardOpportunity],
) -> Value {
    opportunities
        .first()
        .map(scorecard_opportunity_json)
        .unwrap_or(Value::Null)
}

pub(crate) fn scorecard_opportunity_priority_counts_json(
    opportunities: &[ScorecardOpportunity],
) -> Value {
    let (high, medium, low, other) = scorecard_opportunity_priority_counts(opportunities);
    json!({
        "high": high,
        "medium": medium,
        "low": low,
        "other": other,
    })
}

pub(crate) fn scorecard_opportunity_effort_counts_json(
    opportunities: &[ScorecardOpportunity],
) -> Value {
    let (high, medium, low, other) = scorecard_opportunity_effort_counts(opportunities);
    json!({
        "high": high,
        "medium": medium,
        "low": low,
        "other": other,
    })
}

pub(crate) fn scorecard_opportunity_summary_text(
    opportunities: &[ScorecardOpportunity],
) -> Vec<String> {
    let Some(recommended) = opportunities.first() else {
        return Vec::new();
    };
    let (high, medium, low, other) = scorecard_opportunity_priority_counts(opportunities);
    let (effort_high, effort_medium, effort_low, effort_other) =
        scorecard_opportunity_effort_counts(opportunities);
    vec![
        format!(
            "recommended opportunity: {} ({}, {})",
            recommended.id, recommended.priority, recommended.effort
        ),
        format!("priority counts: high={high} medium={medium} low={low} other={other}"),
        format!(
            "effort counts: high={effort_high} medium={effort_medium} low={effort_low} other={effort_other}"
        ),
    ]
}

fn scorecard_opportunity_priority_counts(
    opportunities: &[ScorecardOpportunity],
) -> (usize, usize, usize, usize) {
    let mut high = 0usize;
    let mut medium = 0usize;
    let mut low = 0usize;
    let mut other = 0usize;
    for opportunity in opportunities {
        match opportunity.priority {
            "high" => high += 1,
            "medium" => medium += 1,
            "low" => low += 1,
            _ => other += 1,
        }
    }
    (high, medium, low, other)
}

fn scorecard_opportunity_effort_counts(
    opportunities: &[ScorecardOpportunity],
) -> (usize, usize, usize, usize) {
    let mut high = 0usize;
    let mut medium = 0usize;
    let mut low = 0usize;
    let mut other = 0usize;
    for opportunity in opportunities {
        match opportunity.effort {
            "high" => high += 1,
            "medium" => medium += 1,
            "low" => low += 1,
            _ => other += 1,
        }
    }
    (high, medium, low, other)
}

pub(crate) fn scorecard_action_checklist(actions: &[String]) -> Vec<Value> {
    actions
        .iter()
        .filter(|action| action.starts_with("deepcli ") && !action.contains('<'))
        .enumerate()
        .map(|(index, command)| {
            json!({
                "step": index + 1,
                "label": scorecard_checklist_label(command),
                "command": command,
            })
        })
        .collect()
}

pub(crate) fn local_action_checklist(actions: &[String]) -> Vec<Value> {
    actions
        .iter()
        .filter(|action| {
            (action.starts_with("deepcli ")
                || action.starts_with("cargo ")
                || action.starts_with("git ")
                || action.starts_with("cd ")
                || action.starts_with("mkdir ")
                || action.starts_with("chmod ")
                || action.starts_with("ln ")
                || action.starts_with("rm "))
                && !action.contains('<')
                && !action.contains('>')
        })
        .enumerate()
        .map(|(index, command)| {
            json!({
                "step": index + 1,
                "label": local_checklist_label(command),
                "command": command,
            })
        })
        .collect()
}

fn benchmark_action_checklist(actions: &[String]) -> Vec<Value> {
    actions
        .iter()
        .filter(|action| {
            action.starts_with("deepcli ") && !action.contains('<') && !action.contains('>')
        })
        .enumerate()
        .map(|(index, command)| {
            json!({
                "step": index + 1,
                "label": benchmark_checklist_label(command),
                "command": command,
            })
        })
        .collect()
}

fn benchmark_value_with_action_checklist(mut value: Value) -> Value {
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

fn benchmark_checklist_label(command: &str) -> &'static str {
    if command.starts_with("deepcli benchmark compare --baseline") {
        "Compare benchmark baseline"
    } else if command.starts_with("deepcli benchmark clean --force") {
        "Delete benchmark artifacts"
    } else {
        match command {
            "deepcli benchmark list --json" => "List benchmark artifacts",
            "deepcli benchmark show latest --json" => "Show latest benchmark artifact",
            "deepcli benchmark clean --dry-run --json" => "Preview benchmark cleanup",
            "deepcli scorecard --json" => "Inspect product gaps",
            _ => scorecard_checklist_label(command),
        }
    }
}

fn local_checklist_label(command: &str) -> &'static str {
    if command.starts_with("deepcli ") {
        scorecard_checklist_label(command)
    } else if command.starts_with("cd ") && command.contains(" && deepcli resume ") {
        "Resume forked context"
    } else if command == "cargo test mvp_slash_commands_are_registered" {
        "Verify command registry"
    } else if command.starts_with("cargo ") {
        "Run cargo command"
    } else if command.starts_with("git config user.") {
        "Configure Git identity"
    } else if command.starts_with("git ") {
        "Run git command"
    } else if command.starts_with("mkdir ") || command.starts_with("ln ") {
        "Install shell command"
    } else if command.starts_with("chmod ") {
        "Update shell permissions"
    } else if command.starts_with("rm ") {
        "Remove stale shell file"
    } else if command.starts_with("cd ") {
        "Enter workspace"
    } else {
        "Run command"
    }
}

fn scorecard_checklist_label(command: &str) -> &'static str {
    match command {
        "deepcli quickstart" => "Open quickstart guide",
        "deepcli quickstart --check" => "Check quickstart readiness",
        "deepcli quickstart --json" => "Open quickstart readiness",
        "deepcli init --quick" => "Initialize project config",
        "deepcli config show --json" => "Inspect project config",
        "deepcli config sources --json" => "Inspect config sources",
        "deepcli config validate" => "Validate project config",
        "deepcli config validate --json" => "Validate project config",
        command if command.starts_with("deepcli config get ") => "Inspect config value",
        "deepcli recipes" => "Open workflow recipes",
        "deepcli recipes release" => "Open release workflow",
        "deepcli completion json" => "Export command catalog",
        command if command.starts_with("deepcli completion install ") => "Install shell completion",
        command if command.starts_with("deepcli completion status ") => "Check shell completion",
        "deepcli status --json" => "Inspect current status",
        command if command.starts_with("deepcli usage ") => "Inspect session usage",
        command if command.starts_with("deepcli trace ") => "Inspect session trace",
        command if command.starts_with("deepcli next ") => "Inspect recovery actions",
        command if command.starts_with("deepcli logs") => "Inspect local logs",
        "deepcli review" => "Review current diff",
        "deepcli test discover --json" => "Discover test commands",
        command if command.starts_with("deepcli test run ") => "Run test command",
        "deepcli help test" => "Open test help",
        "deepcli prompt list --json" => "List prompts",
        command if command.starts_with("deepcli prompt get ") => "Open prompt",
        command if command.starts_with("deepcli prompt render ") => "Render prompt",
        "deepcli help prompt" => "Open prompt help",
        "deepcli skill list --json" => "List skills",
        command if command.starts_with("deepcli skill run ") => "Run skill",
        "deepcli help skill" => "Open skill help",
        "deepcli agent list --json" => "List sub-agents",
        command if command.starts_with("deepcli agent show ") => "Inspect sub-agent",
        "deepcli help agent" => "Open agent help",
        "deepcli git status --json" => "Inspect git status",
        "deepcli git diff --json" => "Inspect git diff",
        "deepcli git message --json" => "Prepare commit message",
        "deepcli git branch --json" => "Inspect git branches",
        "deepcli help git" => "Open git help",
        "deepcli resume" => "Resume saved work",
        "deepcli resume --dry-run --json" => "Resume preview",
        "deepcli resume candidates --json" => "Inspect resume candidates",
        command if command.starts_with("deepcli resume ") && command.contains("--dry-run") => {
            "Resume preview"
        }
        command if command.starts_with("deepcli resume ") => "Resume saved work",
        "deepcli sessions --all --limit 20" => "List saved sessions",
        command if command.starts_with("deepcli history ") => "List saved sessions",
        "deepcli next --json" => "Inspect recovery actions",
        "deepcli handoff --pr" => "Prepare PR handoff",
        "deepcli permissions show --json" => "Inspect permissions",
        command if command.starts_with("deepcli permissions set-mode ") => "Set permission mode",
        "deepcli help permissions" => "Open permissions help",
        "deepcli credentials status --json" => "Inspect credentials",
        command if command.starts_with("deepcli credentials status ") => "Inspect credentials",
        command if command.starts_with("deepcli credentials set ") => {
            "Configure provider credentials"
        }
        command if command.starts_with("deepcli credentials import-env ") => {
            "Import credentials from environment"
        }
        command if command.starts_with("deepcli credentials template ") => {
            "Create credentials template"
        }
        "deepcli help credentials" => "Open credentials help",
        "deepcli model list" => "List configured models",
        "deepcli model list --json" => "List configured models",
        "deepcli model show --json" => "Inspect active model",
        "deepcli help model" => "Open model help",
        command if command.starts_with("deepcli model set ") => "Switch configured model",
        "deepcli timeout --json" => "Inspect provider timeout",
        "deepcli timeout reset" => "Reset provider timeout",
        "deepcli help timeout" => "Open timeout help",
        "deepcli stop" => "Stop running task",
        "deepcli fork --dry-run --json" => "Preview session fork",
        command if command.starts_with("deepcli fork --current") => "Fork active context",
        command if command.starts_with("deepcli fork ") => "Create session fork",
        "deepcli use deepseek deepseek-v4-pro" => "Switch to DeepSeek v4-pro",
        "deepcli doctor --quick" => "Run quick diagnostics",
        "deepcli doctor --quick --json" => "Run quick diagnostics",
        "deepcli doctor shell --json" => "Check shell install",
        "deepcli env check docker --json" => "Check Docker environment",
        command if command.starts_with("deepcli env check ") => "Check local environment",
        command if command.starts_with("deepcli env plan ") => "Inspect environment plan",
        command if command.starts_with("deepcli env test ") => "Run environment test",
        command if command.starts_with("deepcli setup ") => "Set up local environment",
        "deepcli diagnose --json" => "Collect diagnostics",
        "deepcli diagnose --full-env --json" => "Run full diagnostics",
        "deepcli diagnose --probe-provider --json" => "Probe provider diagnostics",
        command if command.starts_with("deepcli diagnose --full-env --bundle ") => {
            "Create full support bundle"
        }
        "deepcli session diagnose --json" => "Inspect session diagnostics",
        command if command.starts_with("deepcli session diagnose ") => {
            "Inspect session diagnostics"
        }
        command if command.starts_with("deepcli session next ") => "Inspect recovery actions",
        command if command.starts_with("deepcli session list") => "List saved sessions",
        command
            if command.starts_with("deepcli session prune-empty ")
                && command.contains("--force") =>
        {
            "Delete empty sessions"
        }
        command if command.starts_with("deepcli session prune-empty ") => {
            "Preview empty session cleanup"
        }
        command if command.starts_with("deepcli session tools --failed") => "Inspect failed tools",
        command if command.starts_with("deepcli session tests ") => "Inspect session tests",
        command if command.starts_with("deepcli session history ") => "Inspect session history",
        command if command.starts_with("deepcli session summary ") => "Inspect session summary",
        "deepcli help session" => "Open session help",
        "deepcli help resume" => "Open resume help",
        command if command.starts_with("deepcli approval approve ") => "Approve request",
        command if command.starts_with("deepcli approval deny ") => "Deny request",
        command if command.starts_with("deepcli approval list ") => "Review approvals",
        "deepcli help approval" => "Open approval help",
        command if command.starts_with("deepcli btw list ") => "Review by-the-way questions",
        "deepcli help btw" => "Open by-the-way help",
        command if command.starts_with("deepcli support ") || command == "deepcli support" => {
            "Create support bundle"
        }
        "deepcli version --json" => "Inspect version",
        "deepcli scorecard --json" => "Inspect product scorecard",
        "deepcli recipes sota --json" => "Open SOTA product loop recipe",
        "deepcli opportunities" | "deepcli opportunities --json" => "Open product opportunities",
        command if command.starts_with("deepcli opportunities ") => "Open product opportunities",
        "deepcli benchmark presets --json" => "List benchmark presets",
        "deepcli benchmark status --json" => "Check benchmark evidence",
        "deepcli benchmark run-suite --json --fail-on-command" => "Run benchmark suite",
        "deepcli benchmark run --preset cargo-test --json --fail-on-command" => {
            "Run cargo-test benchmark"
        }
        "deepcli benchmark gate --json" => "Gate benchmark evidence",
        "deepcli benchmark summary --json" => "Review benchmark summary",
        "deepcli benchmark trends --json" => "Check benchmark trends",
        "deepcli benchmark baselines --json" => "List benchmark baselines",
        DEFAULT_BENCHMARK_CURRENT_BASELINE_TEMPLATE_ACTION => "Capture current benchmark baseline",
        DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION => "Create competitor baseline template",
        DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION => "Compare against competitor baseline",
        "deepcli round --json --run-benchmark --fail-on-command" => "Refresh benchmark evidence",
        SCORECARD_ROUND_REPORT_ACTION => "Review current product round",
        "deepcli accept --json" => "Run acceptance checks",
        command if command.starts_with("deepcli accept ") => "Run acceptance checks",
        command if command.starts_with("deepcli gate ") => "Run delivery gate",
        _ => generic_recipe_command_label(command),
    }
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
        "schema": "deepcli.scorecard.v1",
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

#[derive(Debug, Clone)]
pub(crate) struct RoundGoalStatus {
    session: SessionMetadata,
    source: GoalSessionSource,
    ready: bool,
    blockers: Vec<String>,
    plan: GoalPlanReadiness,
    acceptance: Vec<GoalAcceptanceEvidence>,
    report: String,
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

fn round_benchmark_trends_needs_attention(status: &str) -> bool {
    matches!(status, "insufficient_history" | "regression")
}

fn round_benchmark_trends_gate_summary(status: &str, case_count: usize) -> String {
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

fn round_benchmark_trends_gap(status: &str) -> Option<String> {
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

fn round_benchmark_trends_next_action(status: &str) -> String {
    match status {
        "insufficient_history" => {
            "deepcli round --json --run-benchmark --fail-on-command".to_string()
        }
        _ => "deepcli benchmark trends --json".to_string(),
    }
}

fn round_benchmark_gate_summary(benchmark: &BenchmarkStatusReport) -> String {
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

fn build_round_goal_status(workspace: &Path) -> Option<RoundGoalStatus> {
    let store = SessionStore::new(workspace);
    let selection = select_goal_session(&store, None).ok().flatten()?;
    let report = collect_goal_readiness(workspace, &selection.session, &selection.goal).ok()?;
    Some(RoundGoalStatus {
        session: selection.session.metadata.clone(),
        source: selection.source,
        ready: report.ready,
        blockers: report.blockers,
        plan: report.plan,
        acceptance: report.acceptance,
        report: report.report,
    })
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
        "schema": "deepcli.round.v1",
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
        .map(|command| {
            vec![json!({
                "step": 1,
                "label": scorecard_checklist_label(command),
                "command": command,
            })]
        })
        .unwrap_or_default()
}

fn round_goal_status_json(goal: &RoundGoalStatus) -> Value {
    json!({
        "schema": "deepcli.goal.status.summary.v1",
        "status": if goal.ready { "ready" } else { "blocked" },
        "ready": goal.ready,
        "sessionSource": goal.source.as_str(),
        "session": session_metadata_json(&goal.session),
        "blockerCount": goal.blockers.len(),
        "blockers": &goal.blockers,
        "plan": {
            "present": goal.plan.present,
            "total": goal.plan.total,
            "completed": goal.plan.completed,
            "pending": goal.plan.pending,
            "inProgress": goal.plan.in_progress,
            "failed": goal.plan.failed,
        },
        "acceptance": goal.acceptance.iter().map(|item| json!({
            "command": redact_sensitive_text(&item.command),
            "status": item.status,
            "exitCode": item.exit_code,
            "createdAt": item.created_at,
        })).collect::<Vec<_>>(),
        "report": &goal.report,
    })
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

fn round_benchmark_status_json(report: &BenchmarkStatusReport) -> Value {
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

fn format_round_benchmark_freshness_suffix(report: &BenchmarkStatusReport) -> String {
    if report.latest_meaningful_age_seconds.is_none() {
        return String::new();
    }
    format!(
        " freshness={} age={}",
        benchmark_freshness_status(report),
        format_benchmark_age(benchmark_freshness_age_seconds(report))
    )
}

const DEFAULT_BENCHMARK_SUITE: &str = "product";
const DEFAULT_BENCHMARK_CASE: &str = "scorecard";
const DEFAULT_BENCHMARK_RUN_CASE: &str = "command";
pub(crate) const BENCHMARK_ARTIFACT_SCHEMA: &str = "deepcli.benchmark.record.v1";
pub(crate) const BENCHMARK_SUITE_SCHEMA: &str = "deepcli.benchmark.suite.v1";
pub(crate) const BENCHMARK_STATUS_SCHEMA: &str = "deepcli.benchmark.status.v1";
const DEFAULT_BENCHMARK_TIMEOUT_SECONDS: u64 = 120;
const BENCHMARK_OUTPUT_SAMPLE_CHARS: usize = 8_000;
const BENCHMARK_EVIDENCE_REFRESH_AFTER_DAYS: i64 = 1;
pub(crate) const BENCHMARK_EVIDENCE_STALE_AFTER_DAYS: i64 = 7;
pub(crate) const MEANINGFUL_BENCHMARK_PRESETS: &[&str] =
    &["cargo-test", "preflight-quick", "selftest", "scorecard"];
const DEFAULT_BENCHMARK_RUN_SUITE_PRESETS: &[&str] =
    &["cargo-test", "preflight-quick", "selftest", "scorecard"];

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
struct BenchmarkRunOptions {
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
struct BenchmarkRunSuiteOptions {
    json_output: bool,
    output_path: Option<String>,
    presets: Vec<String>,
    include_scorecard: bool,
    fail_on_command: bool,
    fail_fast: bool,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkListOptions {
    json_output: bool,
    output_path: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkBaselinesOptions {
    json_output: bool,
    output_path: Option<String>,
    limit: Option<usize>,
}

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkCleanupOptions {
    json_output: bool,
    output_path: Option<String>,
    force: bool,
    keep: Option<usize>,
    older_than_days: Option<i64>,
    all: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkStatusOptions {
    json_output: bool,
    output_path: Option<String>,
    fail_on_not_ready: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BenchmarkPresetsOptions {
    json_output: bool,
    output_path: Option<String>,
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
struct BenchmarkArtifact {
    relative_path: String,
    value: Value,
    created_at: Option<DateTime<Utc>>,
    modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct BenchmarkRunArtifact {
    artifact: Value,
    relative_path: String,
    execution: BenchmarkCommandExecution,
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkStatusReport {
    status: &'static str,
    artifact_count: usize,
    executable_count: usize,
    passed_count: usize,
    failed_count: usize,
    timeout_count: usize,
    recorded_count: usize,
    other_count: usize,
    smoke_count: usize,
    meaningful_count: usize,
    meaningful_executable_count: usize,
    meaningful_passed_count: usize,
    meaningful_failed_count: usize,
    meaningful_timeout_count: usize,
    latest_artifact: Option<BenchmarkStatusArtifact>,
    latest_meaningful: Option<BenchmarkStatusArtifact>,
    latest_meaningful_age_seconds: Option<i64>,
    seen_meaningful_presets: Vec<String>,
    missing_meaningful_presets: Vec<String>,
    required_preset_statuses: Vec<BenchmarkRequiredPresetStatus>,
    gaps: Vec<String>,
    next_actions: Vec<String>,
}

#[derive(Debug, Clone)]
struct BenchmarkRequiredPresetStatus {
    preset: String,
    status: String,
    artifact: Option<BenchmarkStatusArtifact>,
    age_seconds: Option<i64>,
    gap: Option<String>,
}

#[derive(Debug, Clone)]
struct BenchmarkStatusArtifact {
    artifact_path: String,
    created_at: Option<DateTime<Utc>>,
    suite: String,
    case_name: String,
    preset: Option<String>,
    status: String,
    ran_by_deepcli: Option<bool>,
    duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct BenchmarkPreset {
    name: &'static str,
    aliases: &'static [&'static str],
    title: &'static str,
    summary: &'static str,
    suite: &'static str,
    case_name: &'static str,
    command: &'static str,
    timeout_seconds: u64,
}

const BENCHMARK_PRESETS: &[BenchmarkPreset] = &[
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

pub(crate) fn handle_benchmark(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    args: Vec<String>,
) -> Result<String> {
    if benchmark_args_are_scorecard_compatible(&args) {
        return handle_scorecard(workspace, config, registry, args);
    }
    let Some((subcommand, rest)) = args.split_first() else {
        return handle_scorecard(workspace, config, registry, Vec::new());
    };
    match subcommand.as_str() {
        "scorecard" | "rubric" => handle_scorecard(workspace, config, registry, rest.to_vec()),
        "run-suite" | "suite" | "run-all" => {
            handle_benchmark_run_suite(workspace, config, registry, rest)
        }
        "run" | "exec" => handle_benchmark_run(workspace, config, registry, rest),
        "record" | "save" => handle_benchmark_record(workspace, config, registry, rest),
        "presets" | "preset" | "catalog" => handle_benchmark_presets(workspace, rest),
        "status" | "health" | "doctor" => handle_benchmark_status(workspace, rest),
        "gate" | "check" => {
            let mut gate_args = rest.to_vec();
            if !benchmark_status_args_request_failure(&gate_args) {
                gate_args.push("--fail-on-not-ready".to_string());
            }
            handle_benchmark_status(workspace, &gate_args)
        }
        "summary" | "summarize" | "report" => handle_benchmark_summary(workspace, rest),
        "trend" | "trends" | "history" => handle_benchmark_trends(workspace, rest),
        "baseline-template" | "template" | "baseline-init" => {
            handle_benchmark_baseline_template(workspace, rest)
        }
        "compare" | "comparison" | "baseline" => handle_benchmark_compare(workspace, rest),
        "baselines" | "baseline-list" | "baseline-ls" => {
            handle_benchmark_baselines(workspace, rest)
        }
        "list" | "ls" => handle_benchmark_list(workspace, rest),
        "show" | "view" => handle_benchmark_show(workspace, rest),
        "clean" | "cleanup" | "prune" => handle_benchmark_cleanup(workspace, rest),
        "latest" => {
            let mut show_args = vec!["latest".to_string()];
            show_args.extend(rest.iter().cloned());
            handle_benchmark_show(workspace, &show_args)
        }
        value => bail!(
            "unknown /benchmark subcommand `{value}`; expected presets, run-suite, run, record, status, gate, summary, trends, baseline-template, compare, baselines, list, show, clean, or scorecard"
        ),
    }
}

fn benchmark_status_args_request_failure(args: &[String]) -> bool {
    args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--fail-on-not-ready" | "--fail-on-gaps" | "--strict"
        )
    })
}

fn benchmark_args_are_scorecard_compatible(args: &[String]) -> bool {
    args.is_empty()
        || args.first().is_some_and(|arg| {
            matches!(
                arg.as_str(),
                "--json" | "--output" | "-o" | "--fail-below" | "--min-score"
            ) || arg.starts_with("--output=")
                || arg.starts_with("--fail-below=")
                || arg.starts_with("--min-score=")
        })
}

fn handle_benchmark_run(
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

fn execute_benchmark_run_artifact(
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

fn handle_benchmark_run_suite(
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

fn push_benchmark_run_suite_presets(target: &mut Vec<String>, raw: &str) -> Result<()> {
    for name in raw.split(',') {
        push_benchmark_run_suite_preset(target, name)?;
    }
    Ok(())
}

fn push_benchmark_run_suite_preset(target: &mut Vec<String>, raw: &str) -> Result<()> {
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

fn benchmark_run_suite_preset_names(options: &BenchmarkRunSuiteOptions) -> Result<Vec<String>> {
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

fn benchmark_run_options_for_suite_preset(
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

fn benchmark_run_suite_status(runs: &[BenchmarkRunArtifact]) -> &'static str {
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

fn benchmark_run_suite_artifact_json(run: &BenchmarkRunArtifact) -> Value {
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

fn benchmark_preset_by_name(name: &str) -> Result<&'static BenchmarkPreset> {
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

fn handle_benchmark_record(
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

#[derive(Debug, Clone)]
struct BenchmarkCommandExecution {
    command: String,
    status: &'static str,
    exit_code: Option<i32>,
    timed_out: bool,
    duration_ms: u128,
    stdout_chars: usize,
    stderr_chars: usize,
    stdout_sample: String,
    stderr_sample: String,
    error: Option<String>,
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

pub(crate) fn scorecard_summary_json(report: &ScorecardReport) -> Value {
    json!({
        "schema": "deepcli.scorecard.summary.v1",
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

fn benchmark_slug(raw: &str, fallback: &str) -> String {
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

fn handle_benchmark_list(workspace: &Path, args: &[String]) -> Result<String> {
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

fn handle_benchmark_presets(workspace: &Path, args: &[String]) -> Result<String> {
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

fn handle_benchmark_status(workspace: &Path, args: &[String]) -> Result<String> {
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

fn build_benchmark_status_report(
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

fn benchmark_freshness_next_actions(report: &BenchmarkStatusReport) -> Vec<String> {
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

fn benchmark_status_artifact_json(artifact: &Option<BenchmarkStatusArtifact>) -> Value {
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

fn format_benchmark_status_json(
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

fn benchmark_status_summary_json(report: &BenchmarkStatusReport, checklist: &[Value]) -> Value {
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

fn benchmark_required_preset_status_json(status: &BenchmarkRequiredPresetStatus) -> Value {
    json!({
        "preset": status.preset,
        "status": status.status,
        "artifact": benchmark_status_artifact_json(&status.artifact),
        "ageSeconds": status.age_seconds,
        "gap": status.gap,
    })
}

fn benchmark_freshness_json(report: &BenchmarkStatusReport) -> Value {
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

fn benchmark_freshness_status(report: &BenchmarkStatusReport) -> &'static str {
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

fn benchmark_freshness_refresh_recommended(report: &BenchmarkStatusReport) -> bool {
    matches!(benchmark_freshness_status(report), "aging" | "stale")
}

fn benchmark_freshness_refresh_action(report: &BenchmarkStatusReport) -> Option<&'static str> {
    benchmark_freshness_refresh_recommended(report)
        .then_some(SCORECARD_BENCHMARK_REMEDIATION_ACTION)
}

fn benchmark_freshness_age_seconds(report: &BenchmarkStatusReport) -> Option<i64> {
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

fn format_benchmark_age(age_seconds: Option<i64>) -> String {
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

fn format_benchmark_status_text(workspace: &Path, report: &BenchmarkStatusReport) -> String {
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

fn benchmark_artifact_matches_preset(value: &Value, preset: &BenchmarkPreset) -> bool {
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

fn benchmark_artifact_preset(value: &Value) -> Option<&str> {
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

fn handle_benchmark_summary(workspace: &Path, args: &[String]) -> Result<String> {
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

fn handle_benchmark_trends(workspace: &Path, args: &[String]) -> Result<String> {
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

fn handle_benchmark_compare(workspace: &Path, args: &[String]) -> Result<String> {
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

fn handle_benchmark_baselines(workspace: &Path, args: &[String]) -> Result<String> {
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

fn set_benchmark_baseline_path(target: &mut Option<String>, raw: &str) -> Result<()> {
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

fn handle_benchmark_baseline_template(workspace: &Path, args: &[String]) -> Result<String> {
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

fn handle_benchmark_cleanup(workspace: &Path, args: &[String]) -> Result<String> {
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

fn handle_benchmark_show(workspace: &Path, args: &[String]) -> Result<String> {
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

fn load_benchmark_artifacts(workspace: &Path) -> Result<Vec<BenchmarkArtifact>> {
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
        "schema": "deepcli.benchmark.list.v1",
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
        "schema": "deepcli.benchmark.cleanup.v1",
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

fn format_benchmark_presets_json(workspace: &Path) -> Result<String> {
    let next_actions = benchmark_presets_next_actions(workspace);
    let checklist = benchmark_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.benchmark.presets.v1",
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
struct BenchmarkCaseTrend {
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

#[derive(Debug, Clone, Default)]
struct BenchmarkBaselineReport {
    present: bool,
    name: Option<String>,
    path: Option<String>,
    cases: Vec<BenchmarkBaselineCase>,
}

#[derive(Debug, Clone)]
struct BenchmarkBaselineCase {
    suite: String,
    case_name: String,
    status: Option<String>,
    duration_ms: Option<u64>,
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

fn build_benchmark_case_trends(
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
        "schema": "deepcli.benchmark.baseline.v1",
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

#[derive(Debug, Clone)]
struct BenchmarkBaselineTemplateCapture {
    status: String,
    duration_ms: Option<u64>,
    artifact_path: String,
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

fn load_benchmark_baseline(
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

fn artifact_string_field(value: &Value, key: &str, fallback: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(redact_sensitive_text)
        .unwrap_or_else(|| fallback.to_string())
}

fn benchmark_artifact_status(value: &Value) -> &str {
    value
        .get("execution")
        .and_then(|execution| execution.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

fn benchmark_artifact_duration_ms(value: &Value) -> Option<u64> {
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

fn format_benchmark_compare_json(
    workspace: &Path,
    artifacts: &[BenchmarkArtifact],
    baseline: &BenchmarkBaselineReport,
    comparisons: &[BenchmarkCaseComparison],
) -> Result<String> {
    let next_actions = benchmark_compare_next_actions(baseline, artifacts.is_empty());
    let checklist = benchmark_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.benchmark.compare.v1",
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

fn benchmark_baseline_report_json(baseline: &BenchmarkBaselineReport) -> Value {
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

fn benchmark_baseline_case_json(case: &BenchmarkBaselineCase) -> Value {
    json!({
        "suite": case.suite,
        "case": case.case_name,
        "status": case.status,
        "durationMs": case.duration_ms,
    })
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
        "schema": "deepcli.benchmark.baselines.v1",
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

fn benchmark_baseline_needs_values(baseline: &BenchmarkBaselineReport) -> bool {
    baseline
        .cases
        .iter()
        .any(|case| case.status.is_none() || case.duration_ms.is_none())
}

fn format_benchmark_summary_json(
    workspace: &Path,
    artifacts: &[BenchmarkArtifact],
    summaries: &[BenchmarkCaseSummary],
) -> Result<String> {
    let next_actions = benchmark_summary_next_actions(workspace);
    let checklist = benchmark_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.benchmark.summary.v1",
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
        "schema": "deepcli.benchmark.trends.v1",
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

fn benchmark_trends_status(artifact_count: usize, trends: &[BenchmarkCaseTrend]) -> &'static str {
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

fn format_benchmark_artifact_text(
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
