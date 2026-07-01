use super::*;
use crate::schema_ids;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone)]
pub(crate) struct RoundGoalStatus {
    pub(crate) session: SessionMetadata,
    pub(crate) source: GoalSessionSource,
    pub(crate) ready: bool,
    pub(crate) blockers: Vec<String>,
    pub(crate) plan: GoalPlanReadiness,
    pub(crate) acceptance: Vec<GoalAcceptanceEvidence>,
    pub(crate) report: String,
}

pub(crate) fn build_round_goal_status(workspace: &Path) -> Option<RoundGoalStatus> {
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

pub(crate) fn round_goal_status_json(goal: &RoundGoalStatus) -> Value {
    json!({
        "schema": schema_ids::GOAL_STATUS_SUMMARY_V1,
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
