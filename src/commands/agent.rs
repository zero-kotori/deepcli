use super::{
    dedup_preserve_order, local_action_checklist, required_arg, set_command_output_path, short_id,
    write_command_output,
};
use crate::agents::{AgentStore, SubagentEvent, SubagentStatus, SubagentTask};
use crate::config::AppConfig;
use crate::runtime::{AgentRuntime, RuntimeOptions};
use crate::schema_ids;
use crate::tools::ToolExecutor;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

#[cfg(test)]
pub(crate) async fn handle_agent(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    handle_agent_inner(workspace, None, None, executor, args).await
}

pub(crate) async fn handle_agent_with_config(
    workspace: &Path,
    config: &AppConfig,
    provider_override: Option<&str>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    handle_agent_inner(workspace, Some(config), provider_override, executor, args).await
}

async fn handle_agent_inner(
    workspace: &Path,
    config: Option<&AppConfig>,
    provider_override: Option<&str>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let store = AgentStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let option_args = if args.first().map(String::as_str) == Some("list") {
                &args[1..]
            } else {
                args.as_slice()
            };
            let options = parse_agent_read_options(option_args, "/agent list")?;
            let tasks = store.list()?;
            let text = serde_json::to_string_pretty(&tasks)?;
            let output = if options.json_output {
                format_agent_list_json(workspace, &tasks, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_agent_read_options(&args, "/agent list")?;
            let tasks = store.list()?;
            let text = serde_json::to_string_pretty(&tasks)?;
            let output = if options.json_output {
                format_agent_list_json(workspace, &tasks, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("show") => {
            let id = required_arg(&args, 1, "sub-agent id")?;
            let options = parse_agent_read_options(&args[2..], "/agent show")?;
            let task = select_subagent_task(&store, id)?;
            let text = serde_json::to_string_pretty(&task)?;
            let output = if options.json_output {
                format_agent_show_json(workspace, &task, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("logs") => {
            let id = required_arg(&args, 1, "sub-agent id")?;
            let options = parse_agent_read_options(&args[2..], "/agent logs")?;
            let task = select_subagent_task(&store, id)?;
            let events = store.read_subagent_events(task.id)?;
            let text = format_agent_logs_text(&events);
            let output = if options.json_output {
                format_agent_logs_json(workspace, &store, &task, &events, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("resume") => {
            let action = "resume";
            let id = required_arg(&args, 1, "sub-agent id")?;
            let options = parse_agent_resume_options(&args[2..], &format!("/agent {action}"))?;
            let config = config
                .ok_or_else(|| anyhow::anyhow!("/agent {action} requires command context"))?;
            let task = select_subagent_task(&store, id)?;
            let output = resume_subagent_command(
                workspace,
                config,
                provider_override,
                &store,
                task,
                action,
                &options,
            )
            .await?;
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("spawn") => {
            let task = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if task.trim().is_empty() {
                bail!("/agent spawn requires a task");
            }
            Ok(executor
                .execute("spawn_subagent", json!({"task": task}))
                .await?
                .content)
        }
        Some(other) => bail!("unsupported /agent action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AgentReadOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn parse_agent_read_options(args: &[String], command: &str) -> Result<AgentReadOptions> {
    let mut options = AgentReadOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("{command} --output requires a path"))?;
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
            value => bail!("unsupported {command} option `{value}`"),
        }
    }
    Ok(options)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AgentResumeOptions {
    json_output: bool,
    output_path: Option<String>,
    background_child: bool,
}

fn parse_agent_resume_options(args: &[String], command: &str) -> Result<AgentResumeOptions> {
    let mut options = AgentResumeOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--background-child" => {
                options.background_child = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("{command} --output requires a path"))?;
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
            value => bail!("unsupported {command} option `{value}`"),
        }
    }
    Ok(options)
}

fn select_subagent_task(store: &AgentStore, selector: &str) -> Result<SubagentTask> {
    if let Ok(id) = uuid::Uuid::parse_str(selector) {
        return store.load(id);
    }
    let matches = store
        .list()?
        .into_iter()
        .filter(|task| task.id.to_string().starts_with(selector))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => bail!("sub-agent `{selector}` was not found"),
        [task] => Ok(task.clone()),
        _ => bail!("sub-agent id prefix `{selector}` is ambiguous; use the full id"),
    }
}

async fn resume_subagent_command(
    workspace: &Path,
    config: &AppConfig,
    provider_override: Option<&str>,
    store: &AgentStore,
    task: SubagentTask,
    action: &str,
    options: &AgentResumeOptions,
) -> Result<String> {
    if task.status == SubagentStatus::Completed {
        bail!("sub-agent {} is already completed", short_id(&task.id));
    }
    let resume_session = task.child_session_id.map(|id| id.to_string());
    let mut runtime = match AgentRuntime::new(
        config.clone(),
        RuntimeOptions {
            workspace: workspace.to_path_buf(),
            provider: provider_override.map(str::to_string),
            model: None,
            assume_yes: false,
            resume_session,
            stream_output: false,
        },
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            let failed = store.fail_subagent(task.id, &error.to_string())?;
            return format_agent_resume_output(
                workspace,
                store,
                &failed,
                action,
                options,
                None,
                Some(&error.to_string()),
            );
        }
    };
    if let Err(error) = runtime.restrict_to_subagent(&task) {
        let failed = store.fail_subagent(task.id, &error.to_string())?;
        return format_agent_resume_output(
            workspace,
            store,
            &failed,
            action,
            options,
            None,
            Some(&error.to_string()),
        );
    }
    let child_session_id = uuid::Uuid::parse_str(&runtime.session_id())
        .with_context(|| "child runtime returned an invalid session id")?;
    let pid = Some(std::process::id());
    let started = store.mark_subagent_started(task.id, Some(child_session_id), pid)?;
    store.heartbeat_subagent(task.id)?;
    let prompt = subagent_runtime_prompt(&started);
    let result = Box::pin(runtime.handle_input(&prompt)).await;
    match result {
        Ok(output) => {
            append_subagent_output(store, task.id, &output)?;
            let terminal_task = if runtime.state_label() == "AwaitingApproval" {
                store.await_subagent_approval(task.id, first_line_for_summary(&output))?
            } else {
                store.complete_subagent(task.id, first_line_for_summary(&output))?
            };
            format_agent_resume_output(
                workspace,
                store,
                &terminal_task,
                action,
                options,
                Some(&output),
                None,
            )
        }
        Err(error) => {
            append_subagent_output(store, task.id, &format!("error: {error:#}"))?;
            let failed = store.fail_subagent(task.id, &error.to_string())?;
            format_agent_resume_output(
                workspace,
                store,
                &failed,
                action,
                options,
                None,
                Some(&error.to_string()),
            )
        }
    }
}

fn subagent_runtime_prompt(task: &SubagentTask) -> String {
    let mut lines = vec![
        "You are a deepcli background sub-agent.".to_string(),
        format!("Task: {}", task.task),
        format!("Depth: {}", task.depth),
    ];
    if !task.read_scope.is_empty() {
        lines.push(format!(
            "Read scope: {}",
            task.read_scope
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !task.write_scope.is_empty() {
        lines.push(format!(
            "Write scope: {}",
            task.write_scope
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !task.allowed_tools.is_empty() {
        lines.push(format!("Allowed tools: {}", task.allowed_tools.join(", ")));
    }
    if let Some(context) = task.context.as_deref() {
        lines.push(format!("Context: {context}"));
    }
    lines.push(
        "Work only on this delegated task and return a concise result with changes, tests, and blockers."
            .to_string(),
    );
    lines.join("\n")
}

fn append_subagent_output(store: &AgentStore, id: uuid::Uuid, output: &str) -> Result<()> {
    let path = store.subagent_output_log_path(id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to write {}", path.display()))?;
    writeln!(file, "{output}")?;
    Ok(())
}

fn first_line_for_summary(output: &str) -> &str {
    output.lines().next().unwrap_or("").trim()
}

fn format_agent_resume_output(
    workspace: &Path,
    store: &AgentStore,
    task: &SubagentTask,
    action: &str,
    options: &AgentResumeOptions,
    output: Option<&str>,
    error: Option<&str>,
) -> Result<String> {
    let events = store.read_subagent_events(task.id)?;
    let status = subagent_status_label(&task.status);
    let report = match (output, error) {
        (Some(output), _) if !output.trim().is_empty() => output.to_string(),
        (_, Some(error)) => format!("sub-agent {status}: {error}"),
        _ => format!("sub-agent {status}: {}", task.task),
    };
    if !options.json_output {
        return Ok(report);
    }
    let next_actions = agent_next_actions(Some(task), false);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::AGENT_INSPECT_V1,
        "status": status,
        "workspace": workspace.display().to_string(),
        "kind": action,
        "agent": subagent_task_json(workspace, task),
        "output": output,
        "error": error,
        "backgroundChild": options.background_child,
        "eventCount": events.len(),
        "events": subagent_events_json(&events),
        "eventLogPath": store.subagent_event_log_path(task.id).display().to_string(),
        "outputLogPath": store.subagent_output_log_path(task.id).display().to_string(),
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
        "format": "json",
    }))?)
}

fn format_agent_logs_text(events: &[SubagentEvent]) -> String {
    if events.is_empty() {
        return "no sub-agent events".to_string();
    }
    events
        .iter()
        .map(|event| {
            let message = event
                .message
                .as_deref()
                .map(|message| format!(" {message}"))
                .unwrap_or_default();
            format!(
                "{} {} {}{}",
                event.timestamp.to_rfc3339(),
                subagent_status_label(&event.status),
                event.event_type,
                message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_agent_logs_json(
    workspace: &Path,
    store: &AgentStore,
    task: &SubagentTask,
    events: &[SubagentEvent],
    report: &str,
) -> Result<String> {
    let next_actions = agent_next_actions(Some(task), false);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::AGENT_INSPECT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "logs",
        "agent": subagent_task_json(workspace, task),
        "eventCount": events.len(),
        "events": subagent_events_json(events),
        "eventLogPath": store.subagent_event_log_path(task.id).display().to_string(),
        "outputLogPath": store.subagent_output_log_path(task.id).display().to_string(),
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
        "format": "json",
    }))?)
}

fn subagent_events_json(events: &[SubagentEvent]) -> Vec<Value> {
    events
        .iter()
        .map(|event| {
            json!({
                "timestamp": event.timestamp.to_rfc3339(),
                "taskId": event.task_id.to_string(),
                "type": event.event_type,
                "status": subagent_status_label(&event.status),
                "childSessionId": event.child_session_id.map(|id| id.to_string()),
                "pid": event.pid,
                "message": event.message,
            })
        })
        .collect()
}

fn format_agent_list_json(
    workspace: &Path,
    tasks: &[SubagentTask],
    report: &str,
) -> Result<String> {
    let next_actions = agent_list_next_actions(tasks);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::AGENT_INSPECT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "list",
        "agentCount": tasks.len(),
        "agents": tasks
            .iter()
            .map(|task| subagent_task_json(workspace, task))
            .collect::<Vec<_>>(),
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
        "format": "json",
    }))?)
}

fn format_agent_show_json(workspace: &Path, task: &SubagentTask, report: &str) -> Result<String> {
    let next_actions = agent_next_actions(Some(task), false);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::AGENT_INSPECT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "show",
        "agent": subagent_task_json(workspace, task),
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
        "format": "json",
    }))?)
}

