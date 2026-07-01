use crate::runtime::RuntimeProgress;
use anyhow::Result;
use std::sync::mpsc::Sender;

use super::credential_prompt::handle_tui_credential_set_for_state;
use super::resume_picker::ResumePicker;
use super::running_commands::{handle_running_tui_local_command, running_tui_deferred_input_hint};
use super::runtime_lifecycle::stop_running_task;
use super::session_projection::sync_active_session_ref;
use super::worker::WorkerDone;
use super::{chat_lines_from_runtime, ChatLine, MonitorTab, TuiState};

pub(super) fn submit_tui_input(
    state: &mut TuiState,
    input: String,
    progress_tx: Sender<RuntimeProgress>,
    done_tx: Sender<WorkerDone>,
) {
    if input.trim().is_empty() {
        return;
    }
    state.transcript_scroll = 0;
    state.result_scroll = 0;
    let trimmed = input.trim();
    if trimmed == "/quit" {
        if state.running {
            stop_running_task(state, true, trimmed);
            return;
        }
        state.exit_requested = true;
        return;
    }
    if handle_running_tui_local_command(state, trimmed) {
        return;
    }
    if handle_tui_local_command(state, trimmed) {
        return;
    }
    if state.running {
        state.chat.push(ChatLine {
            role: "你".to_string(),
            content: input.clone(),
        });
        state.chat.push(ChatLine {
            role: "deepcli".to_string(),
            content: running_tui_deferred_input_hint(),
        });
        state.last_event = "input deferred while running".to_string();
        return;
    }
    sync_active_session_ref(state);
    let Some(mut runtime) = state.runtime.take() else {
        return;
    };
    runtime.set_progress_sender(Some(progress_tx));
    state.chat.push(ChatLine {
        role: "你".to_string(),
        content: input.clone(),
    });
    state.running = true;
    state.last_event = "running".to_string();
    state.worker = Some(tokio::spawn(async move {
        let result = runtime
            .handle_input(&input)
            .await
            .map_err(|error| error.to_string());
        let _ = done_tx.send(WorkerDone { runtime, result });
    }));
}

pub(super) fn handle_tui_local_command(state: &mut TuiState, input: &str) -> bool {
    if handle_tui_credential_set_for_state(state, input) {
        return true;
    }

    if input == "/resume" {
        let Some(runtime) = state.runtime.as_mut() else {
            return false;
        };
        match runtime.list_sessions() {
            Ok(sessions) if sessions.is_empty() => state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: "当前目录没有可恢复的历史会话。可用 `/session list --all` 查看空会话。"
                    .to_string(),
            }),
            Ok(sessions) => {
                state.resume_picker = Some(ResumePicker::new(sessions));
            }
            Err(error) => state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            }),
        }
        return true;
    }

    if let Some(id) = input.strip_prefix("/resume ") {
        let result = {
            let Some(runtime) = state.runtime.as_mut() else {
                return false;
            };
            runtime
                .resume_session(id.trim())
                .map(|message| (message, chat_lines_from_runtime(runtime)))
        };
        apply_resume_result(state, result);
        return true;
    }

    if input == "/rename" {
        let Some(runtime) = state.runtime.as_mut() else {
            return false;
        };
        match runtime.rename_current_session("") {
            Ok(message) => state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: message,
            }),
            Err(error) => state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            }),
        }
        return true;
    }

    if let Some(title) = input.strip_prefix("/rename ") {
        let Some(runtime) = state.runtime.as_mut() else {
            return false;
        };
        match runtime.rename_current_session(title.trim()) {
            Ok(message) => state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: message,
            }),
            Err(error) => state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            }),
        }
        return true;
    }

    false
}

pub(super) fn apply_resume_result(
    state: &mut TuiState,
    result: Result<(String, Result<Vec<ChatLine>>)>,
) {
    match result {
        Ok((message, Ok(mut chat))) => {
            chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: message,
            });
            state.chat = chat;
            state.tool_log.clear();
            state.selected_tool = None;
            state.monitor_tab = MonitorTab::Overview;
            sync_active_session_ref(state);
            state.last_event = "session resumed".to_string();
        }
        Ok((message, Err(error))) => {
            state.chat = vec![
                ChatLine {
                    role: "deepcli".to_string(),
                    content: message,
                },
                ChatLine {
                    role: "error".to_string(),
                    content: format!("读取历史会话失败：{error}"),
                },
            ];
            state.tool_log.clear();
            state.selected_tool = None;
            state.monitor_tab = MonitorTab::Overview;
            sync_active_session_ref(state);
            state.last_event = "session resumed with history error".to_string();
        }
        Err(error) => state.chat.push(ChatLine {
            role: "error".to_string(),
            content: error.to_string(),
        }),
    }
}
