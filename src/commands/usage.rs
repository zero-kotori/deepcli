use super::{
    compact_text_line, display_json_value, display_optional_u64, display_optional_usize,
    latest_session_with_recorded_activity, local_action_checklist, required_arg,
    session_has_no_recorded_activity, session_storage_bytes, set_command_output_path, short_id,
    status_u128_value, truncate_display, write_command_output,
};
use crate::schema_ids;
use crate::session::{AuditEvent, Session, SessionActivitySummary, SessionStore};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn handle_usage(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_usage_options(&args, current)?;
    let store = SessionStore::new(workspace);
    let usage = select_usage_report(workspace, &store, &options)?;
    let output = if options.json_output {
        format_usage_report_json(workspace, &usage)?
    } else {
        usage.report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, PartialEq, Eq)]
struct UsageOptions {
    session_id: Option<String>,
    explicit_session: bool,
    json_output: bool,
    output_path: Option<String>,
}

fn parse_usage_options(args: &[String], current: Option<String>) -> Result<UsageOptions> {
    let mut options = UsageOptions {
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
                    bail!("usage: /usage [--json] [--output path] [session_id|--current]");
                }
                options.session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /usage option `{value}`"),
            value if !value.trim().is_empty() => {
                if options.session_id.is_some() {
                    bail!("usage: /usage [--json] [--output path] [session_id|--current]");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
            _ => index += 1,
        }
    }
    if options.session_id.is_none() {
        options.session_id = current;
    }
    Ok(options)
}

struct UsageReport {
    report: String,
    session_source: &'static str,
    session: Option<UsageSessionReport>,
    note: Option<String>,
}

struct UsageSessionReport {
    session: Session,
    activity: SessionActivitySummary,
    audits: Vec<AuditEvent>,
    storage_bytes: u64,
    usage: UsageSummary,
}

fn select_usage_report(
    _workspace: &Path,
    store: &SessionStore,
    options: &UsageOptions,
) -> Result<UsageReport> {
    let Some(id) = options.session_id.as_deref() else {
        return if let Some((session, activity, audits)) =
            latest_session_with_recorded_activity(store, None)?
        {
            usage_report_for_session(
                session,
                activity,
                audits,
                "latest",
                Some("latest session with recorded usage/activity; no current session".to_string()),
            )
        } else {
            Ok(UsageReport {
                report: "no sessions with recorded usage/activity".to_string(),
                session_source: "none",
                session: None,
                note: Some("no sessions with recorded usage/activity".to_string()),
            })
        };
    };

    let session = store.load(id)?;
    let activity = session.activity_summary()?;
    let audits = session.load_audit_events()?;
    if !options.explicit_session && session_has_no_recorded_activity(&activity, &audits) {
        if let Some((fallback, fallback_activity, fallback_audits)) =
            latest_session_with_recorded_activity(store, Some(id))?
        {
            return usage_report_for_session(
                fallback,
                fallback_activity,
                fallback_audits,
                "latest",
                Some(format!(
                    "latest session with recorded usage/activity; current session {id} had none"
                )),
            );
        }
    }
    usage_report_for_session(
        session,
        activity,
        audits,
        if options.explicit_session {
            "explicit"
        } else {
            "current"
        },
        None,
    )
}

fn usage_report_for_session(
    session: Session,
    activity: SessionActivitySummary,
    audits: Vec<AuditEvent>,
    session_source: &'static str,
    note: Option<String>,
) -> Result<UsageReport> {
    let report = format_usage_report(&session, activity.clone(), audits.clone(), note.clone())?;
    let storage_bytes = session_storage_bytes(session.path())?;
    let usage = summarize_audit_usage(&audits);
    Ok(UsageReport {
        report,
        session_source,
        session: Some(UsageSessionReport {
            session,
            activity,
            audits,
            storage_bytes,
            usage,
        }),
        note,
    })
}

