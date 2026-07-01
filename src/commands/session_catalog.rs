use super::*;
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SessionListOptions {
    include_all: bool,
    limit: Option<usize>,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SessionPruneEmptyOptions {
    force: bool,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionPruneEmptyReport {
    force: bool,
    deleted: bool,
    candidates: Vec<SessionMetadata>,
    skipped_current: Option<SessionMetadata>,
    skipped_titled: Vec<SessionMetadata>,
    report: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionSearchOptions {
    query: String,
    limit: usize,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionSearchHit {
    metadata: SessionMetadata,
    matches: Vec<String>,
}

#[derive(Debug, Clone)]
struct SessionSearchReport {
    query: String,
    limit: usize,
    hits: Vec<SessionSearchHit>,
}

#[derive(Debug, Clone)]
struct SessionListReport {
    options: SessionListOptions,
    sessions: Vec<SessionMetadata>,
    total_sessions: usize,
    hidden_empty: usize,
}

pub(crate) fn handle_session_default_list(store: &SessionStore) -> Result<String> {
    let options = SessionListOptions::default();
    let report = collect_session_list_report(store, options)?;
    Ok(format_limited_session_list(
        &report.sessions,
        report.options.limit,
        report.hidden_empty,
    ))
}

pub(crate) fn handle_session_list(
    workspace: &Path,
    store: &SessionStore,
    args: &[String],
) -> Result<String> {
    let options = parse_session_list_args(args)?;
    let report = collect_session_list_report(store, options)?;
    let text =
        format_limited_session_list(&report.sessions, report.options.limit, report.hidden_empty);
    let output = if report.options.json_output {
        format_session_list_json(workspace, store, &report, &text)?
    } else {
        text
    };
    if let Some(output_path) = &report.options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn handle_session_search(
    workspace: &Path,
    store: &SessionStore,
    args: &[String],
) -> Result<String> {
    let options = parse_session_search_args(args)?;
    let report = collect_session_search_report(store, &options.query, options.limit)?;
    let text = format_session_search_report(&report);
    let output = if options.json_output {
        format_session_search_json(workspace, &report, &text)?
    } else {
        text
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(crate) fn handle_session_prune_empty(
    workspace: &Path,
    store: &SessionStore,
    current: Option<&str>,
    args: &[String],
) -> Result<String> {
    let options = parse_session_prune_empty_args(args)?;
    let report = prune_empty_sessions(store, current, options.force)?;
    let output = if options.json_output {
        format_session_prune_empty_json(workspace, &report)?
    } else {
        report.report.clone()
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

fn parse_session_list_args(args: &[String]) -> Result<SessionListOptions> {
    let mut options = SessionListOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--all" => {
                options.include_all = true;
                index += 1;
            }
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                options.limit = Some(parse_positive_usize(raw, "limit")?.clamp(1, 100));
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
            other => bail!("unsupported /session list option `{other}`"),
        }
    }
    Ok(options)
}

fn parse_session_search_args(args: &[String]) -> Result<SessionSearchOptions> {
    let mut query_parts = Vec::new();
    let mut limit = 10usize;
    let mut json_output = false;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = parse_positive_usize(raw, "limit")?.clamp(1, 50);
                index += 2;
            }
            "--json" => {
                json_output = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(&mut output_path, value.trim_start_matches("--output="))?;
                index += 1;
            }
            value if value.starts_with('-') => {
                bail!("unsupported /session search option `{value}`")
            }
            value => {
                query_parts.push(value.to_string());
                index += 1;
            }
        }
    }
    let query = query_parts.join(" ").trim().to_string();
    if query.is_empty() {
        bail!("/session search requires a query");
    }
    Ok(SessionSearchOptions {
        query,
        limit,
        json_output,
        output_path,
    })
}

fn parse_session_prune_empty_args(args: &[String]) -> Result<SessionPruneEmptyOptions> {
    let mut options = SessionPruneEmptyOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => {
                index += 1;
            }
            "--force" => {
                options.force = true;
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
            other => bail!("unsupported /session prune-empty option `{other}`"),
        }
    }
    Ok(options)
}

fn prune_empty_sessions(
    store: &SessionStore,
    current: Option<&str>,
    force: bool,
) -> Result<SessionPruneEmptyReport> {
    let current = current.map(|id| store.resolve_id(id)).transpose()?;
    let mut empty_sessions = Vec::new();
    let mut skipped_current = None;
    let mut skipped_titled = Vec::new();
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        let session = store.load(&id)?;
        if session_has_recorded_activity(&session)? {
            continue;
        }
        if current.as_deref() == Some(id.as_str()) {
            skipped_current = Some(metadata);
        } else if metadata
            .title
            .as_deref()
            .is_some_and(|title| !title.trim().is_empty())
        {
            skipped_titled.push(metadata);
        } else {
            empty_sessions.push(metadata);
        }
    }

    let action = if force { "deleted" } else { "would delete" };
    let mut lines = vec![format!("{action} empty sessions: {}", empty_sessions.len())];
    if !force {
        lines.push("dry-run: pass `--force` to delete these empty session directories".to_string());
    }
    if let Some(metadata) = &skipped_current {
        lines.push(format!(
            "skipped current empty session: id={} full={}",
            short_id(&metadata.id),
            metadata.id
        ));
    }
    if !skipped_titled.is_empty() {
        lines.push(format!(
            "skipped titled empty sessions: {}",
            skipped_titled.len()
        ));
        for metadata in &skipped_titled {
            lines.push(format!(
                "  - id={} full={} title={}",
                short_id(&metadata.id),
                metadata.id,
                metadata
                    .title
                    .as_deref()
                    .map(redact_sensitive_text)
                    .unwrap_or_else(|| "<untitled>".to_string())
            ));
        }
    }
    for metadata in &empty_sessions {
        lines.push(format!(
            "  - id={} full={} created={} updated={} provider={} model={}",
            short_id(&metadata.id),
            metadata.id,
            metadata.created_at,
            metadata.updated_at,
            metadata.provider,
            metadata.model.as_deref().unwrap_or("<unset>")
        ));
    }

    if force {
        for metadata in &empty_sessions {
            let session = store.load(&metadata.id.to_string())?;
            fs::remove_dir_all(session.path()).with_context(|| {
                format!("failed to remove session {}", session.path().display())
            })?;
        }
    }

    Ok(SessionPruneEmptyReport {
        force,
        deleted: force,
        candidates: empty_sessions,
        skipped_current,
        skipped_titled,
        report: lines.join("\n"),
    })
}

fn format_session_prune_empty_json(
    workspace: &Path,
    report: &SessionPruneEmptyReport,
) -> Result<String> {
    let next_actions = session_prune_empty_next_actions(report);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_PRUNE_EMPTY_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "dryRun": !report.force,
        "force": report.force,
        "deleted": report.deleted,
        "candidateCount": report.candidates.len(),
        "deletedCount": if report.deleted { report.candidates.len() } else { 0 },
        "skippedCurrent": report.skipped_current.as_ref().map(session_metadata_json).unwrap_or(Value::Null),
        "skippedTitledCount": report.skipped_titled.len(),
        "candidates": report.candidates.iter().map(session_metadata_json).collect::<Vec<_>>(),
        "skippedTitled": report.skipped_titled.iter().map(session_metadata_json).collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": local_action_checklist(&next_actions),
        "report": report.report,
    }))?)
}

