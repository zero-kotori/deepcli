use super::*;
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn handle_session_next(
    workspace: &Path,
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let options = parse_session_next_options(args, current)?;
    let (session, note) = resolve_session_for_next_actions(
        store,
        options.session_id.as_deref(),
        options.explicit_session,
    )?;
    let report = prefix_session_note(
        format_session_next_actions(&session)?,
        &session,
        note.clone(),
    );
    let output = if options.json_output {
        format_session_next_json(workspace, &session, note.as_deref(), &report)?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn handle_session_diagnose(
    workspace: &Path,
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let options = parse_session_diagnose_options(args, current)?;
    let (session, note) = resolve_session_for_next_actions(
        store,
        options.session_id.as_deref(),
        options.explicit_session,
    )?;
    let report = prefix_session_note(
        format_session_diagnosis(&session, options.limit)?,
        &session,
        note.clone(),
    );
    let output = if options.json_output {
        format_session_diagnosis_json(workspace, &session, note.as_deref(), options.limit, &report)?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn resolve_session_for_next_actions(
    store: &SessionStore,
    id: Option<&str>,
    explicit: bool,
) -> Result<(Session, Option<String>)> {
    if let Some(id) = id {
        let session = store.load(id)?;
        if explicit || session_has_recorded_activity(&session)? {
            return Ok((session, None));
        }

        if let Some(candidate) = latest_session_with_next_action_signals(store, Some(id))? {
            return Ok((
                candidate,
                Some(format!(
                    "latest session with next action signals; current session {id} had no recorded activity"
                )),
            ));
        }

        if let Some((candidate, _, _)) = latest_session_with_recorded_activity(store, Some(id))? {
            return Ok((
                candidate,
                Some(format!(
                    "latest session with recorded activity; current session {id} had none"
                )),
            ));
        }

        return Ok((session, None));
    }

    if let Some(session) = latest_session_with_next_action_signals(store, None)? {
        return Ok((
            session,
            Some("latest session with next action signals; no current session".to_string()),
        ));
    }

    if let Some((session, _, _)) = latest_session_with_recorded_activity(store, None)? {
        return Ok((
            session,
            Some("latest session with recorded activity; no current session".to_string()),
        ));
    }

    bail!("missing session id and no session with recorded activity was found")
}

fn latest_session_with_next_action_signals(
    store: &SessionStore,
    skip_id: Option<&str>,
) -> Result<Option<Session>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        if session_has_next_action_signals(&session)? {
            return Ok(Some(session));
        }
    }
    Ok(None)
}

pub(crate) fn session_has_next_action_signals(session: &Session) -> Result<bool> {
    if matches!(
        session.metadata.state,
        SessionState::WaitingUser
            | SessionState::AwaitingApproval
            | SessionState::Paused
            | SessionState::Failed
    ) {
        return Ok(true);
    }
    if session
        .load_approval_requests()?
        .iter()
        .any(|item| item.status == ApprovalStatus::Pending)
    {
        return Ok(true);
    }
    if session
        .load_side_questions()?
        .iter()
        .any(|item| item.status == SideQuestionStatus::Open)
    {
        return Ok(true);
    }
    if session
        .load_tool_calls()?
        .iter()
        .any(is_failed_or_denied_tool_call)
    {
        return Ok(true);
    }
    if session.load_test_runs()?.iter().any(|item| !item.passed) {
        return Ok(true);
    }
    Ok(session.load_plan()?.is_some_and(|plan| {
        plan.steps.iter().any(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        })
    }))
}

#[derive(Debug, PartialEq, Eq)]
struct SessionNextOptions {
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_session_next_options(
    args: &[String],
    current: Option<String>,
) -> Result<SessionNextOptions> {
    let mut options = SessionNextOptions {
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
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
            "--current" => {
                if options.session_id.is_some() {
                    bail!("usage: /session next [--json] [--output path] [session_id|--current]");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /session next option `{value}`"),
            value => {
                if options.session_id.is_some() {
                    bail!("usage: /session next [--json] [--output path] [session_id|--current]");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
        }
    }
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

#[derive(Debug, PartialEq, Eq)]
struct SessionDiagnoseOptions {
    limit: usize,
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_session_diagnose_options(
    args: &[String],
    current: Option<String>,
) -> Result<SessionDiagnoseOptions> {
    let mut options = SessionDiagnoseOptions {
        limit: 5,
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
    let usage =
        "usage: /session diagnose [--limit n] [--json] [--output path] [session_id|--current]";
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid limit `{raw}`"))?;
                index += 2;
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
            value if index == 0 && value.parse::<usize>().is_ok() => {
                options.limit = value.parse::<usize>()?;
                index += 1;
            }
            "--current" => {
                if options.session_id.is_some() {
                    bail!("{usage}");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => {
                bail!("unsupported /session diagnose option `{value}`")
            }
            value => {
                if options.session_id.is_some() {
                    bail!("{usage}");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
        }
    }
    options.limit = options.limit.clamp(1, 100);
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

fn format_session_next_actions(session: &Session) -> Result<String> {
    let session_id = session.id();
    let short = short_id(&session_id);
    let metadata = &session.metadata;
    let activity = session.activity_summary()?;
    let title = metadata.title.as_deref().unwrap_or("<untitled>");
    let model = metadata.model.as_deref().unwrap_or("<default>");
    let mut lines = vec![
        format!("session id={short} full={session_id}"),
        format!(
            "title={} state={:?} provider={} model={}",
            title, metadata.state, metadata.provider, model
        ),
        format!(
            "activity: messages={} tools={} tests={} diffs={} backups={} approvals={} btw={} summary={}",
            activity.message_count,
            activity.tool_call_count,
            activity.test_run_count,
            activity.diff_count,
            activity.backup_count,
            activity.approval_request_count,
            activity.side_question_count,
            if activity.has_summary { "yes" } else { "no" }
        ),
    ];

    if let Some(summary) = session.load_summary()? {
        let summary = redact_sensitive_text(summary.trim());
        if !summary.is_empty() {
            lines.push(format!(
                "summary preview:\n{}",
                indent_text(&truncate_display(&summary, 600), "  ")
            ));
        }
    }

    let mut actions = Vec::new();
    match metadata.state {
        SessionState::Paused => {
            actions.push(format!("resume paused task: run `/resume {short}`"));
        }
        SessionState::Failed => {
            actions.push(format!(
                "resume failed task after inspecting diagnostics: run `/resume {short}`"
            ));
        }
        SessionState::WaitingUser => {
            actions.push(format!(
                "answer the waiting user prompt in context: run `/resume {short}`"
            ));
        }
        SessionState::AwaitingApproval => {
            actions.push(format!(
                "resolve approval-gated work: run `/approval list {short}`"
            ));
        }
        _ => {}
    }

    let approvals = session.load_approval_requests()?;
    let pending_approvals = approvals
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .collect::<Vec<_>>();
    if !pending_approvals.is_empty() {
        let latest = pending_approvals
            .last()
            .expect("pending approvals checked as non-empty");
        actions.push(format!(
            "resolve {} pending approval request(s): run `/approval list {short}`; latest={} tool={}",
            pending_approvals.len(),
            latest.id,
            latest.tool
        ));
    }

    let questions = session.load_side_questions()?;
    let open_questions = questions
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .collect::<Vec<_>>();
    if !open_questions.is_empty() {
        let latest = open_questions
            .last()
            .expect("open questions checked as non-empty");
        actions.push(format!(
            "answer {} open by-the-way question(s): run `/btw list {short}`; latest={} {}",
            open_questions.len(),
            latest.id,
            truncate_display(&latest.question, 140)
        ));
    }

    let failed_tools = load_recent_failed_tool_calls(session, 5)?;
    if !failed_tools.is_empty() {
        let latest = failed_tools
            .last()
            .expect("failed tool calls checked as non-empty");
        actions.push(format!(
            "inspect {} recent failed or denied tool call(s): run `/session tools --failed --limit 5 {short}`; latest tool={} status={:?}",
            failed_tools.len(),
            latest.tool,
            latest.status
        ));
        actions.push(format!(
            "latest failed tool output: {}",
            compact_json(&redact_sensitive_value(&latest.output), 240)
        ));
    }

    let tests = session.load_test_runs()?;
    let failed_tests = tests.iter().filter(|item| !item.passed).collect::<Vec<_>>();
    if !failed_tests.is_empty() {
        let latest = failed_tests
            .last()
            .expect("failed tests checked as non-empty");
        let stderr = compact_text_line(&redact_sensitive_text(&latest.stderr), 160);
        let stdout = compact_text_line(&redact_sensitive_text(&latest.stdout), 160);
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        actions.push(format!(
            "repair {} failed test run(s): run `/session tests --limit 5 {short}`; latest command={} exit={:?}",
            failed_tests.len(),
            latest.command,
            latest.exit_code
        ));
        if !detail.is_empty() {
            actions.push(format!("latest test output: {detail}"));
        }
    }

    if let Some(plan) = session.load_plan()? {
        let failed_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Failed)
            .collect::<Vec<_>>();
        let in_progress_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::InProgress)
            .collect::<Vec<_>>();
        let pending_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Pending)
            .collect::<Vec<_>>();
        if let Some(step) = failed_steps.last() {
            actions.push(format!(
                "repair failed plan step `{}` from `{}`: {}; then run `/resume {short}`",
                step.id,
                plan.title,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        } else if let Some(step) = in_progress_steps.last() {
            actions.push(format!(
                "continue in-progress plan step `{}` from `{}`: {}; run `/resume {short}`",
                step.id,
                plan.title,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        } else if let Some(step) = pending_steps.first() {
            actions.push(format!(
                "continue next pending plan step `{}` from `{}`: {}; run `/resume {short}`",
                step.id,
                plan.title,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        }
    }

    if actions.is_empty() {
        actions.push(
            "no blocking signals found; inspect `/status`, `/usage`, or `/trace --limit 30` for broader diagnostics".to_string(),
        );
    }

    lines.push("next actions:".to_string());
    lines.extend(actions.into_iter().map(|action| format!("- {action}")));
    lines.push("quick links:".to_string());
    lines.push(format!("- resume: `/resume {short}`"));
    lines.push(format!("- history: `/session history --limit 20 {short}`"));
    lines.push(format!("- usage: `/usage {short}`"));
    Ok(lines.join("\n"))
}

fn format_session_next_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    report: &str,
) -> Result<String> {
    let next_actions = session_next_action_items(session)?;
    let quick_links = session_quick_link_items(session);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::NEXT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "note": note,
        "session": session_next_session_json(session)?,
        "signals": session_next_signals_json(session)?,
        "checklist": local_action_checklist(&next_actions),
        "nextActions": next_actions,
        "quickLinkChecklist": local_action_checklist(&quick_links),
        "quickLinks": quick_links,
        "report": report,
    }))?)
}

fn session_next_action_items(session: &Session) -> Result<Vec<String>> {
    let short = short_id(&session.id());
    let mut actions = Vec::new();
    match session.metadata.state {
        SessionState::Paused | SessionState::Failed | SessionState::WaitingUser => {
            push_unique_action(&mut actions, format!("deepcli resume {short}"));
        }
        SessionState::AwaitingApproval => {
            push_unique_action(
                &mut actions,
                format!("deepcli approval list {short} --json"),
            );
        }
        _ => {}
    }

    let approvals = session.load_approval_requests()?;
    if approvals
        .iter()
        .any(|item| item.status == ApprovalStatus::Pending)
    {
        push_unique_action(
            &mut actions,
            format!("deepcli approval list {short} --json"),
        );
    }

    let questions = session.load_side_questions()?;
    if questions
        .iter()
        .any(|item| item.status == SideQuestionStatus::Open)
    {
        push_unique_action(&mut actions, format!("deepcli btw list {short} --json"));
    }

    if !load_recent_failed_tool_calls(session, 5)?.is_empty() {
        push_unique_action(
            &mut actions,
            format!("deepcli session tools --failed --limit 5 {short} --json"),
        );
    }

    let tests = session.load_test_runs()?;
    if tests.iter().any(|item| !item.passed) {
        push_unique_action(
            &mut actions,
            format!("deepcli session tests --limit 5 {short} --json"),
        );
    }

    if let Some(plan) = session.load_plan()? {
        if plan.steps.iter().any(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        }) {
            push_unique_action(&mut actions, format!("deepcli resume {short}"));
        }
    }

    if actions.is_empty() {
        push_unique_action(&mut actions, format!("deepcli status {short} --json"));
        push_unique_action(&mut actions, format!("deepcli usage {short} --json"));
        push_unique_action(
            &mut actions,
            format!("deepcli trace --limit 30 {short} --json"),
        );
    }

    Ok(actions)
}

fn session_quick_link_items(session: &Session) -> Vec<String> {
    let short = short_id(&session.id());
    vec![
        format!("deepcli resume {short}"),
        format!("deepcli session history {short} --limit 20 --json"),
        format!("deepcli usage {short} --json"),
    ]
}

pub(crate) fn push_unique_action(actions: &mut Vec<String>, action: String) {
    if !actions.iter().any(|existing| existing == &action) {
        actions.push(action);
    }
}

fn session_next_session_json(session: &Session) -> Result<Value> {
    let activity = session.activity_summary()?;
    let summary_preview = session.load_summary()?.and_then(|summary| {
        let redacted = redact_sensitive_text(summary.trim());
        if redacted.is_empty() {
            None
        } else {
            Some(truncate_display(&redacted, 600))
        }
    });
    let plan = session.load_plan()?.map(|plan| {
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Completed)
            .count();
        let pending = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Pending)
            .count();
        let in_progress = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::InProgress)
            .count();
        let failed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Failed)
            .count();
        json!({
            "title": plan.title.as_str(),
            "completed": completed,
            "pending": pending,
            "inProgress": in_progress,
            "failed": failed,
            "total": plan.steps.len(),
            "updatedAt": &plan.updated_at,
        })
    });
    Ok(json!({
        "id": session.id().to_string(),
        "shortId": short_id(&session.id()),
        "title": session.metadata.title.as_deref(),
        "state": &session.metadata.state,
        "provider": session.metadata.provider.as_str(),
        "model": session.metadata.model.as_deref(),
        "createdAt": &session.metadata.created_at,
        "updatedAt": &session.metadata.updated_at,
        "activity": {
            "messages": activity.message_count,
            "tools": activity.tool_call_count,
            "tests": activity.test_run_count,
            "diffs": activity.diff_count,
            "backups": activity.backup_count,
            "approvals": activity.approval_request_count,
            "sideQuestions": activity.side_question_count,
            "hasSummary": activity.has_summary,
        },
        "summaryPreview": summary_preview,
        "plan": plan.unwrap_or(Value::Null),
    }))
}

fn session_next_signals_json(session: &Session) -> Result<Value> {
    let pending_approvals = session
        .load_approval_requests()?
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let open_questions = session
        .load_side_questions()?
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let failed_or_denied_tools = session
        .load_tool_calls()?
        .iter()
        .filter(|item| is_failed_or_denied_tool_call(item))
        .count();
    let failed_tests = session
        .load_test_runs()?
        .iter()
        .filter(|item| !item.passed)
        .count();
    let incomplete_plan_steps = session
        .load_plan()?
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.status,
                        PlanStepStatus::Pending
                            | PlanStepStatus::InProgress
                            | PlanStepStatus::Failed
                    )
                })
                .count()
        })
        .unwrap_or(0);
    let state_needs_attention = matches!(
        session.metadata.state,
        SessionState::WaitingUser
            | SessionState::AwaitingApproval
            | SessionState::Paused
            | SessionState::Failed
    );
    Ok(json!({
        "stateNeedsAttention": state_needs_attention,
        "pendingApprovals": pending_approvals,
        "openByTheWayQuestions": open_questions,
        "failedOrDeniedTools": failed_or_denied_tools,
        "failedTests": failed_tests,
        "incompletePlanSteps": incomplete_plan_steps,
        "hasNextActionSignals": session_has_next_action_signals(session)?,
    }))
}