fn format_usage_report(
    session: &Session,
    activity: SessionActivitySummary,
    audits: Vec<AuditEvent>,
    session_note: Option<String>,
) -> Result<String> {
    let usage = summarize_audit_usage(&audits);
    let audit_count = audits.len();
    let storage_bytes = session_storage_bytes(session.path())?;
    let session_line = session_note
        .map(|note| format!("session: {} ({note})", session.id()))
        .unwrap_or_else(|| format!("session: {}", session.id()));

    let mut lines = vec![
        session_line,
        format!("state: {:?}", session.metadata.state),
        format!("storage: {} bytes", storage_bytes),
        format!(
            "activity: messages={} tools={} tests={} diffs={} backups={} approvals={} side_questions={} summary={}",
            activity.message_count,
            activity.tool_call_count,
            activity.test_run_count,
            activity.diff_count,
            activity.backup_count,
            activity.approval_request_count,
            activity.side_question_count,
            activity.has_summary
        ),
        format!("audit_events: {audit_count}"),
        format!(
            "provider turns: started={} completed={} total_elapsed_ms={}",
            usage.provider_turns_started, usage.provider_turns_completed, usage.provider_elapsed_ms
        ),
        format!(
            "tokens: prompt={} completion={} total={} cache_hit={} cache_miss={}",
            display_optional_u64(usage.prompt_tokens),
            display_optional_u64(usage.completion_tokens),
            display_optional_u64(usage.total_tokens),
            display_optional_u64(usage.prompt_cache_hit_tokens),
            display_optional_u64(usage.prompt_cache_miss_tokens)
        ),
        format!(
            "provider request: max_bytes={} latest_bytes={}",
            display_optional_usize(usage.max_request_bytes),
            display_optional_usize(usage.latest_request_bytes)
        ),
    ];
    lines.push(format_usage_diagnostics(&usage, &audits));

    Ok(lines.join("\n"))
}

fn format_usage_report_json(workspace: &Path, usage: &UsageReport) -> Result<String> {
    let session = usage
        .session
        .as_ref()
        .map(format_usage_session_json)
        .transpose()?
        .unwrap_or(Value::Null);
    let checklist = session
        .get("checklist")
        .cloned()
        .unwrap_or_else(|| json!([]));
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::USAGE_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "sessionSource": usage.session_source,
        "checklist": checklist,
        "session": session,
        "note": usage.note.as_deref(),
        "report": usage.report.as_str(),
    }))?)
}

fn format_usage_session_json(usage: &UsageSessionReport) -> Result<Value> {
    let summary_preview = usage.session.load_summary()?.and_then(|summary| {
        let trimmed = summary.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(truncate_display(trimmed, 1_000))
        }
    });
    let next_actions = vec![
        format!("deepcli trace --limit 20 {}", short_id(&usage.session.id())),
        format!("deepcli session diagnose {}", short_id(&usage.session.id())),
    ];
    Ok(json!({
        "id": usage.session.id().to_string(),
        "shortId": short_id(&usage.session.id()),
        "title": usage.session.metadata.title.as_deref(),
        "state": &usage.session.metadata.state,
        "provider": usage.session.metadata.provider.as_str(),
        "model": usage.session.metadata.model.as_deref(),
        "createdAt": &usage.session.metadata.created_at,
        "updatedAt": &usage.session.metadata.updated_at,
        "storageBytes": usage.storage_bytes,
        "activity": {
            "messages": usage.activity.message_count,
            "tools": usage.activity.tool_call_count,
            "tests": usage.activity.test_run_count,
            "diffs": usage.activity.diff_count,
            "backups": usage.activity.backup_count,
            "approvals": usage.activity.approval_request_count,
            "sideQuestions": usage.activity.side_question_count,
            "hasSummary": usage.activity.has_summary,
        },
        "auditEvents": usage.audits.len(),
        "providerTurns": {
            "started": usage.usage.provider_turns_started,
            "completed": usage.usage.provider_turns_completed,
            "elapsedMs": status_u128_value(usage.usage.provider_elapsed_ms),
            "maxElapsedMs": usage.usage.provider_max_elapsed_ms.map(status_u128_value),
            "averageElapsedMs": usage_average_elapsed_value(&usage.usage),
            "toolCalls": usage.usage.provider_tool_calls,
        },
        "tokens": {
            "prompt": usage.usage.prompt_tokens,
            "completion": usage.usage.completion_tokens,
            "total": usage.usage.total_tokens,
            "promptCacheHit": usage.usage.prompt_cache_hit_tokens,
            "promptCacheMiss": usage.usage.prompt_cache_miss_tokens,
            "cacheHitRate": cache_hit_rate(&usage.usage),
        },
        "request": {
            "maxBytes": usage.usage.max_request_bytes,
            "latestBytes": usage.usage.latest_request_bytes,
        },
        "context": {
            "compactedTurns": usage.usage.compacted_turns,
        },
        "diagnostics": usage_diagnostic_findings(&usage.usage, &usage.audits),
        "failedTools": count_failed_tool_events(&usage.audits),
        "failedTests": count_failed_test_events(&usage.audits),
        "summaryPreview": summary_preview,
        "checklist": local_action_checklist(&next_actions),
        "nextActions": next_actions,
    }))
}

