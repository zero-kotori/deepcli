use super::*;
use serde_json::{json, Value};
use std::path::Path;

pub(crate) const SCORECARD_ROUND_REPORT_ACTION: &str = "deepcli round --json";
pub(crate) const SCORECARD_OPPORTUNITIES_ACTION: &str = "deepcli opportunities --json";

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

pub(crate) fn scorecard_product_opportunities(
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

pub(crate) fn opportunity_baseline_next_actions(actions: Vec<String>) -> Vec<String> {
    let mut next_actions = vec!["deepcli benchmark baselines --json".to_string()];
    next_actions.extend(actions);
    dedup_preserve_order(next_actions)
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