fn session_prune_empty_next_actions(report: &SessionPruneEmptyReport) -> Vec<String> {
    if !report.force && !report.candidates.is_empty() {
        vec![
            "deepcli session prune-empty --force --json".to_string(),
            "deepcli session list --all --json".to_string(),
            "deepcli history --limit 20".to_string(),
        ]
    } else {
        vec![
            "deepcli session list --json".to_string(),
            "deepcli history --limit 20".to_string(),
        ]
    }
}

fn collect_session_search_report(
    store: &SessionStore,
    query: &str,
    limit: usize,
) -> Result<SessionSearchReport> {
    let query_lower = query.to_lowercase();
    let mut hits = Vec::new();
    for metadata in store.list()? {
        let session = store.load(&metadata.id.to_string())?;
        let matches = session_search_matches(&session, &query_lower)?;
        if !matches.is_empty() {
            hits.push(SessionSearchHit { metadata, matches });
        }
        if hits.len() >= limit {
            break;
        }
    }
    Ok(SessionSearchReport {
        query: query.to_string(),
        limit,
        hits,
    })
}

fn format_session_search_report(report: &SessionSearchReport) -> String {
    let hits = &report.hits;
    if hits.is_empty() {
        return format!("no sessions matched `{}`", report.query);
    }
    hits.iter()
        .map(format_session_search_hit)
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_session_search_json(
    workspace: &Path,
    report: &SessionSearchReport,
    text: &str,
) -> Result<String> {
    let next_actions = session_search_next_actions(report);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_SEARCH_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "query": redact_sensitive_text(&report.query),
        "limit": report.limit,
        "hitCount": report.hits.len(),
        "hits": report.hits.iter().map(session_search_hit_json).collect::<Vec<_>>(),
        "nextActions": next_actions,
        "checklist": local_action_checklist(&next_actions),
        "report": text,
    }))?)
}

