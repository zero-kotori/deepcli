use super::{
    compact_json, compact_text_line, display_json_value, redact_sensitive_value, required_arg,
    set_command_output_path, short_id, write_command_output,
};
use crate::session::{AuditEvent, Session, SessionStore};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn handle_trace(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_trace_options(&args, current)?;
    let store = SessionStore::new(workspace);
    let trace = select_trace_report(&store, &options)?;
    let output = if options.json_output {
        format_trace_report_json(workspace, &trace)?
    } else {
        trace.report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, PartialEq, Eq)]
struct TraceOptions {
    limit: usize,
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_trace_options(args: &[String], current: Option<String>) -> Result<TraceOptions> {
    let mut options = TraceOptions {
        limit: 30,
        session_id: None,
        explicit_session: false,
        json_output: false,
        output_path: None,
    };
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
            value if index == 0 && value.parse::<usize>().is_ok() => {
                options.limit = value.parse::<usize>()?;
                index += 1;
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
            "--current" => {
                if options.session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /trace option `{value}`"),
            value => {
                if options.session_id.is_some() {
                    bail!("multiple session ids were provided");
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
    options.limit = options.limit.clamp(1, 100);
    Ok(options)
}

struct TraceReport {
    report: String,
    session_source: &'static str,
    session: Option<Session>,
    events: Vec<AuditEvent>,
    limit: usize,
    note: Option<String>,
}

fn select_trace_report(store: &SessionStore, options: &TraceOptions) -> Result<TraceReport> {
    let Some(id) = options.session_id.as_deref() else {
        return if let Some((session, events)) = latest_session_with_audit_events(store, None)? {
            Ok(trace_report_for_session(
                session,
                events,
                options.limit,
                "latest",
                Some("latest session with audit events; no current session".to_string()),
            ))
        } else {
            Ok(TraceReport {
                report: "no sessions with audit events".to_string(),
                session_source: "none",
                session: None,
                events: Vec::new(),
                limit: options.limit,
                note: Some("no sessions with audit events".to_string()),
            })
        };
    };

    let session = store.load(id)?;
    let events = session.load_audit_events()?;
    if events.is_empty() && !options.explicit_session {
        if let Some((fallback, fallback_events)) =
            latest_session_with_audit_events(store, Some(id))?
        {
            return Ok(trace_report_for_session(
                fallback,
                fallback_events,
                options.limit,
                "latest",
                Some(format!(
                    "latest session with audit events; current session {id} had none"
                )),
            ));
        }
    }
    Ok(trace_report_for_session(
        session,
        events,
        options.limit,
        if options.explicit_session {
            "explicit"
        } else {
            "current"
        },
        None,
    ))
}

fn trace_report_for_session(
    session: Session,
    events: Vec<AuditEvent>,
    limit: usize,
    session_source: &'static str,
    note: Option<String>,
) -> TraceReport {
    let trace = format_audit_trace(&events, limit);
    let report = if let Some(note) = &note {
        format!("session: {} ({note})\n{trace}", session.id())
    } else {
        format!("session: {}\n{trace}", session.id())
    };
    TraceReport {
        report,
        session_source,
        session: Some(session),
        events,
        limit,
        note,
    }
}

fn format_trace_report_json(workspace: &Path, trace: &TraceReport) -> Result<String> {
    let skip = trace.events.len().saturating_sub(trace.limit);
    let shown_events = trace
        .events
        .iter()
        .skip(skip)
        .map(format_trace_event_json)
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.trace.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "sessionSource": trace.session_source,
        "session": trace.session.as_ref().map(format_trace_session_json).unwrap_or(Value::Null),
        "limit": trace.limit,
        "totalEvents": trace.events.len(),
        "shownEvents": shown_events.len(),
        "note": trace.note.as_deref(),
        "events": shown_events,
        "report": trace.report.as_str(),
    }))?)
}

fn format_trace_session_json(session: &Session) -> Value {
    json!({
        "id": session.id().to_string(),
        "shortId": short_id(&session.id()),
        "title": session.metadata.title.as_deref(),
        "state": &session.metadata.state,
        "provider": session.metadata.provider.as_str(),
        "model": session.metadata.model.as_deref(),
        "createdAt": &session.metadata.created_at,
        "updatedAt": &session.metadata.updated_at,
    })
}

fn format_trace_event_json(event: &AuditEvent) -> Value {
    json!({
        "createdAt": &event.created_at,
        "eventType": event.event_type.as_str(),
        "line": format_trace_event(event),
        "payload": redact_sensitive_value(&event.payload),
    })
}

fn latest_session_with_audit_events(
    store: &SessionStore,
    skip_id: Option<&str>,
) -> Result<Option<(Session, Vec<AuditEvent>)>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        let events = session.load_audit_events()?;
        if !events.is_empty() {
            return Ok(Some((session, events)));
        }
    }
    Ok(None)
}

