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
        let event_text = event.plain_text();
        state.last_event = event_text.clone();
        match event {
            RuntimeProgress::AssistantDelta { delta } => {
                append_assistant_delta(state, &delta);
            }
            RuntimeProgress::ProviderStreamStarted
            | RuntimeProgress::ProviderTurnStarted { .. } => {
                state.streaming_assistant = None;
                state.tool_log.push(ToolLogItem {
                    title: event.plain_text(),
                    detail: event.plain_text(),
                    expanded: false,
                });
            }
            RuntimeProgress::ProviderTurnCompleted { tool_calls, .. } => {
                if tool_calls > 0 {
                    state.streaming_assistant = None;
                }
                state.tool_log.push(ToolLogItem {
                    title: event.plain_text(),
                    detail: event.plain_text(),
                    expanded: false,
                });
            }
            RuntimeProgress::ToolStarted { tool, detail } => {
                state.tool_log.push(ToolLogItem {
                    title: format!("tool: {tool}"),
                    detail: detail.unwrap_or_else(|| format!("正在运行工具 `{tool}`")),
                    expanded: false,
                });
                state.chat.push(ChatLine {
                    role: "deepcli".to_string(),
                    content: event_text,
                });
                state.transcript_scroll = 0;
                state.streaming_assistant = None;
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
        }
    }
}

fn append_assistant_delta(state: &mut TuiState, delta: &str) {
    if delta.is_empty() {
        return;
    }
    let index = match state.streaming_assistant {
        Some(index)
            if state
                .chat
                .get(index)
                .is_some_and(|line| line.role == "deepcli") =>
        {
            index
        }
        _ => {
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: String::new(),
            });
            let index = state.chat.len().saturating_sub(1);
            state.streaming_assistant = Some(index);
            index
        }
    };
    if let Some(line) = state.chat.get_mut(index) {
        line.content.push_str(delta);
    }
    state.transcript_scroll = 0;
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
        state.transcript_scroll = 0;
        state.result_scroll = 0;
        match done.result {
            Ok(output) => {
                state.last_event = format_action_event("action ok", &output);
                if let Some(index) = state.streaming_assistant.take().filter(|index| {
                    state
                        .chat
                        .get(*index)
                        .is_some_and(|line| line.role == "deepcli")
                }) {
                    if let Some(line) = state.chat.get_mut(index) {
                        line.content = output;
                    }
                } else {
                    state.chat.push(ChatLine {
                        role: "deepcli".to_string(),
                        content: output,
                    });
                }
            }
            Err(error) => {
                state.last_event = format_action_event("action failed", &error);
                state.streaming_assistant = None;
                state.chat.push(ChatLine {
                    role: "error".to_string(),
                    content: error,
                });
            }
        }
    }
}
