use super::{
    dedup_preserve_order, local_action_checklist, parse_queue_action_options,
    parse_scoped_action_args, parse_scoped_list_args, prefix_session_note, required_arg,
    resolve_session_for_approval_action, resolve_session_for_inspection,
    resolve_session_for_optional_inspection, session_activity_json, session_inspect_metadata_json,
    short_id, write_command_output, SessionFallbackKind,
};
use crate::privacy::redact_sensitive_text;
use crate::schema_ids;
use crate::session::{ApprovalRequest, ApprovalStatus, Session, SessionStore};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn handle_approval(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let store = SessionStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let options = parse_scoped_list_args(&args[1..], current, "/approval list")?;
            let fallback = if options.include_all {
                SessionFallbackKind::ApprovalRequests
            } else {
                SessionFallbackKind::PendingApprovalRequests
            };
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                fallback,
            )?;
            let requests = session.load_approval_requests()?;
            let report = prefix_session_note(
                format_approval_requests(&requests, options.include_all),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_approval_list_json(
                    workspace,
                    &session,
                    note.as_deref(),
                    options.include_all,
                    &requests,
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
        Some("approve") => {
            let approval_id = required_arg(&args, 1, "approval request id")?;
            let options = parse_queue_action_options(
                &args[2..],
                "/approval approve <id> [--current] [--json] [--output path]",
            )?;
            let session = resolve_session_for_approval_action(
                &store,
                current.as_deref(),
                approval_id,
                options.current_only,
            )?;
            let item = session.approve_approval_request(approval_id)?;
            let outcome = if item.status == ApprovalStatus::Approved {
                "approved request"
            } else {
                "recorded confirmation for request"
            };
            let report = format!(
                "{outcome} {} in session {} {}",
                short_id(&item.id),
                session.id(),
                approval_request_metadata_text(&item)
            );
            let output = if options.json_output {
                format_approval_action_json(workspace, &session, "approve", &item, &report)?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("deny") => {
            let approval_id = required_arg(&args, 1, "approval request id")?;
            let options = parse_queue_action_options(
                &args[2..],
                "/approval deny <id> [--current] [--json] [--output path]",
            )?;
            let session = resolve_session_for_approval_action(
                &store,
                current.as_deref(),
                approval_id,
                options.current_only,
            )?;
            let item = session.update_approval_request(approval_id, ApprovalStatus::Denied)?;
            let report = format!(
                "denied request {} in session {} {}",
                short_id(&item.id),
                session.id(),
                approval_request_metadata_text(&item)
            );
            let output = if options.json_output {
                format_approval_action_json(workspace, &session, "deny", &item, &report)?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("clear") => {
            let options = parse_scoped_action_args(
                &args[1..],
                current,
                "/approval clear [--json] [--output path] [session_id|--current]",
            )?;
            let (session, _note) = resolve_session_for_inspection(
                &store,
                &options.session_id,
                options.explicit_session,
                SessionFallbackKind::PendingApprovalRequests,
            )?;
            let cleared = session.clear_pending_approval_requests()?;
            let report = format!(
                "cleared {cleared} pending approval request(s) in session {}",
                session.id()
            );
            let output = if options.json_output {
                format_approval_clear_json(workspace, &session, cleared, &report)?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(other) => bail!("unsupported /approval action `{other}`"),
    }
}

pub(super) fn format_approval_requests(items: &[ApprovalRequest], include_all: bool) -> String {
    let rows = items
        .iter()
        .filter(|item| include_all || item.status == ApprovalStatus::Pending)
        .map(|item| {
            format!(
                "{} [{}] {} risk={:?} outcome={:?} reason={}",
                short_id(&item.id),
                approval_status_label(&item.status),
                approval_request_metadata_text(item),
                item.decision.risk,
                item.decision.outcome,
                redact_sensitive_text(&item.decision.reason)
            )
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        "no approval requests".to_string()
    } else {
        rows.join("\n")
    }
}

fn approval_request_metadata_text(item: &ApprovalRequest) -> String {
    let digest = item.invocation_digest.as_deref().unwrap_or("unbound");
    let summary = item
        .input_summary
        .as_deref()
        .map(redact_sensitive_text)
        .unwrap_or_else(|| "-".to_string());
    let approved_at = item
        .approved_at
        .as_ref()
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| "-".to_string());
    let consumed_at = item
        .consumed_at
        .as_ref()
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| "-".to_string());
    format!(
        "tool={} digest={} summary={} confirmations={}/{} approved_at={} consumed_at={}",
        item.tool,
        digest,
        summary,
        item.confirmations_received,
        item.confirmations_required,
        approved_at,
        consumed_at
    )
}

fn format_approval_list_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    include_all: bool,
    requests: &[ApprovalRequest],
    report: &str,
) -> Result<String> {
    let items = requests
        .iter()
        .filter(|item| include_all || item.status == ApprovalStatus::Pending)
        .map(approval_request_json)
        .collect::<Vec<_>>();
    let activity = session.activity_summary()?;
    let next_actions = approval_list_next_actions(session, include_all, requests);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::APPROVAL_LIST_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "note": note,
        "includeAll": include_all,
        "session": session_inspect_metadata_json(session),
        "activity": session_activity_json(&activity),
        "itemCount": items.len(),
        "totalCount": requests.len(),
        "pendingCount": requests
            .iter()
            .filter(|item| item.status == ApprovalStatus::Pending)
            .count(),
        "approvals": items,
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
    }))?)
}

