use super::{
    dedup_preserve_order, local_action_checklist, required_arg, set_command_output_path, short_id,
    write_command_output,
};
use crate::agents::{AgentStore, SubagentStatus, SubagentTask};
use crate::schema_ids;
use crate::tools::ToolExecutor;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) async fn handle_agent(
    workspace: &Path,
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
        Some("spawn") => {
            let task = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if task.trim().is_empty() {
                bail!("/agent spawn requires a task");
            }
            Ok(executor
                .execute("spawn_subagent", json!({"task": task, "depth": 1}))
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
        SubagentStatus::Completed => "completed",
        SubagentStatus::Failed => "failed",
    }
}

fn agent_next_actions(task: Option<&SubagentTask>, empty: bool) -> Vec<String> {
    let mut actions = Vec::new();
    if let Some(task) = task {
        actions.push(format!("deepcli agent show {}", short_id(&task.id)));
    } else if empty {
        actions.push("deepcli help agent".to_string());
    } else {
        actions.push("deepcli agent list --json".to_string());
        actions.push("deepcli help agent".to_string());
    }
    if let Some(task) = task {
        if matches!(task.status, SubagentStatus::Queued) {
            actions.push("deepcli agent list --json".to_string());
        }
    }
    actions.push("deepcli agent list --json".to_string());
    dedup_preserve_order(actions)
}

fn agent_list_next_actions(tasks: &[SubagentTask]) -> Vec<String> {
    let mut actions = Vec::new();
    if let Some(task) = tasks.first() {
        actions.push(format!("deepcli agent show {}", short_id(&task.id)));
    } else {
        actions.push("deepcli help agent".to_string());
    }
    actions.push("deepcli agent list --json".to_string());
    dedup_preserve_order(actions)
}