fn usage_average_elapsed_value(summary: &UsageSummary) -> Value {
    if summary.provider_turns_completed == 0 {
        Value::Null
    } else {
        status_u128_value(summary.provider_elapsed_ms / summary.provider_turns_completed as u128)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct UsageSummary {
    pub(crate) provider_turns_started: usize,
    pub(crate) provider_turns_completed: usize,
    pub(crate) provider_elapsed_ms: u128,
    pub(crate) provider_max_elapsed_ms: Option<u128>,
    pub(crate) provider_tool_calls: usize,
    pub(crate) compacted_turns: usize,
    pub(crate) prompt_tokens: Option<u64>,
    pub(crate) completion_tokens: Option<u64>,
    pub(crate) total_tokens: Option<u64>,
    pub(crate) prompt_cache_hit_tokens: Option<u64>,
    pub(crate) prompt_cache_miss_tokens: Option<u64>,
    pub(crate) max_request_bytes: Option<usize>,
    pub(crate) latest_request_bytes: Option<usize>,
}

pub(crate) fn summarize_audit_usage(events: &[AuditEvent]) -> UsageSummary {
    let mut summary = UsageSummary::default();
    for event in events {
        match event.event_type.as_str() {
            "provider_turn_started" => {
                summary.provider_turns_started += 1;
                if event
                    .payload
                    .pointer("/request/compacted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    summary.compacted_turns += 1;
                }
                if let Some(bytes) = event
                    .payload
                    .pointer("/request/total_bytes")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                {
                    summary.latest_request_bytes = Some(bytes);
                    summary.max_request_bytes =
                        Some(summary.max_request_bytes.unwrap_or_default().max(bytes));
                }
            }
            "provider_turn_completed" => {
                summary.provider_turns_completed += 1;
                let elapsed = event
                    .payload
                    .get("elapsed_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from)
                    .unwrap_or_default();
                summary.provider_elapsed_ms += elapsed;
                summary.provider_max_elapsed_ms = Some(
                    summary
                        .provider_max_elapsed_ms
                        .unwrap_or_default()
                        .max(elapsed),
                );
                summary.provider_tool_calls += event
                    .payload
                    .get("tool_calls")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or_default();
                let usage = event.payload.get("usage").unwrap_or(&Value::Null);
                add_optional_u64(
                    &mut summary.prompt_tokens,
                    usage.get("prompt_tokens").and_then(Value::as_u64),
                );
                add_optional_u64(
                    &mut summary.completion_tokens,
                    usage.get("completion_tokens").and_then(Value::as_u64),
                );
                add_optional_u64(
                    &mut summary.total_tokens,
                    usage.get("total_tokens").and_then(Value::as_u64),
                );
                add_optional_u64(
                    &mut summary.prompt_cache_hit_tokens,
                    usage.get("prompt_cache_hit_tokens").and_then(Value::as_u64),
                );
                add_optional_u64(
                    &mut summary.prompt_cache_miss_tokens,
                    usage
                        .get("prompt_cache_miss_tokens")
                        .and_then(Value::as_u64),
                );
            }
            _ => {}
        }
    }
    summary
}

pub(crate) fn format_usage_diagnostics(summary: &UsageSummary, events: &[AuditEvent]) -> String {
    let mut lines = vec!["diagnostics:".to_string()];
    lines.extend(
        usage_diagnostic_findings(summary, events)
            .into_iter()
            .map(|finding| format!("  - {finding}")),
    );
    lines.join("\n")
}

fn usage_diagnostic_findings(summary: &UsageSummary, events: &[AuditEvent]) -> Vec<String> {
    let mut findings = Vec::new();

    if let Some(latest) = events.last() {
        findings.push(format!(
            "audit events recorded: {} latest={}; inspect `/trace --limit 20`",
            events.len(),
            latest.event_type
        ));
    }

    if summary.provider_turns_completed > 0 {
        let average = summary.provider_elapsed_ms / summary.provider_turns_completed as u128;
        findings.push(format!(
            "provider latency: avg={}ms max={}ms turns={}",
            average,
            summary.provider_max_elapsed_ms.unwrap_or_default(),
            summary.provider_turns_completed
        ));
        if average >= 30_000 {
            findings.push(
                "slow provider responses detected; run `/doctor --probe-provider` and inspect `/trace`"
                    .to_string(),
            );
        }
    } else if summary.provider_turns_started > 0 {
        findings.push("provider turns started but no completed response was recorded".to_string());
    } else {
        findings.push("no provider turns recorded for this session".to_string());
    }

    if let Some(max_bytes) = summary.max_request_bytes {
        let kib = max_bytes.div_ceil(1024);
        findings.push(format!(
            "largest provider request: {} KiB ({} bytes)",
            kib, max_bytes
        ));
        if max_bytes >= 512 * 1024 {
            findings.push(
                "large provider requests may slow responses; narrow file reads or use `/trace --limit 20`"
                    .to_string(),
            );
        }
    }

    if summary.compacted_turns > 0 {
        findings.push(format!(
            "context compaction happened on {} provider turn(s)",
            summary.compacted_turns
        ));
    }

    if summary.provider_tool_calls > 0 {
        findings.push(format!(
            "provider requested {} tool call(s)",
            summary.provider_tool_calls
        ));
    }

    if let Some(hit_rate) = cache_hit_rate(summary) {
        findings.push(format!("context cache hit rate: {hit_rate:.1}%"));
        if hit_rate < 50.0 {
            findings.push(
                "low cache hit rate; repeated large context changes may be increasing cost/latency"
                    .to_string(),
            );
        }
    }

    let probe_findings = provider_probe_findings(events);
    findings.extend(probe_findings);

    let failed_tools = count_failed_tool_events(events);
    if failed_tools > 0 {
        findings.push(format!(
            "tool failures recorded: {failed_tools}; inspect `/trace --limit 30`"
        ));
    }

    let failed_tests = count_failed_test_events(events);
    if failed_tests > 0 {
        findings.push(format!(
            "failed test runs recorded: {failed_tests}; run `/session tests`"
        ));
    }

    if findings.is_empty() {
        findings.push("no obvious latency or failure signal recorded".to_string());
    }

    findings
}

fn cache_hit_rate(summary: &UsageSummary) -> Option<f64> {
    let hit = summary.prompt_cache_hit_tokens?;
    let miss = summary.prompt_cache_miss_tokens.unwrap_or_default();
    let total = hit + miss;
    if total == 0 {
        None
    } else {
        Some(hit as f64 * 100.0 / total as f64)
    }
}

fn provider_probe_findings(events: &[AuditEvent]) -> Vec<String> {
    let mut findings = Vec::new();
    let probes = events
        .iter()
        .filter(|event| event.event_type == "provider_probe")
        .collect::<Vec<_>>();
    if probes.is_empty() {
        return findings;
    }

    let mut ok = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut timeout = 0usize;
    for event in &probes {
        match event
            .payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>")
        {
            "ok" => ok += 1,
            "skipped" => skipped += 1,
            "failed" => failed += 1,
            "timeout" => timeout += 1,
            _ => {}
        }
    }
    findings.push(format!(
        "provider probes: ok={ok} skipped={skipped} failed={failed} timeout={timeout}"
    ));
    if let Some(latest) = probes.last() {
        findings.push(format!(
            "latest provider probe: provider={} status={} message={}",
            display_json_value(latest.payload.get("provider")),
            display_json_value(latest.payload.get("status")),
            compact_text_line(
                latest
                    .payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>"),
                180
            )
        ));
    }
    if failed > 0 || timeout > 0 || skipped > 0 {
        findings.push("provider probe needs attention; run `/doctor --probe-provider` after fixing credentials/config".to_string());
    }
    findings
}

fn count_failed_tool_events(events: &[AuditEvent]) -> usize {
    events
        .iter()
        .filter(|event| {
            event.event_type == "tool_failed"
                || (event.event_type == "tool_call"
                    && matches!(
                        event.payload.get("status").and_then(Value::as_str),
                        Some("failed" | "denied")
                    ))
        })
        .count()
}

fn count_failed_test_events(events: &[AuditEvent]) -> usize {
    events
        .iter()
        .filter(|event| {
            event.event_type == "test_run"
                && event
                    .payload
                    .get("passed")
                    .and_then(Value::as_bool)
                    .is_some_and(|passed| !passed)
        })
        .count()
}

fn add_optional_u64(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or_default() + value);
    }
}