fn subagent_task_json(workspace: &Path, task: &SubagentTask) -> Value {
    let id = task.id.to_string();
    json!({
        "id": id,
        "shortId": short_id(&task.id),
        "parentSessionId": task.parent_session_id.map(|id| id.to_string()),
        "childSessionId": task.child_session_id.map(|id| id.to_string()),
        "task": task.task.as_str(),
        "depth": task.depth,
        "readScope": task
            .read_scope
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
        "writeScope": task
            .write_scope
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
        "allowedTools": task.allowed_tools.clone(),
        "context": task.context.clone(),
        "status": subagent_status_label(&task.status),
        "attempts": task.attempts,
        "pid": task.pid,
        "eventLogPath": task.event_log_path.as_ref().map(|path| path.display().to_string()),
        "outputLogPath": task.output_log_path.as_ref().map(|path| path.display().to_string()),
        "startedAt": task.started_at.map(|time| time.to_rfc3339()),
        "heartbeatAt": task.heartbeat_at.map(|time| time.to_rfc3339()),
        "completedAt": task.completed_at.map(|time| time.to_rfc3339()),
        "lastError": task.last_error.as_deref(),
        "createdAt": task.created_at.to_rfc3339(),
        "updatedAt": task.updated_at.to_rfc3339(),
        "path": workspace
            .join(".deepcli")
            .join("agents")
            .join("tasks")
            .join(format!("{id}.json"))
            .display()
            .to_string(),
    })
}