pub(crate) fn format_audit_trace(events: &[AuditEvent], limit: usize) -> String {
    if events.is_empty() {
        return format!("no audit events in the latest {limit} record(s)");
    }
    let skip = events.len().saturating_sub(limit);
    let shown = events.len() - skip;
    let mut lines = vec![format!(
        "showing latest {shown}/{} audit event(s)",
        events.len()
    )];
    lines.extend(events.iter().skip(skip).map(format_trace_event));
    lines.join("\n")
}

fn format_trace_event(event: &AuditEvent) -> String {
    let payload = &event.payload;
    match event.event_type.as_str() {
        "provider_turn_started" => format!(
            "{} provider_turn_started iteration={} timeout={}s messages={} tools={} request={} bytes compacted={}",
            event.created_at,
            display_json_value(payload.get("iteration")),
            display_json_value(payload.get("timeout_seconds")),
            display_json_value(payload.pointer("/request/message_count")),
            display_json_value(payload.pointer("/request/tool_count")),
            display_json_value(payload.pointer("/request/total_bytes")),
            display_json_value(payload.pointer("/request/compacted"))
        ),
        "provider_turn_completed" => format!(
            "{} provider_turn_completed elapsed={}ms tool_calls={} tokens={}",
            event.created_at,
            display_json_value(payload.get("elapsed_ms")),
            display_json_value(payload.get("tool_calls")),
            display_json_value(payload.pointer("/usage/total_tokens"))
        ),
        "provider_probe" => format!(
            "{} provider_probe provider={} status={} elapsed={}ms message={}{}",
            event.created_at,
            display_json_value(payload.get("provider")),
            display_json_value(payload.get("status")),
            display_json_value(payload.get("elapsed_ms")),
            compact_text_line(
                payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>"),
                200
            ),
            payload
                .get("content_preview")
                .and_then(Value::as_str)
                .map(|content| format!(" content={}", compact_text_line(content, 120)))
                .unwrap_or_default()
        ),
        "tool_call" => format!(
            "{} tool_call tool={} status={} risk={} outcome={}",
            event.created_at,
            display_json_value(payload.get("tool")),
            display_json_value(payload.get("status")),
            display_json_value(payload.pointer("/decision/risk")),
            display_json_value(payload.pointer("/decision/outcome"))
        ),
        "test_run" => format!(
            "{} test_run passed={} exit={} command={}",
            event.created_at,
            display_json_value(payload.get("passed")),
            display_json_value(payload.get("exit_code")),
            compact_text_line(
                payload
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>"),
                200
            )
        ),
        "approval_requested" => format!(
            "{} approval_requested tool={} risk={} reason={}",
            event.created_at,
            display_json_value(payload.get("tool")),
            display_json_value(payload.pointer("/decision/risk")),
            compact_text_line(
                payload
                    .pointer("/decision/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>"),
                200
            )
        ),
        "approval_updated" => format!(
            "{} approval_updated id={} status={} tool={}",
            event.created_at,
            display_json_value(payload.get("id")),
            display_json_value(payload.get("status")),
            display_json_value(payload.get("tool"))
        ),
        "model_updated" => format!(
            "{} model_updated provider={} model={}",
            event.created_at,
            display_json_value(payload.get("provider")),
            display_json_value(payload.get("model"))
        ),
        "credentials_updated" => format!(
            "{} credentials_updated provider={} source={} apiKey=<redacted>",
            event.created_at,
            display_json_value(payload.get("provider")),
            display_json_value(payload.get("source"))
        ),
        other => format!(
            "{} {} {}",
            event.created_at,
            other,
            compact_json(payload, 500)
        ),
    }
}