pub(crate) fn format_session_diagnosis(session: &Session, limit: usize) -> Result<String> {
    let session_id = session.id();
    let short = short_id(&session_id);
    let metadata = &session.metadata;
    let activity = session.activity_summary()?;
    let title = metadata.title.as_deref().unwrap_or("<untitled>");
    let model = metadata.model.as_deref().unwrap_or("<default>");
    let approvals = session.load_approval_requests()?;
    let pending_approvals = approvals
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let questions = session.load_side_questions()?;
    let open_questions = questions
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let failed_tools = load_recent_failed_tool_calls(session, limit)?;
    let tests = session.load_test_runs()?;
    let failed_tests = tests.iter().filter(|item| !item.passed).count();
    let recent_tests = session.load_recent_test_runs(limit)?;
    let plan = session.load_plan()?;
    let incomplete_steps = plan
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.status,
                        PlanStepStatus::Pending
                            | PlanStepStatus::InProgress
                            | PlanStepStatus::Failed
                    )
                })
                .count()
        })
        .unwrap_or(0);

    let mut lines = vec![
        "session diagnosis".to_string(),
        format!("session id={short} full={session_id}"),
        format!(
            "title={} state={:?} provider={} model={}",
            title, metadata.state, metadata.provider, model
        ),
        format!(
            "activity: messages={} tools={} tests={} diffs={} backups={} approvals={} btw={} summary={}",
            activity.message_count,
            activity.tool_call_count,
            activity.test_run_count,
            activity.diff_count,
            activity.backup_count,
            activity.approval_request_count,
            activity.side_question_count,
            if activity.has_summary { "yes" } else { "no" }
        ),
        "signals:".to_string(),
        format!("- pending approvals: {pending_approvals}"),
        format!("- open by-the-way questions: {open_questions}"),
        format!("- recent failed or denied tools: {}", failed_tools.len()),
        format!("- failed test runs: {failed_tests}"),
        format!("- incomplete plan steps: {incomplete_steps}"),
    ];

    if let Some(summary) = session.load_summary()? {
        let summary = summary.trim();
        if !summary.is_empty() {
            lines.push(format!(
                "summary preview:\n{}",
                indent_text(&truncate_display(summary, 600), "  ")
            ));
        }
    }

    lines.push("recent failures:".to_string());
    if failed_tools.is_empty() {
        lines.push("  no failed or denied tool calls found".to_string());
    } else {
        lines.push(indent_text(
            &format_tool_calls(&failed_tools, limit, ToolCallFilter { failed_only: true }),
            "  ",
        ));
    }

    lines.push("recent tests:".to_string());
    lines.push(indent_text(&format_test_runs(&recent_tests, limit), "  "));

    if let Some(plan) = plan {
        lines.push("plan status:".to_string());
        lines.push(format!(
            "  title={} steps={} incomplete={}",
            plan.title,
            plan.steps.len(),
            incomplete_steps
        ));
        for step in plan.steps.iter().filter(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        }) {
            lines.push(format!(
                "  - [{}] {}: {}",
                match step.status {
                    PlanStepStatus::Pending => "pending",
                    PlanStepStatus::InProgress => "in_progress",
                    PlanStepStatus::Completed => "completed",
                    PlanStepStatus::Failed => "failed",
                },
                step.id,
                truncate_display(&redact_sensitive_text(&step.description), 180)
            ));
        }
    }

    let next_report = format_session_next_actions(session)?;
    let next_actions = session_next_action_items_from_report(&next_report);
    lines.push("recommended next actions:".to_string());
    if next_actions.is_empty() {
        lines
            .push("- inspect `/trace --limit 30` and `/usage` for broader diagnostics".to_string());
    } else {
        lines.extend(next_actions.into_iter().map(|action| format!("- {action}")));
    }
    lines.push("quick links:".to_string());
    lines.push(format!("- resume: `/resume {short}`"));
    lines.push(format!(
        "- failed tools: `/session tools --failed --limit {limit} {short}`"
    ));
    lines.push(format!("- tests: `/session tests --limit {limit} {short}`"));
    lines.push(format!("- trace: `/trace --limit 30 {short}`"));
    lines.push(format!("- usage: `/usage {short}`"));
    Ok(lines.join("\n"))
}