fn subagent_status_label(status: &SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::Queued => "queued",
        SubagentStatus::Running => "running",
        SubagentStatus::AwaitingApproval => "awaiting_approval",
        SubagentStatus::Completed => "completed",
        SubagentStatus::Failed => "failed",
    }
}

fn agent_next_actions(task: Option<&SubagentTask>, empty: bool) -> Vec<String> {
    let mut actions = Vec::new();
    if let Some(task) = task {
        actions.push(format!("deepcli agent show {}", short_id(&task.id)));
        actions.push(format!("deepcli agent logs {} --json", short_id(&task.id)));
    } else if empty {
        actions.push("deepcli help agent".to_string());
    } else {
        actions.push("deepcli agent list --json".to_string());
        actions.push("deepcli help agent".to_string());
    }
    if let Some(task) = task {
        match task.status {
            SubagentStatus::Queued => {
                actions.push(format!("deepcli agent resume {}", short_id(&task.id)))
            }
            SubagentStatus::Running | SubagentStatus::Failed => {
                actions.push(format!("deepcli agent resume {}", short_id(&task.id)));
            }
            SubagentStatus::AwaitingApproval => {
                if let Some(session_id) = task.child_session_id {
                    actions.push(format!(
                        "deepcli approval list --session {session_id} --json"
                    ));
                }
                actions.push(format!("deepcli agent resume {}", short_id(&task.id)));
            }
            SubagentStatus::Completed => {}
        }
    }
    actions.push("deepcli agent list --json".to_string());
    dedup_preserve_order(actions)
}

fn agent_list_next_actions(tasks: &[SubagentTask]) -> Vec<String> {
    let mut actions = Vec::new();
    if let Some(task) = tasks.first() {
        actions.push(format!("deepcli agent show {}", short_id(&task.id)));
        actions.push(format!("deepcli agent logs {} --json", short_id(&task.id)));
    } else {
        actions.push("deepcli help agent".to_string());
    }
    actions.push("deepcli agent list --json".to_string());
    dedup_preserve_order(actions)
}