fn session_search_next_actions(report: &SessionSearchReport) -> Vec<String> {
    if let Some(hit) = report.hits.first() {
        let short = short_id(&hit.metadata.id);
        vec![
            format!("deepcli resume {short} --dry-run --json"),
            format!("deepcli session history {short} --limit 20"),
            format!("deepcli session next {short} --json"),
            format!("deepcli session diagnose {short} --json"),
        ]
    } else {
        vec![
            "deepcli sessions --all --limit 20".to_string(),
            "deepcli resume --dry-run --json".to_string(),
            "deepcli session list --json".to_string(),
        ]
    }
}

fn session_search_hit_json(hit: &SessionSearchHit) -> Value {
    json!({
        "session": session_metadata_json(&hit.metadata),
        "matches": hit
            .matches
            .iter()
            .map(|item| redact_sensitive_text(item))
            .collect::<Vec<_>>(),
    })
}

fn session_search_matches(session: &Session, query_lower: &str) -> Result<Vec<String>> {
    let mut matches = Vec::new();
    if session
        .metadata
        .title
        .as_deref()
        .is_some_and(|title| text_matches_query(title, query_lower))
    {
        matches.push(format!(
            "title: {}",
            redact_sensitive_text(session.metadata.title.as_deref().unwrap_or_default())
        ));
    }
    if text_matches_query(&session.metadata.provider, query_lower) {
        matches.push(format!("provider: {}", session.metadata.provider));
    }
    if session
        .metadata
        .model
        .as_deref()
        .is_some_and(|model| text_matches_query(model, query_lower))
    {
        matches.push(format!(
            "model: {}",
            redact_sensitive_text(session.metadata.model.as_deref().unwrap_or_default())
        ));
    }
    if let Some(summary) = session.load_summary()? {
        if text_matches_query(&summary, query_lower) {
            matches.push(format!(
                "summary: {}",
                compact_text_line(&redact_sensitive_text(&summary), 180)
            ));
        }
    }
    for message in session.load_recent_messages(20)? {
        if text_matches_query(&message.content, query_lower) {
            matches.push(format!(
                "message/{}: {}",
                message.role,
                compact_text_line(&redact_sensitive_text(&message.content), 180)
            ));
            break;
        }
    }
    for record in session.load_recent_tool_calls(20)? {
        let haystack = format!(
            "{} {} {}",
            record.tool,
            compact_json(&redact_sensitive_value(&record.input), 1_000),
            compact_json(&redact_sensitive_value(&record.output), 1_000)
        );
        if text_matches_query(&haystack, query_lower) {
            matches.push(format!("tool: {}", record.tool));
            break;
        }
    }
    for record in session.load_recent_test_runs(20)? {
        let haystack = format!("{} {} {}", record.command, record.stdout, record.stderr);
        if text_matches_query(&haystack, query_lower) {
            matches.push(format!(
                "test: {}",
                compact_text_line(&redact_sensitive_text(&record.command), 180)
            ));
            break;
        }
    }
    for record in session.load_recent_diffs(20)? {
        if text_matches_query(&record.name, query_lower)
            || text_matches_query(&record.content, query_lower)
        {
            matches.push(format!("diff: {}", redact_sensitive_text(&record.name)));
            break;
        }
    }
    for record in session.load_recent_backups(20)? {
        let target = record
            .target_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        if text_matches_query(&record.name, query_lower)
            || text_matches_query(&target, query_lower)
            || text_matches_query(&record.content, query_lower)
        {
            matches.push(format!("backup: {}", redact_sensitive_text(&record.name)));
            break;
        }
    }
    Ok(matches.into_iter().take(5).collect())
}

fn text_matches_query(value: &str, query_lower: &str) -> bool {
    value.to_lowercase().contains(query_lower)
}

fn format_session_search_hit(hit: &SessionSearchHit) -> String {
    let title = hit
        .metadata
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .map(redact_sensitive_text)
        .unwrap_or_else(|| "untitled".to_string());
    let model = hit
        .metadata
        .model
        .as_deref()
        .map(redact_sensitive_text)
        .unwrap_or_else(|| "-".to_string());
    let mut line = format!(
        "id={} full={} [{:?}] provider={} model={} updated={} title={}",
        short_id(&hit.metadata.id),
        hit.metadata.id,
        hit.metadata.state,
        hit.metadata.provider,
        model,
        hit.metadata.updated_at,
        title
    );
    for item in &hit.matches {
        line.push_str(&format!("\n  - {}", redact_sensitive_text(item)));
    }
    line
}