fn format_session_diagnosis_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    limit: usize,
    report: &str,
) -> Result<String> {
    let activity = session.activity_summary()?;
    let approvals = session.load_approval_requests()?;
    let pending_approvals = approvals
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let questions = session.load_side_questions()?;
    let open_questions = questions
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let tool_calls = session.load_tool_calls()?;
    let failed_or_denied_tools = tool_calls
        .iter()
        .filter(|item| is_failed_or_denied_tool_call(item))
        .count();
    let recent_failed_tools = load_recent_failed_tool_calls(session, limit)?;
    let tests = session.load_test_runs()?;
    let failed_tests = tests.iter().filter(|item| !item.passed).count();
    let recent_tests = session.load_recent_test_runs(limit)?;
    let plan = session.load_plan()?;
    let incomplete_plan_steps = plan
        .as_ref()
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.status,
                        PlanStepStatus::Pending
                            | PlanStepStatus::InProgress
                            | PlanStepStatus::Failed
                    )
                })
                .count()
        })
        .unwrap_or(0);
    let summary_preview = session.load_summary()?.and_then(|summary| {
        let redacted = redact_sensitive_text(summary.trim());
        (!redacted.is_empty()).then(|| truncate_display(&redacted, 600))
    });

    let recommended_next_actions = session_next_action_items(session)?;
    let quick_links = session_quick_link_items(session);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_DIAGNOSE_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "note": note,
        "limit": limit,
        "session": {
            "id": session.id().to_string(),
            "shortId": short_id(&session.id()),
            "title": session.metadata.title.as_deref(),
            "state": &session.metadata.state,
            "provider": session.metadata.provider.as_str(),
            "model": session.metadata.model.as_deref(),
            "createdAt": &session.metadata.created_at,
            "updatedAt": &session.metadata.updated_at,
            "activity": {
                "messages": activity.message_count,
                "tools": activity.tool_call_count,
                "tests": activity.test_run_count,
                "diffs": activity.diff_count,
                "backups": activity.backup_count,
                "approvals": activity.approval_request_count,
                "sideQuestions": activity.side_question_count,
                "hasSummary": activity.has_summary,
            },
            "summaryPreview": summary_preview,
        },
        "signals": {
            "pendingApprovals": pending_approvals,
            "openByTheWayQuestions": open_questions,
            "failedOrDeniedTools": failed_or_denied_tools,
            "recentFailedOrDeniedTools": recent_failed_tools.len(),
            "failedTests": failed_tests,
            "recentTests": recent_tests.len(),
            "incompletePlanSteps": incomplete_plan_steps,
            "hasNextActionSignals": session_has_next_action_signals(session)?,
        },
        "recentFailures": recent_failed_tools
            .iter()
            .map(session_diagnosis_tool_call_json)
            .collect::<Vec<_>>(),
        "recentTests": recent_tests
            .iter()
            .map(session_diagnosis_test_json)
            .collect::<Vec<_>>(),
        "plan": plan
            .as_ref()
            .map(session_diagnosis_plan_json)
            .unwrap_or(Value::Null),
        "checklist": local_action_checklist(&recommended_next_actions),
        "recommendedNextActions": recommended_next_actions,
        "quickLinkChecklist": local_action_checklist(&quick_links),
        "quickLinks": quick_links,
        "report": report,
    }))?)
}

