use super::{
    compact_text_line, display_optional_u64, display_optional_usize,
    latest_session_with_recorded_activity, local_action_checklist, required_arg,
    session_has_next_action_signals, session_storage_bytes, set_command_output_path, short_id,
    status_u128_value, write_command_output, CommandContext,
};
use crate::schema_ids;
use crate::session::{PlanStepStatus, Session, SessionStore};
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub(super) fn handle_status(context: CommandContext<'_>, args: Vec<String>) -> Result<String> {
    let options = parse_status_options(&args)?;
    let report = format_status_report_text(&context)?;
    let output = if options.json_output {
        format_status_report_json(&context, &report)?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(context.workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, PartialEq, Eq)]
struct StatusOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn parse_status_options(args: &[String]) -> Result<StatusOptions> {
    let mut options = StatusOptions {
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
            value => bail!("unsupported /status option `{value}`"),
        }
    }
    Ok(options)
}

fn format_status_report_text(context: &CommandContext<'_>) -> Result<String> {
    let mut lines = vec![
        format!("workspace: {}", context.workspace.display()),
        format!(
            "session: {}",
            context.session_id.as_deref().unwrap_or("<none>")
        ),
        format!(
            "registered tools: {}",
            context.registry.declarations().len()
        ),
        format!(
            "token warning threshold: {}",
            context.config.usage.token_warning_threshold
        ),
        format!(
            "provider turn timeout: {}s",
            context.config.agent.provider_turn_timeout_seconds
        ),
    ];

    let store = SessionStore::new(context.workspace);
    if let Some(session_id) = context.session_id.as_deref() {
        match store.load(session_id) {
            Ok(session) => append_status_session_lines(&mut lines, "active session", &session)?,
            Err(error) => lines.push(format!(
                "session status: unavailable ({})",
                compact_text_line(&error.to_string(), 200)
            )),
        }
    } else if let Some((session, _, _)) = latest_session_with_recorded_activity(&store, None)? {
        append_status_session_lines(&mut lines, "latest session", &session)?;
        lines.push(format!(
            "note: no active session; showing latest recorded activity. Run `/resume {}` to continue it.",
            short_id(&session.id())
        ));
    } else {
        lines.push("latest session: none with recorded activity".to_string());
    }

    Ok(lines.join("\n"))
}

fn format_status_report_json(context: &CommandContext<'_>, report: &str) -> Result<String> {
    let store = SessionStore::new(context.workspace);
    let (session_source, session_value, note) = if let Some(session_id) =
        context.session_id.as_deref()
    {
        match store.load(session_id) {
            Ok(session) => (
                "active",
                status_session_json(&session)?,
                Option::<String>::None,
            ),
            Err(error) => (
                "active",
                Value::Null,
                Some(format!(
                    "active session unavailable: {}",
                    compact_text_line(&error.to_string(), 200)
                )),
            ),
        }
    } else if let Some((session, _, _)) = latest_session_with_recorded_activity(&store, None)? {
        (
            "latest",
            status_session_json(&session)?,
            Some(format!(
                "no active session; showing latest recorded activity. Run `/resume {}` to continue it.",
                short_id(&session.id())
            )),
        )
    } else {
        (
            "none",
            Value::Null,
            Some("no active session and no session with recorded activity".to_string()),
        )
    };

    let checklist = session_value
        .get("checklist")
        .cloned()
        .unwrap_or_else(|| json!([]));
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::STATUS_V1,
        "status": "ok",
        "workspace": context.workspace.display().to_string(),
        "activeSession": context.session_id.as_deref(),
        "registeredTools": context.registry.declarations().len(),
        "tokenWarningThreshold": context.config.usage.token_warning_threshold,
        "providerTurnTimeoutSeconds": context.config.agent.provider_turn_timeout_seconds,
        "sessionSource": session_source,
        "checklist": checklist,
        "session": session_value,
        "note": note,
        "report": report,
    }))?)
}

