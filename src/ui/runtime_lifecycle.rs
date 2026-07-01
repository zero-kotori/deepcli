use crate::config::AppConfig;
use crate::runtime::{AgentRuntime, RuntimeOptions};
use crate::session::{SessionState, SessionStore};
use anyhow::{anyhow, Result};

use super::{sync_active_session_ref, ActiveSessionRef, ChatLine, TuiState};

pub(super) fn stop_running_task(state: &mut TuiState, exit_after: bool, source: &str) {
    if !state.running {
        if exit_after {
            state.exit_requested = true;
        } else {
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: "当前没有正在运行的任务。".to_string(),
            });
            state.last_event = "no running task to stop".to_string();
        }
        return;
    }

    if let Some(worker) = state.worker.take() {
        worker.abort();
    }
    state.running = false;

    let result = state
        .active_session
        .as_ref()
        .ok_or_else(|| anyhow!("当前运行会话不可用"))
        .and_then(|active| {
            mark_active_session_paused(active, source)?;
            rebuild_runtime_for_active_session(active)
        });

    match result {
        Ok(runtime) => {
            state.runtime = Some(runtime);
            sync_active_session_ref(state);
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: "已停止当前任务；会话已标记为 paused，可通过 `/resume` 继续。".to_string(),
            });
            state.last_event = "task stopped".to_string();
        }
        Err(error) => {
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: format!(
                    "已请求停止当前任务，但恢复交互 runtime 失败：{}。请重新启动 deepcli 并用 `/resume` 恢复会话。",
                    error
                ),
            });
            state.last_event = "task stopped with runtime rebuild error".to_string();
        }
    }

    if exit_after {
        state.exit_requested = true;
    }
}

pub(super) fn mark_active_session_paused(active: &ActiveSessionRef, source: &str) -> Result<()> {
    let store = SessionStore::new(&active.workspace);
    let mut session = store.load(&active.session_id)?;
    session.set_state(SessionState::Paused)?;
    session.append_audit_event(
        "task_stopped",
        serde_json::json!({
            "source": source,
        }),
    )?;
    Ok(())
}

pub(super) fn rebuild_runtime_for_active_session(
    active: &ActiveSessionRef,
) -> Result<AgentRuntime> {
    let config = AppConfig::load_effective(&active.workspace, None)?;
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    AgentRuntime::new(
        config,
        RuntimeOptions {
            workspace: active.workspace.clone(),
            provider: Some(session.metadata.provider),
            model: session.metadata.model,
            assume_yes: false,
            resume_session: Some(active.session_id.clone()),
            stream_output: false,
        },
    )
}
