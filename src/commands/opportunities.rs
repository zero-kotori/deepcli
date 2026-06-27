use super::{
    build_round_report, dedup_preserve_order, required_arg, scorecard_action_checklist,
    scorecard_opportunities_json, scorecard_opportunity_effort_counts_json,
    scorecard_opportunity_priority_counts_json, scorecard_opportunity_summary_text,
    scorecard_recommended_opportunity_json, set_command_output_path, write_command_output,
    RoundReport, ScorecardOpportunity, DEFAULT_ROUND_SCORE_THRESHOLD,
};
use crate::config::AppConfig;
use crate::tools::ToolRegistry;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct OpportunitiesOptions {
    json_output: bool,
    output_path: Option<String>,
    priority_filter: Option<&'static str>,
    effort_filter: Option<&'static str>,
}

pub(crate) fn handle_opportunities(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_opportunities_options(&args)?;
    let round = build_round_report(
        workspace,
        config,
        registry,
        DEFAULT_ROUND_SCORE_THRESHOLD,
        None,
    );
    let opportunities = filtered_opportunities(&round.opportunities, &options);
    let next_actions = opportunity_next_actions(&opportunities, &round);
    let report =
        format_opportunities_text(workspace, &round, &opportunities, &options, &next_actions);
    let output = if options.json_output {
        format_opportunities_json(
            workspace,
            &round,
            &opportunities,
            &options,
            &next_actions,
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

fn parse_opportunities_options(args: &[String]) -> Result<OpportunitiesOptions> {
    let mut options = OpportunitiesOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--priority" => {
                options.priority_filter = Some(parse_opportunity_priority_filter(required_arg(
                    args,
                    index + 1,
                    "priority",
                )?)?);
                index += 2;
            }
            value if value.starts_with("--priority=") => {
                options.priority_filter = Some(parse_opportunity_priority_filter(
                    value.trim_start_matches("--priority="),
                )?);
                index += 1;
            }
            "--effort" => {
                options.effort_filter = Some(parse_opportunity_effort_filter(required_arg(
                    args,
                    index + 1,
                    "effort",
                )?)?);
                index += 2;
            }
            value if value.starts_with("--effort=") => {
                options.effort_filter = Some(parse_opportunity_effort_filter(
                    value.trim_start_matches("--effort="),
                )?);
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
            value => bail!("unsupported /opportunities option `{value}`"),
        }
    }
    Ok(options)
}

fn parse_opportunity_priority_filter(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "high" => Ok("high"),
        "medium" => Ok("medium"),
        "low" => Ok("low"),
        "other" => Ok("other"),
        _ => bail!(
            "unsupported /opportunities priority `{value}`; expected high, medium, low, or other"
        ),
    }
}

fn parse_opportunity_effort_filter(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "high" => Ok("high"),
        "medium" => Ok("medium"),
        "low" => Ok("low"),
        "other" => Ok("other"),
        _ => {
            bail!(
                "unsupported /opportunities effort `{value}`; expected high, medium, low, or other"
            )
        }
    }
}

fn filtered_opportunities(
    opportunities: &[ScorecardOpportunity],
    options: &OpportunitiesOptions,
) -> Vec<ScorecardOpportunity> {
    opportunities
        .iter()
        .filter(|opportunity| {
            options
                .priority_filter
                .is_none_or(|priority| opportunity.priority == priority)
                && options
                    .effort_filter
                    .is_none_or(|effort| opportunity.effort == effort)
        })
        .cloned()
        .collect()
}

fn opportunity_next_actions(
    opportunities: &[ScorecardOpportunity],
    round: &RoundReport,
) -> Vec<String> {
    let mut actions = opportunities
        .iter()
        .flat_map(|opportunity| opportunity.next_actions.clone())
        .collect::<Vec<_>>();
    if actions.is_empty() {
        actions.extend(round.next_actions.clone());
    }
    dedup_preserve_order(actions)
}