fn status_session_json(session: &Session) -> Result<Value> {
    let summary = session.activity_summary()?;
    let audits = session.load_audit_events()?;
    let usage = super::usage::summarize_audit_usage(&audits);
    let has_next_action_signals = session_has_next_action_signals(session)?;
    let short = short_id(&session.id());
    let next_actions = status_next_actions(&short, has_next_action_signals);
    let plan = session.load_plan()?.map(|plan| {
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Completed)
            .count();
        json!({
            "title": plan.title,
            "completed": completed,
            "total": plan.steps.len(),
            "updatedAt": plan.updated_at,
        })
    });
    Ok(json!({
        "id": session.id().to_string(),
        "shortId": short.as_str(),
        "title": session.metadata.title.as_deref(),
        "state": &session.metadata.state,
        "provider": session.metadata.provider.as_str(),
        "model": session.metadata.model.as_deref(),
        "createdAt": &session.metadata.created_at,
        "updatedAt": &session.metadata.updated_at,
        "activity": {
            "messages": summary.message_count,
            "tools": summary.tool_call_count,
            "tests": summary.test_run_count,
            "diffs": summary.diff_count,
            "backups": summary.backup_count,
            "sideQuestions": summary.side_question_count,
            "approvals": summary.approval_request_count,
            "hasSummary": summary.has_summary,
        },
        "usage": {
            "providerTurnsStarted": usage.provider_turns_started,
            "providerTurnsCompleted": usage.provider_turns_completed,
            "providerElapsedMs": status_u128_value(usage.provider_elapsed_ms),
            "providerMaxElapsedMs": usage.provider_max_elapsed_ms.map(status_u128_value),
            "providerToolCalls": usage.provider_tool_calls,
            "promptTokens": usage.prompt_tokens,
            "completionTokens": usage.completion_tokens,
            "totalTokens": usage.total_tokens,
            "promptCacheHitTokens": usage.prompt_cache_hit_tokens,
            "promptCacheMissTokens": usage.prompt_cache_miss_tokens,
        },
        "context": {
            "compactedTurns": usage.compacted_turns,
            "auditEvents": audits.len(),
            "maxRequestBytes": usage.max_request_bytes,
            "latestRequestBytes": usage.latest_request_bytes,
            "storageBytes": session_storage_bytes(session.path())?,
        },
        "plan": plan.unwrap_or(Value::Null),
        "nextActionSignals": has_next_action_signals,
        "checklist": local_action_checklist(&next_actions),
        "nextActions": next_actions,
    }))
}

fn status_next_actions(short: &str, has_next_action_signals: bool) -> Vec<String> {
    if has_next_action_signals {
        vec![
            format!("deepcli next {short}"),
            format!("deepcli session diagnose {short}"),
        ]
    } else {
        vec![
            format!("deepcli usage {short}"),
            format!("deepcli trace --limit 20 {short}"),
        ]
    }
}

fn append_status_session_lines(
    lines: &mut Vec<String>,
    label: &str,
    session: &Session,
) -> Result<()> {
    let summary = session.activity_summary()?;
    let audits = session.load_audit_events()?;
    let usage = super::usage::summarize_audit_usage(&audits);
    lines.push(format!(
        "{label}: {} title={} state={:?} provider={} model={}",
        session.id(),
        session.metadata.title.as_deref().unwrap_or("<untitled>"),
        session.metadata.state,
        session.metadata.provider,
        session.metadata.model.as_deref().unwrap_or("<unset>")
    ));
    lines.push(format!(
        "activity: messages={} tools={} tests={} diffs={} backups={} side_questions={} approvals={} summary={}",
        summary.message_count,
        summary.tool_call_count,
        summary.test_run_count,
        summary.diff_count,
        summary.backup_count,
        summary.side_question_count,
        summary.approval_request_count,
        summary.has_summary
    ));
    lines.push(format!(
        "provider turns: started={} completed={} total_elapsed_ms={}",
        usage.provider_turns_started, usage.provider_turns_completed, usage.provider_elapsed_ms
    ));
    lines.push(format!(
        "tokens: prompt={} completion={} total={} cache_hit={} cache_miss={}",
        display_optional_u64(usage.prompt_tokens),
        display_optional_u64(usage.completion_tokens),
        display_optional_u64(usage.total_tokens),
        display_optional_u64(usage.prompt_cache_hit_tokens),
        display_optional_u64(usage.prompt_cache_miss_tokens)
    ));
    lines.push(format!(
        "context: compacted_turns={} audit_events={} max_request_bytes={} latest_request_bytes={} storage_bytes={}",
        usage.compacted_turns,
        audits.len(),
        display_optional_usize(usage.max_request_bytes),
        display_optional_usize(usage.latest_request_bytes),
        session_storage_bytes(session.path())?
    ));

    if let Some(plan) = session.load_plan()? {
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Completed)
            .count();
        lines.push(format!("plan: {completed}/{} completed", plan.steps.len()));
    }

    if session_has_next_action_signals(session)? {
        lines.push(format!(
            "next: run `/next {}` or `/session diagnose {}`",
            short_id(&session.id()),
            short_id(&session.id())
        ));
    } else {
        lines.push(format!(
            "next: run `/usage {}` or `/trace --limit 20 {}` for deeper diagnostics",
            short_id(&session.id()),
            short_id(&session.id())
        ));
    }

    Ok(())
}
