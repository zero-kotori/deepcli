use super::{
    build_round_report, dedup_preserve_order, required_arg, scorecard_action_checklist,
    scorecard_opportunities_json, scorecard_opportunity_effort_counts_json,
    scorecard_opportunity_priority_counts_json, scorecard_opportunity_summary_text,
    scorecard_recommended_opportunity_json, set_command_output_path, sota_baseline_next_actions,
    write_command_output, ScorecardOpportunity, DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION,
    DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION, DEFAULT_BENCHMARK_CURRENT_BASELINE_TEMPLATE_ACTION,
    DEFAULT_ROUND_SCORE_THRESHOLD,
};
use crate::config::AppConfig;
use crate::schema_ids;
use crate::tools::ToolRegistry;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RecipesOptions {
    topic: Option<&'static str>,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct Recipe {
    name: &'static str,
    title: &'static str,
    summary: &'static str,
    commands: &'static [&'static str],
    notes: &'static [&'static str],
}

#[derive(Debug, Clone, Default)]
struct RecipesState {
    next_actions: Vec<String>,
    opportunities: Vec<ScorecardOpportunity>,
}

pub(crate) fn handle_recipes(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_recipes_options(&args)?;
    let recipes = selected_recipes(options.topic)?;
    let state = recipes_state(workspace, config, registry, options.topic);
    let report = format_recipes_text(
        workspace,
        options.topic,
        &recipes,
        &state.next_actions,
        &state.opportunities,
    );
    let output = if options.json_output {
        format_recipes_json(
            workspace,
            options.topic,
            &recipes,
            &state.next_actions,
            &state.opportunities,
            &report,
        )?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_recipes_options(args: &[String]) -> Result<RecipesOptions> {
    let mut options = RecipesOptions::default();
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
            "--all" | "all" => {
                options.topic = None;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /recipes option `{value}`"),
            value => {
                if options.topic.is_some() {
                    bail!("usage: /recipes [topic|all] [--json] [--output path]");
                }
                options.topic = Some(normalize_recipe_topic(value)?);
                index += 1;
            }
        }
    }
    Ok(options)
}

fn normalize_recipe_topic(value: &str) -> Result<&'static str> {
    match value {
        "start" | "onboard" | "onboarding" | "quickstart" => Ok("start"),
        "code" | "coding" | "task" | "agent" => Ok("code"),
        "debug" | "diagnose" | "troubleshoot" | "troubleshooting" => Ok("debug"),
        "release" | "ship" | "preflight" | "publish" => Ok("release"),
        "support" | "issue" | "bundle" => Ok("support"),
        "environment" | "env" | "setup" | "install" => Ok("environment"),
        "shell" | "completion" | "completions" => Ok("shell"),
        "sota" | "product" | "product-loop" | "loop" | "iterate" | "iteration" | "round"
        | "benchmark" | "bench" => Ok("sota"),
        other => bail!(
            "unknown /recipes topic `{other}`; supported topics: {}",
            recipes_topic_names().join(", ")
        ),
    }
}

fn recipes_topic_names() -> Vec<&'static str> {
    recipes_catalog()
        .iter()
        .map(|recipe| recipe.name)
        .collect::<Vec<_>>()
}

fn selected_recipes(topic: Option<&'static str>) -> Result<Vec<Recipe>> {
    let recipes = recipes_catalog();
    if let Some(topic) = topic {
        let selected = recipes
            .into_iter()
            .filter(|recipe| recipe.name == topic)
            .collect::<Vec<_>>();
        if selected.is_empty() {
            bail!("unknown /recipes topic `{topic}`");
        }
        Ok(selected)
    } else {
        Ok(recipes)
    }
}