fn format_opportunities_text(
    workspace: &Path,
    round: &RoundReport,
    opportunities: &[ScorecardOpportunity],
    options: &OpportunitiesOptions,
    next_actions: &[String],
) -> String {
    let mut lines = vec![
        "deepcli opportunities".to_string(),
        format!("workspace: {}", workspace.display()),
        format!("round status: {}", round.status),
        format!("opportunities: {}", opportunities.len()),
    ];
    if let Some(priority) = options.priority_filter {
        lines.push(format!("filter: priority={priority}"));
    }
    if let Some(effort) = options.effort_filter {
        lines.push(format!("filter: effort={effort}"));
    }
    if options.priority_filter.is_some() || options.effort_filter.is_some() {
        lines.push(format!(
            "total opportunities: {}",
            round.opportunities.len()
        ));
        lines.push(format!(
            "filtered out: {}",
            round
                .opportunities
                .len()
                .saturating_sub(opportunities.len())
        ));
    }
    if opportunities.is_empty() {
        lines.push(
            "no non-blocking opportunities are available; follow the round actions first"
                .to_string(),
        );
    } else {
        lines.extend(scorecard_opportunity_summary_text(opportunities));
        for opportunity in opportunities {
            lines.push(format!(
                "- {}: {} ({})",
                opportunity.id, opportunity.title, opportunity.status
            ));
            lines.push(format!("  summary: {}", opportunity.summary));
            lines.push(format!("  impact: {}", opportunity.impact));
            lines.push(format!("  priority: {}", opportunity.priority));
            lines.push(format!("  effort: {}", opportunity.effort));
            lines.push("  next actions:".to_string());
            lines.extend(
                opportunity
                    .next_actions
                    .iter()
                    .map(|action| format!("    - {action}")),
            );
        }
    }
    lines.push("next actions:".to_string());
    lines.extend(next_actions.iter().map(|action| format!("  - {action}")));
    lines.join("\n")
}

fn format_opportunities_json(
    workspace: &Path,
    round: &RoundReport,
    opportunities: &[ScorecardOpportunity],
    options: &OpportunitiesOptions,
    next_actions: &[String],
    report: &str,
) -> Result<String> {
    let checklist = scorecard_action_checklist(next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.opportunities.v1",
        "status": round.status,
        "ready": round.status == "ready",
        "workspace": workspace.display().to_string(),
        "source": {
            "schema": "deepcli.round.v1",
            "command": "deepcli round --json",
        },
        "filter": opportunity_filter_json(options),
        "summary": opportunities_summary_json(round, opportunities, options, &checklist),
        "opportunityCount": opportunities.len(),
        "totalOpportunityCount": round.opportunities.len(),
        "filteredOutOpportunityCount": round.opportunities.len().saturating_sub(opportunities.len()),
        "recommendedOpportunity": scorecard_recommended_opportunity_json(opportunities),
        "opportunityPriorityCounts": scorecard_opportunity_priority_counts_json(opportunities),
        "opportunityEffortCounts": scorecard_opportunity_effort_counts_json(opportunities),
        "availablePriorityCounts": scorecard_opportunity_priority_counts_json(&round.opportunities),
        "availableEffortCounts": scorecard_opportunity_effort_counts_json(&round.opportunities),
        "opportunities": scorecard_opportunities_json(opportunities),
        "nextActions": next_actions,
        "checklist": checklist,
        "report": report,
    }))?)
}

fn opportunities_summary_json(
    round: &RoundReport,
    opportunities: &[ScorecardOpportunity],
    options: &OpportunitiesOptions,
    checklist: &[Value],
) -> Value {
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
        "status": round.status,
        "ready": round.status == "ready",
        "priorityFilter": options.priority_filter,
        "effortFilter": options.effort_filter,
        "opportunityCount": opportunities.len(),
        "totalOpportunityCount": round.opportunities.len(),
        "filteredOutOpportunityCount": round.opportunities.len().saturating_sub(opportunities.len()),
        "recommendedOpportunityId": opportunities
            .first()
            .map(|opportunity| Value::from(opportunity.id))
            .unwrap_or(Value::Null),
        "recommendedAction": recommended_action,
        "recommendedActionLabel": recommended_action_label,
    })
}

fn opportunity_filter_json(options: &OpportunitiesOptions) -> Value {
    json!({
        "priority": options.priority_filter,
        "effort": options.effort_filter,
    })
}