fn approval_list_next_actions(
    session: &Session,
    include_all: bool,
    requests: &[ApprovalRequest],
) -> Vec<String> {
    let session_id = session.id().to_string();
    let mut actions = Vec::new();
    if let Some(item) = requests
        .iter()
        .find(|item| item.status == ApprovalStatus::Pending)
    {
        let short = short_id(&item.id);
        actions.push(format!("deepcli approval approve {short}"));
        actions.push(format!("deepcli approval deny {short}"));
    }
    actions.push(format!("deepcli approval list {session_id} --json"));
    if !include_all {
        actions.push(format!("deepcli approval list {session_id} --all --json"));
    }
    actions.push("deepcli help approval".to_string());
    dedup_preserve_order(actions)
}

fn approval_action_next_actions(session: &Session) -> Vec<String> {
    let session_id = session.id().to_string();
    vec![
        format!("deepcli approval list {session_id} --json"),
        format!("deepcli approval list {session_id} --all --json"),
        "deepcli help approval".to_string(),
    ]
}

fn format_approval_action_json(
    workspace: &Path,
    session: &Session,
    action: &str,
    item: &ApprovalRequest,
    report: &str,
) -> Result<String> {
    let next_actions = approval_action_next_actions(session);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::APPROVAL_ACTION_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "action": action,
        "session": session_inspect_metadata_json(session),
        "approval": approval_request_json(item),
        "clearedCount": Value::Null,
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
    }))?)
}

fn format_approval_clear_json(
    workspace: &Path,
    session: &Session,
    cleared: usize,
    report: &str,
) -> Result<String> {
    let next_actions = approval_action_next_actions(session);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::APPROVAL_ACTION_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "action": "clear",
        "session": session_inspect_metadata_json(session),
        "approval": Value::Null,
        "clearedCount": cleared,
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
    }))?)
}

fn approval_request_json(item: &ApprovalRequest) -> Value {
    let input_summary = item.input_summary.as_deref().map(redact_sensitive_text);
    json!({
        "id": item.id.to_string(),
        "shortId": short_id(&item.id),
        "status": &item.status,
        "tool": item.tool.as_str(),
        "invocationDigest": &item.invocation_digest,
        "inputSummary": input_summary,
        "confirmationsRequired": item.confirmations_required,
        "confirmationsReceived": item.confirmations_received,
        "approvedAt": &item.approved_at,
        "consumedAt": &item.consumed_at,
        "decision": {
            "risk": &item.decision.risk,
            "outcome": &item.decision.outcome,
            "reason": redact_sensitive_text(&item.decision.reason),
        },
        "createdAt": &item.created_at,
        "updatedAt": &item.updated_at,
    })
}

fn approval_status_label(status: &ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Consumed => "consumed",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Cleared => "cleared",
    }
}