fn recipes_catalog() -> Vec<Recipe> {
    vec![
        Recipe {
            name: "start",
            title: "Start Or Onboard",
            summary: "Open deepcli, verify local setup, configure credentials, and resume prior work.",
            commands: &[
                "deepcli",
                "deepcli quickstart --json",
                "deepcli doctor --quick --json",
                "deepcli credentials status --json",
                "deepcli resume",
            ],
            notes: &[
                "Use this when entering a new project or checking whether deepcli is ready before asking the agent to code.",
                "If credentials are missing, run `deepcli credentials set <provider>` or `deepcli login <provider> --stdin --force`.",
            ],
        },
        Recipe {
            name: "code",
            title: "Code With An Agent",
            summary: "Start an interactive or one-shot coding task with a clear provider/model path.",
            commands: &[
                "deepcli deepseek",
                "deepcli kimi",
                "deepcli ask '阅读项目结构并说明如何运行测试'",
                "deepcli status --json",
                "deepcli next --json",
            ],
            notes: &[
                "Use native terminal chat for multi-step edits and `ask` for bounded one-shot analysis.",
                "Use `/status`, `/usage`, and `/trace` during long tasks instead of interrupting the agent.",
            ],
        },
        Recipe {
            name: "debug",
            title: "Debug Slow Or Failed Runs",
            summary: "Collect local evidence for provider latency, tool failures, logs, and failed tests.",
            commands: &[
                "deepcli usage --json",
                "deepcli trace --limit 30",
                "deepcli logs --limit 80",
                "deepcli session tools --failed --limit 5",
                "deepcli diagnose --json",
            ],
            notes: &[
                "Use `deepcli diagnose --probe-provider --provider <name>` only when an online provider probe is needed.",
                "Use `deepcli support` after diagnostics if you need a redacted bundle for an issue.",
            ],
        },
        Recipe {
            name: "release",
            title: "Release Or Push",
            summary: "Run local acceptance checks, privacy scanning, strict gate, and handoff output.",
            commands: &[
                "deepcli preflight --dry-run",
                "deepcli scorecard --json",
                "deepcli preflight --json",
                "deepcli privacy --json --fail-on-findings",
                "deepcli gate --json",
                "deepcli handoff --pr",
            ],
            notes: &[
                "Use `--quick` on preflight only for fast local iteration; use full mode before pushing.",
                "Preflight keeps going across checks so one run can show every blocker.",
            ],
        },
        Recipe {
            name: "support",
            title: "Support Bundle",
            summary: "Create redacted diagnostics for issues, bug reports, or teammate handoff.",
            commands: &[
                "deepcli support",
                "deepcli support .deepcli/support/latest",
                "deepcli diagnose --bundle .deepcli/support/latest",
                "deepcli version --json",
                "deepcli privacy --json",
            ],
            notes: &[
                "Support bundles include redacted logs, version data, diagnose output, quickstart, status, usage, trace, and sessions.",
                "Keep bundles inside the workspace so path safety checks and redaction stay consistent.",
            ],
        },
        Recipe {
            name: "environment",
            title: "Prepare Local Environment",
            summary: "Check, plan, install, and smoke-test Docker or compiler task environments.",
            commands: &[
                "deepcli doctor docker --json",
                "deepcli compiler plan --smoke --json",
                "deepcli install docker --smoke",
                "deepcli install compiler --smoke",
                "deepcli compiler test --json",
            ],
            notes: &[
                "Always preview with `compiler plan` before install if the task may touch Docker, Colima, or compiler tools.",
                "Environment setup runs through the permission engine; read-only check/plan commands do not need a provider call.",
            ],
        },
        Recipe {
            name: "shell",
            title: "Shell Integration",
            summary: "Install or audit shell completion and command discovery integrations.",
            commands: &[
                "deepcli doctor shell --json",
                "deepcli completion status zsh --json",
                "deepcli completion install zsh",
                "deepcli completion install zsh --force",
                "deepcli completion json --output .deepcli/exports/commands.json",
            ],
            notes: &[
                "`completion install` is dry-run by default and only writes under allowlisted HOME completion paths with `--force`.",
                "Use the JSON command catalog for external launchers, docs generators, or interactive command surfaces.",
            ],
        },
        Recipe {
            name: "sota",
            title: "SOTA Product Loop",
            summary: "Inspect product gaps, run local benchmark evidence, compare baselines, and gate the next iteration.",
            commands: &[
                "deepcli recipes sota --json",
                "deepcli scorecard --json",
                "deepcli round --json",
                "deepcli round --json --run-benchmark --fail-on-command",
                "deepcli benchmark status --json",
                "deepcli benchmark trends --json",
                DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION,
                DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION,
                "deepcli benchmark gate --json",
            ],
            notes: &[
                "Use this after each product iteration to decide the next highest-value gap before writing code.",
                "Benchmark artifacts stay local under `.deepcli/benchmarks/`; commit docs and code, not local evidence files.",
                "Use the baseline template when you want competitor, older-version, or hand-recorded comparison evidence.",
            ],
        },
    ]
}

