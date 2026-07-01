use crate::agents::AgentStore;
use crate::prompts::PromptStore;
use crate::skills::SkillStore;

use super::monitor::{append_monitor_quick_actions, MonitorQuickAction};
use super::{compact_ui_text, short_id, workspace_for_state, TuiState};

pub(super) fn library_quick_actions_for_state(state: &TuiState) -> Vec<MonitorQuickAction> {
    if workspace_for_state(state).is_none() {
        return vec![
            MonitorQuickAction::run("/prompt list --json"),
            MonitorQuickAction::run("/skill list --json"),
            MonitorQuickAction::run("/agent list --json"),
        ];
    }
    vec![
        MonitorQuickAction::run("/prompt list --json"),
        MonitorQuickAction::edit("/prompt render <name> --file path"),
        MonitorQuickAction::run("/skill list --json"),
        MonitorQuickAction::run("/agent list --json"),
    ]
}

pub(super) fn format_library_tab_lines(
    state: &TuiState,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(workspace) = workspace_for_state(state) else {
        let mut lines = vec!["library unavailable: no workspace".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let mut lines = Vec::new();
    match PromptStore::new(workspace).list() {
        Ok(prompts) => {
            let custom_count = prompts
                .iter()
                .filter(|prompt| {
                    workspace
                        .join(".deepcli")
                        .join("prompts")
                        .join(format!("{}.md", prompt.name))
                        .exists()
                })
                .count();
            lines.push(format!(
                "prompts: total={} custom={} builtins={}",
                prompts.len(),
                custom_count,
                prompts.len().saturating_sub(custom_count)
            ));
            lines.extend(
                prompts
                    .iter()
                    .take(3)
                    .map(|prompt| format_library_item("prompt", &prompt.name, &prompt.description)),
            );
        }
        Err(error) => lines.push(format!(
            "prompts: error={}",
            compact_ui_text(&error.to_string(), 90)
        )),
    }
    match SkillStore::new(workspace).discover() {
        Ok(skills) if skills.is_empty() => {
            lines.push("skills: none registered".to_string());
        }
        Ok(skills) => {
            lines.push(format!("skills: total={}", skills.len()));
            lines.extend(
                skills
                    .iter()
                    .take(3)
                    .map(|skill| format_library_item("skill", &skill.name, &skill.description)),
            );
        }
        Err(error) => lines.push(format!(
            "skills: error={}",
            compact_ui_text(&error.to_string(), 90)
        )),
    }
    match AgentStore::new(workspace).list() {
        Ok(tasks) if tasks.is_empty() => {
            lines.push("agents: no sub-agent tasks".to_string());
        }
        Ok(tasks) => {
            lines.push(format!("agents: total={}", tasks.len()));
            lines.extend(tasks.iter().rev().take(3).map(|task| {
                format!(
                    "  agent {} status={:?} {}",
                    short_id(&task.id.to_string()),
                    task.status,
                    compact_ui_text(&task.task, 58)
                )
            }));
        }
        Err(error) => lines.push(format!(
            "agents: error={}",
            compact_ui_text(&error.to_string(), 90)
        )),
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn format_library_item(kind: &str, name: &str, description: &str) -> String {
    format!(
        "  {kind} {} - {}",
        compact_ui_text(name, 28),
        compact_ui_text(description, 62)
    )
}