fn collect_session_list_report(
    store: &SessionStore,
    options: SessionListOptions,
) -> Result<SessionListReport> {
    let all = store.list()?;
    let (sessions, hidden_empty) = if options.include_all {
        (all.clone(), 0)
    } else {
        let sessions = filter_session_metadata_with_activity(store, &all)?;
        let hidden_empty = all.len().saturating_sub(sessions.len());
        (sessions, hidden_empty)
    };
    Ok(SessionListReport {
        options,
        sessions,
        total_sessions: all.len(),
        hidden_empty,
    })
}

fn format_session_list_json(
    workspace: &Path,
    store: &SessionStore,
    report: &SessionListReport,
    text: &str,
) -> Result<String> {
    let shown = report.options.limit.map_or(report.sessions.len(), |limit| {
        report.sessions.len().min(limit)
    });
    let sessions = report
        .sessions
        .iter()
        .take(shown)
        .map(|metadata| session_list_item_json(store, metadata))
        .collect::<Result<Vec<_>>>()?;
    let next_actions = session_list_next_actions(report);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SESSION_LIST_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "includeAll": report.options.include_all,
        "limit": report.options.limit,
        "totalSessions": report.total_sessions,
        "matchingSessions": report.sessions.len(),
        "shownSessions": sessions.len(),
        "hiddenEmptySessions": report.hidden_empty,
        "sessions": sessions,
        "nextActions": next_actions,
        "checklist": checklist,
        "report": text,
    }))?)
}

fn session_list_next_actions(report: &SessionListReport) -> Vec<String> {
    let shown = report.options.limit.map_or(report.sessions.len(), |limit| {
        report.sessions.len().min(limit)
    });
    let mut actions = Vec::new();
    if let Some(metadata) = report.sessions.iter().take(shown).next() {
        let short = short_id(&metadata.id);
        push_unique_action(
            &mut actions,
            format!("deepcli resume {short} --dry-run --json"),
        );
        push_unique_action(
            &mut actions,
            format!("deepcli session history {short} --limit 20 --json"),
        );
        push_unique_action(&mut actions, format!("deepcli session next {short} --json"));
        push_unique_action(
            &mut actions,
            format!("deepcli session diagnose {short} --json"),
        );
    } else {
        push_unique_action(&mut actions, "deepcli resume --dry-run --json".to_string());
    }
    if !report.options.include_all && report.hidden_empty > 0 {
        push_unique_action(
            &mut actions,
            "deepcli session list --all --limit 20 --json".to_string(),
        );
    }
    push_unique_action(
        &mut actions,
        "deepcli session prune-empty --dry-run --json".to_string(),
    );
    push_unique_action(&mut actions, "deepcli help session".to_string());
    actions
}

fn session_list_item_json(store: &SessionStore, metadata: &SessionMetadata) -> Result<Value> {
    let session = store.load(&metadata.id.to_string())?;
    Ok(json!({
        "metadata": session_metadata_json(metadata),
        "activity": session_activity_json(&session.activity_summary()?),
        "hasRecordedActivity": session_has_recorded_activity(&session)?,
        "hasNextActionSignals": session_has_next_action_signals(&session)?,
    }))
}

fn filter_session_metadata_with_activity(
    store: &SessionStore,
    sessions: &[SessionMetadata],
) -> Result<Vec<SessionMetadata>> {
    let mut filtered = Vec::new();
    for metadata in sessions {
        let session = store.load(&metadata.id.to_string())?;
        if session_has_recorded_activity(&session)? {
            filtered.push(metadata.clone());
        }
    }
    Ok(filtered)
}

fn format_limited_session_list(
    sessions: &[SessionMetadata],
    limit: Option<usize>,
    hidden_empty: usize,
) -> String {
    let shown = limit.map_or(sessions.len(), |limit| sessions.len().min(limit));
    let visible = &sessions[..shown];
    if visible.is_empty() {
        return if hidden_empty == 0 {
            "no sessions".to_string()
        } else {
            format!(
                "no sessions with activity\nhidden empty sessions: {hidden_empty}; run `/session list --all` to show them"
            )
        };
    }
    let mut output = format_session_list(visible);
    if shown < sessions.len() {
        output.push_str(&format!(
            "\nshowing {shown}/{} sessions; omit `--limit` to show all",
            sessions.len()
        ));
    }
    if hidden_empty > 0 {
        output.push_str(&format!(
            "\nhidden empty sessions: {hidden_empty}; run `/session list --all` to show them"
        ));
    }
    output
}