fn recipes_state(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    topic: Option<&'static str>,
) -> RecipesState {
    let actions = match topic {
        Some("start") => vec!["deepcli", "deepcli recipes code"],
        Some("code") => vec!["deepcli status --json", "deepcli recipes release"],
        Some("debug") => vec!["deepcli diagnose --json", "deepcli support"],
        Some("release") => vec!["deepcli preflight --dry-run", "deepcli preflight --json"],
        Some("support") => vec![
            "deepcli support .deepcli/support/latest",
            "deepcli diagnose --bundle .deepcli/support/latest",
        ],
        Some("environment") => vec![
            "deepcli compiler plan --smoke --json",
            "deepcli compiler test --json",
        ],
        Some("shell") => vec![
            "deepcli doctor shell --json",
            "deepcli completion install zsh --force",
        ],
        Some("sota") => {
            let round = build_round_report(
                workspace,
                config,
                registry,
                DEFAULT_ROUND_SCORE_THRESHOLD,
                None,
            );
            let mut actions = round.next_actions;
            actions.retain(|action| action != "deepcli recipes sota --json");
            actions.extend(sota_baseline_next_actions(workspace));
            return RecipesState {
                next_actions: dedup_preserve_order(actions),
                opportunities: round.opportunities,
            };
        }
        _ => vec![
            "deepcli recipes sota",
            "deepcli recipes release",
            "deepcli recipes --json --output .deepcli/exports/recipes.json",
        ],
    };
    RecipesState {
        next_actions: dedup_preserve_order(actions.into_iter().map(str::to_string).collect()),
        opportunities: Vec::new(),
    }
}

fn format_recipes_text(
    workspace: &Path,
    topic: Option<&'static str>,
    recipes: &[Recipe],
    next_actions: &[String],
    opportunities: &[ScorecardOpportunity],
) -> String {
    let mut lines = vec![
        "deepcli recipes".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("topic: {}", topic.unwrap_or("all")),
    ];
    if topic.is_none() {
        lines.push(format!(
            "available topics: {}",
            recipes_topic_names().join(", ")
        ));
    }
    lines.push("recipes:".to_string());
    for recipe in recipes {
        lines.push(format!("  {} - {}", recipe.name, recipe.title));
        lines.push(format!("    summary: {}", recipe.summary));
        lines.push("    commands:".to_string());
        for (index, command) in recipe.commands.iter().enumerate() {
            lines.push(format!("      {}. {command}", index + 1));
        }
        if !recipe.notes.is_empty() {
            lines.push("    notes:".to_string());
            lines.extend(recipe.notes.iter().map(|note| format!("      - {note}")));
        }
    }
    lines.push("next actions:".to_string());
    lines.extend(next_actions.iter().map(|action| format!("  - {action}")));
    if !opportunities.is_empty() {
        lines.extend(scorecard_opportunity_summary_text(opportunities));
        lines.push("opportunities:".to_string());
        for opportunity in opportunities {
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
    lines.join("\n")
}

fn format_recipes_json(
    workspace: &Path,
    topic: Option<&'static str>,
    recipes: &[Recipe],
    next_actions: &[String],
    opportunities: &[ScorecardOpportunity],
    report: &str,
) -> Result<String> {
    let title = recipes_json_title(topic, recipes);
    let summary = recipes_json_summary(topic, recipes);
    let checklist = recipes_checklist(workspace, topic, recipes, next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::RECIPES_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "topic": topic.unwrap_or("all"),
        "title": title,
        "summary": summary,
        "checklist": checklist,
        "availableTopics": recipes_topic_names(),
        "recipes": recipes.iter().map(|recipe| json!({
            "name": recipe.name,
            "title": recipe.title,
            "summary": recipe.summary,
            "commands": recipe.commands,
            "notes": recipe.notes,
        })).collect::<Vec<_>>(),
        "nextActions": next_actions,
        "recommendedOpportunity": scorecard_recommended_opportunity_json(opportunities),
        "opportunityPriorityCounts": scorecard_opportunity_priority_counts_json(opportunities),
        "opportunityEffortCounts": scorecard_opportunity_effort_counts_json(opportunities),
        "opportunities": scorecard_opportunities_json(opportunities),
        "report": report,
    }))?)
}

