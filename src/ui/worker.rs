use crate::runtime::{AgentRuntime, RuntimeProgress};
use std::sync::mpsc::Receiver;

use super::monitor_tools::ToolLogItem;
use super::{format_action_event, sync_active_session_ref, ChatLine, TuiState};

pub(super) struct WorkerDone {
    pub(super) runtime: AgentRuntime,
    pub(super) result: std::result::Result<String, String>,
}

pub(super) fn drain_progress(state: &mut TuiState, progress_rx: &Receiver<RuntimeProgress>) {
    while let Ok(event) = progress_rx.try_recv() {
        state.last_event = event.plain_text();
        match event {
            RuntimeProgress::ToolStarted { tool } => {
                state.tool_log.push(ToolLogItem {
                    title: format!("tool: {tool}"),
                    detail: format!("正在运行工具 `{tool}`"),
                    expanded: false,
                });
                if state.selected_tool.is_none() {
                    state.selected_tool = Some(0);
                }
            }
            RuntimeProgress::ToolCompleted { tool, ok, summary } => {
                let status = if ok { "done" } else { "failed" };
                if let Some(item) = state
                    .tool_log
                    .iter_mut()
                    .rev()
                    .find(|item| item.title == format!("tool: {tool}"))
                {
                    item.title = format!("tool: {tool} [{status}]");
                    item.detail = summary;
                } else {
                    state.tool_log.push(ToolLogItem {
                        title: format!("tool: {tool} [{status}]"),
                        detail: summary,
                        expanded: false,
                    });
                }
            }
            other => {
                state.tool_log.push(ToolLogItem {
                    title: other.plain_text(),
                    detail: other.plain_text(),
                    expanded: false,
                });
            }
        }
    }
}

pub(super) fn drain_done(state: &mut TuiState, done_rx: &Receiver<WorkerDone>) {
    while let Ok(done) = done_rx.try_recv() {
        if state.worker.is_none() && !state.running {
            continue;
        }
        state.worker = None;
        state.runtime = Some(done.runtime);
        sync_active_session_ref(state);
        state.running = false;
        state.result_scroll = 0;
        match done.result {
            Ok(output) => {
                state.last_event = format_action_event("action ok", &output);
                state.chat.push(ChatLine {
                    role: "deepcli".to_string(),
                    content: output,
                });
            }
            Err(error) => {
                state.last_event = format_action_event("action failed", &error);
                state.chat.push(ChatLine {
                    role: "error".to_string(),
                    content: error,
                });
            }
        }
    }
}
