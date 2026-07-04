use super::TuiState;
use crate::runtime::{
    session_environment_observations_from_tool_calls, session_usage_observation_from_audit_events,
    AgentRuntime, SessionMonitor, SessionObservation, SessionObservationApproval,
    SessionObservationEvent, SessionObservationQuestion, SessionObservationTest,
};
use crate::session::{
    ApprovalStatus, Plan, PlanStepStatus, Session, SessionStore, SideQuestionStatus, ToolCallStatus,
};
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActiveSessionRef {
    pub(super) workspace: PathBuf,
    pub(super) session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeaderStatus {
    pub(super) session: String,
    pub(super) title: String,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) state: String,
}

pub(super) fn active_session_ref(runtime: &AgentRuntime) -> ActiveSessionRef {
    ActiveSessionRef {
        workspace: runtime.workspace().to_path_buf(),
        session_id: runtime.session_id(),
    }
}

pub(super) fn sync_active_session_ref(state: &mut TuiState) {
    if let Some(runtime) = &state.runtime {
        state.active_session = Some(active_session_ref(runtime));
    }
}

pub(super) fn session_monitor_for_state(state: &TuiState) -> Option<SessionMonitor> {
    if let Some(monitor) = state
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.session_monitor().ok())
    {
        return Some(monitor);
    }
    state
        .active_session
        .as_ref()
        .and_then(|active| load_active_session_monitor(active).ok())
}

pub(super) fn header_status_for_state(state: &TuiState) -> HeaderStatus {
    if let Some(runtime) = state.runtime.as_ref() {
        return HeaderStatus {
            session: runtime.session_id(),
            title: runtime
                .session_title()
                .map(str::to_string)
                .unwrap_or_else(|| "<untitled>".to_string()),
            provider: runtime.provider_name().to_string(),
            model: runtime
                .model_name()
                .map(str::to_string)
                .unwrap_or_else(|| "<unset>".to_string()),
            state: runtime.state_label(),
        };
    }
    state
        .active_session
        .as_ref()
        .and_then(|active| load_active_session_header(active).ok())
        .unwrap_or_else(|| HeaderStatus {
            session: "<running>".to_string(),
            title: "<untitled>".to_string(),
            provider: "<running>".to_string(),
            model: "<unset>".to_string(),
            state: "Running".to_string(),
        })
}

pub(super) fn load_active_session_header(active: &ActiveSessionRef) -> Result<HeaderStatus> {
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    Ok(HeaderStatus {
        session: session.id().to_string(),
        title: session
            .metadata
            .title
            .clone()
            .unwrap_or_else(|| "<untitled>".to_string()),
        provider: session.metadata.provider,
        model: session
            .metadata
            .model
            .clone()
            .unwrap_or_else(|| "<unset>".to_string()),
        state: format!("{:?}", session.metadata.state),
    })
}

pub(super) fn load_active_session_monitor(active: &ActiveSessionRef) -> Result<SessionMonitor> {
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    session_monitor_from_session(&session)
}

pub(super) fn session_monitor_from_session(session: &Session) -> Result<SessionMonitor> {
    let plan = session.load_plan()?;
    let (plan_total, plan_completed, plan_in_progress, plan_failed, current_step) = plan
        .as_ref()
        .map(summarize_plan_for_tui)
        .unwrap_or((0, 0, 0, 0, None));
    let latest_test = session
        .load_recent_test_runs(1)?
        .into_iter()
        .last()
        .map(|test| SessionObservationTest {
            command: test.command,
            passed: test.passed,
            exit_code: test.exit_code,
        });
    let pending_approvals = session
        .load_approval_requests()?
        .iter()
        .filter(|request| request.status == ApprovalStatus::Pending)
        .count();
    let open_questions = session
        .load_side_questions()?
        .iter()
        .filter(|question| question.status == SideQuestionStatus::Open)
        .count();
    let tools = session.load_tool_calls()?;
    let failed_tools = tools
        .iter()
        .filter(|tool| matches!(tool.status, ToolCallStatus::Failed | ToolCallStatus::Denied))
        .count();
    let observation = SessionObservation {
        state: format!("{:?}", session.metadata.state),
        plan_total,
        plan_completed,
        plan_in_progress,
        plan_failed,
        current_step,
        latest_test,
        pending_approvals,
        open_questions,
        tool_calls: tools.len(),
        failed_tools,
    };
    let recent_tests = session
        .load_recent_test_runs(6)?
        .into_iter()
        .map(|test| SessionObservationTest {
            command: test.command,
            passed: test.passed,
            exit_code: test.exit_code,
        })
        .collect();
    let recent_environment =
        session_environment_observations_from_tool_calls(&session.load_tool_calls()?, 6);
    let pending_approvals = session
        .load_approval_requests()?
        .into_iter()
        .filter(|request| request.status == ApprovalStatus::Pending)
        .map(|request| SessionObservationApproval {
            id: request.id.to_string(),
            tool: request.tool,
            risk: format!("{:?}", request.decision.risk),
            reason: request.decision.reason,
        })
        .collect();
    let open_questions = session
        .load_side_questions()?
        .into_iter()
        .filter(|question| question.status == SideQuestionStatus::Open)
        .map(|question| SessionObservationQuestion {
            id: question.id.to_string(),
            question: question.question,
            options: question.options,
        })
        .collect();
    let events = session.load_audit_events()?;
    let usage = session_usage_observation_from_audit_events(&events);
    let skip = events.len().saturating_sub(8);
    let recent_events = events
        .into_iter()
        .skip(skip)
        .map(|event| SessionObservationEvent {
            event_type: event.event_type,
            created_at: event.created_at.format("%H:%M:%S").to_string(),
        })
        .collect();

    Ok(SessionMonitor {
        observation,
        usage,
        recent_tests,
        recent_environment,
        pending_approvals,
        open_questions,
        recent_events,
    })
}

pub(super) fn summarize_plan_for_tui(plan: &Plan) -> (usize, usize, usize, usize, Option<String>) {
    let mut completed = 0;
    let mut in_progress = 0;
    let mut failed = 0;
    let mut current = None;
    for step in &plan.steps {
        match step.status {
            PlanStepStatus::Completed => completed += 1,
            PlanStepStatus::InProgress => {
                in_progress += 1;
                if current.is_none() {
                    current = Some(step.description.clone());
                }
            }
            PlanStepStatus::Failed => {
                failed += 1;
                if current.is_none() {
                    current = Some(step.description.clone());
                }
            }
            PlanStepStatus::Pending => {}
        }
    }
    (plan.steps.len(), completed, in_progress, failed, current)
}

pub(super) fn workspace_for_state(state: &TuiState) -> Option<&Path> {
    state
        .runtime
        .as_ref()
        .map(AgentRuntime::workspace)
        .or_else(|| {
            state
                .active_session
                .as_ref()
                .map(|active| active.workspace.as_path())
        })
}