fn recipes_json_title(topic: Option<&'static str>, recipes: &[Recipe]) -> String {
    if topic.is_some() && recipes.len() == 1 {
        recipes[0].title.to_string()
    } else {
        "deepcli Recipes".to_string()
    }
}

fn recipes_json_summary(topic: Option<&'static str>, recipes: &[Recipe]) -> String {
    if topic.is_some() && recipes.len() == 1 {
        recipes[0].summary.to_string()
    } else {
        "Task-oriented command recipes for common deepcli workflows.".to_string()
    }
}

fn recipes_checklist(
    workspace: &Path,
    topic: Option<&'static str>,
    recipes: &[Recipe],
    next_actions: &[String],
) -> Vec<Value> {
    if topic == Some("sota") && recipes.len() == 1 {
        return scorecard_action_checklist(next_actions);
    }
    if recipes.len() == 1 {
        let recipe = recipes[0];
        return recipe_checklist_commands(workspace, &recipe)
            .into_iter()
            .enumerate()
            .map(|(index, command)| {
                json!({
                    "step": index + 1,
                    "label": recipe_command_label(recipe.name, &command),
                    "command": command,
                })
            })
            .collect();
    }
    recipes
        .iter()
        .enumerate()
        .map(|(index, recipe)| {
            json!({
                "step": index + 1,
                "label": format!("Open {}", recipe.title),
                "command": format!("deepcli recipes {} --json", recipe.name),
            })
        })
        .collect()
}

fn recipe_checklist_commands(workspace: &Path, recipe: &Recipe) -> Vec<String> {
    if recipe.name != "sota" {
        return recipe
            .commands
            .iter()
            .map(|command| command.to_string())
            .collect();
    }
    let mut commands = Vec::new();
    for command in recipe.commands {
        match *command {
            DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION => {
                commands.extend(sota_baseline_next_actions(workspace));
            }
            DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION => {}
            _ => commands.push(command.to_string()),
        }
    }
    dedup_preserve_order(commands)
}

fn recipe_command_label(recipe_name: &str, command: &str) -> String {
    if recipe_name == "sota" {
        return sota_recipe_command_label(command).to_string();
    }
    generic_recipe_command_label(command).to_string()
}

fn sota_recipe_command_label(command: &str) -> &'static str {
    match command {
        "deepcli recipes sota --json" => "Open SOTA product loop recipe",
        "deepcli scorecard --json" => "Inspect product gaps",
        "deepcli round --json" => "Review current product round",
        "deepcli round --json --run-benchmark --fail-on-command" => "Refresh benchmark evidence",
        "deepcli benchmark status --json" => "Check benchmark evidence",
        "deepcli benchmark trends --json" => "Check benchmark trends",
        DEFAULT_BENCHMARK_CURRENT_BASELINE_TEMPLATE_ACTION => "Capture current benchmark baseline",
        DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION => "Create competitor baseline template",
        DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION => "Compare against competitor baseline",
        "deepcli benchmark gate --json" => "Gate benchmark evidence",
        _ => generic_recipe_command_label(command),
    }
}

pub(crate) fn generic_recipe_command_label(command: &str) -> &'static str {
    if command.contains("preflight") {
        "Run preflight checks"
    } else if command.contains("privacy") {
        "Run privacy scan"
    } else if command.contains("gate") {
        "Run delivery gate"
    } else if command.contains("diagnose") {
        "Collect diagnostics"
    } else if command.contains("support") {
        "Create support bundle"
    } else if command.contains("completion") {
        "Manage shell completion"
    } else if command.contains("env plan") {
        "Preview environment setup"
    } else if command.contains("env test") {
        "Test local environment"
    } else if command.contains("status") {
        "Inspect status"
    } else {
        "Run command"
    }
}