fn session_diagnosis_tool_call_json(record: &ToolCallRecord) -> Value {
    json!({
        "createdAt": &record.created_at,
        "status": &record.status,
        "tool": record.tool.as_str(),
        "decision": record.decision.as_ref().map(|decision| json!(decision)).unwrap_or(Value::Null),
        "input": redact_sensitive_value(&record.input),
        "output": redact_sensitive_value(&record.output),
        "line": redact_sensitive_text(&format_tool_call_record(record)),
    })
}

fn session_diagnosis_test_json(record: &TestRunRecord) -> Value {
    json!({
        "createdAt": &record.created_at,
        "passed": record.passed,
        "exitCode": record.exit_code,
        "command": record.command.as_str(),
        "stdoutPreview": truncate_display(&redact_sensitive_text(&record.stdout), 1_000),
        "stderrPreview": truncate_display(&redact_sensitive_text(&record.stderr), 1_000),
    })
}

fn session_diagnosis_plan_json(plan: &crate::session::Plan) -> Value {
    let completed = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Completed)
        .count();
    let pending = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Pending)
        .count();
    let in_progress = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::InProgress)
        .count();
    let failed = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Failed)
        .count();
    let incomplete_steps = plan
        .steps
        .iter()
        .filter(|step| {
            matches!(
                step.status,
                PlanStepStatus::Pending | PlanStepStatus::InProgress | PlanStepStatus::Failed
            )
        })
        .map(|step| {
            json!({
                "id": step.id.as_str(),
                "status": &step.status,
                "description": truncate_display(&redact_sensitive_text(&step.description), 600),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "title": plan.title.as_str(),
        "updatedAt": &plan.updated_at,
        "total": plan.steps.len(),
        "completed": completed,
        "pending": pending,
        "inProgress": in_progress,
        "failed": failed,
        "incomplete": incomplete_steps.len(),
        "incompleteSteps": incomplete_steps,
    })
}

fn session_next_action_items_from_report(report: &str) -> Vec<String> {
    let mut in_next_actions = false;
    let mut actions = Vec::new();
    for line in report.lines() {
        if line == "next actions:" {
            in_next_actions = true;
            continue;
        }
        if in_next_actions && line == "quick links:" {
            break;
        }
        if in_next_actions {
            if let Some(item) = line.strip_prefix("- ") {
                actions.push(item.to_string());
            }
        }
    }
    actions
}
