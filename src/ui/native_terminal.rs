use crate::runtime::{AgentRuntime, RuntimeProgress};
use anyhow::{anyhow, Result};
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use super::worker::WorkerDone;

const INPUT_PROMPT: &str = "> ";
const NATIVE_PROGRESS_DETAIL_CHARS: usize = 120;

#[derive(Default)]
struct NativeRenderState {
    assistant_open: bool,
    saw_assistant_delta: bool,
}

pub(super) async fn run_native_terminal(mut runtime: AgentRuntime) -> Result<()> {
    println!("deepcli session {}", runtime.session_id());
    println!("Type /help for commands, /quit to exit.");

    let (progress_tx, progress_rx) = mpsc::channel();
    let stdin = io::stdin();
    loop {
        print!("{INPUT_PROMPT}");
        io::stdout().flush()?;
        let mut line = String::new();
        let bytes = stdin.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }

        let input = line.trim_end().to_string();
        if input.trim().is_empty() {
            continue;
        }
        if input.trim() == "/quit" {
            break;
        }

        runtime.set_progress_sender(Some(progress_tx.clone()));
        let (done_tx, done_rx) = mpsc::channel();
        let mut task_runtime = runtime;
        let task_input = input.clone();
        tokio::spawn(async move {
            let result = task_runtime
                .handle_input(&task_input)
                .await
                .map_err(|error| error.to_string());
            let _ = done_tx.send(WorkerDone {
                runtime: task_runtime,
                result,
            });
        });

        let mut render_state = NativeRenderState::default();
        runtime = wait_for_native_task(done_rx, &progress_rx, &mut render_state).await?;
    }

    Ok(())
}

async fn wait_for_native_task(
    done_rx: Receiver<WorkerDone>,
    progress_rx: &Receiver<RuntimeProgress>,
    render_state: &mut NativeRenderState,
) -> Result<AgentRuntime> {
    loop {
        drain_native_progress(progress_rx, render_state)?;
        match done_rx.try_recv() {
            Ok(done) => {
                drain_native_progress(progress_rx, render_state)?;
                finish_native_stream_line(render_state)?;
                match done.result {
                    Ok(output) => {
                        if !render_state.saw_assistant_delta {
                            println!("{output}");
                        }
                    }
                    Err(error) => {
                        println!("error: {error}");
                    }
                }
                return Ok(done.runtime);
            }
            Err(TryRecvError::Empty) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(TryRecvError::Disconnected) => {
                finish_native_stream_line(render_state)?;
                return Err(anyhow!("native terminal worker disconnected"));
            }
        }
    }
}

fn drain_native_progress(
    progress_rx: &Receiver<RuntimeProgress>,
    render_state: &mut NativeRenderState,
) -> io::Result<()> {
    while let Ok(event) = progress_rx.try_recv() {
        render_native_progress(event, render_state)?;
    }
    Ok(())
}

fn render_native_progress(
    event: RuntimeProgress,
    render_state: &mut NativeRenderState,
) -> io::Result<()> {
    match event {
        RuntimeProgress::AssistantDelta { delta } => {
            if delta.is_empty() {
                return Ok(());
            }
            if !render_state.assistant_open {
                render_state.assistant_open = true;
            }
            print!("{delta}");
            io::stdout().flush()?;
            render_state.saw_assistant_delta = true;
        }
        other => {
            if let Some(line) = native_progress_line(&other) {
                finish_native_stream_line(render_state)?;
                println!("{line}");
            }
        }
    }
    Ok(())
}

fn native_progress_line(event: &RuntimeProgress) -> Option<String> {
    match event {
        RuntimeProgress::AssistantDelta { .. } => None,
        RuntimeProgress::ProviderStreamStarted => {
            Some("deepcli | provider stream started".to_string())
        }
        RuntimeProgress::ProviderTurnStarted {
            iteration,
            max_iterations,
            message_count,
            tool_count,
            request_kib,
            compacted,
        } => {
            let mut line = format!(
                "deepcli | provider {iteration}/{max_iterations} | messages {message_count} | tools {tool_count} | request {request_kib} KiB"
            );
            if *compacted {
                line.push_str(" | compacted");
            }
            Some(line)
        }
        RuntimeProgress::ProviderTurnCompleted {
            elapsed_ms,
            tool_calls,
        } => Some(format!(
            "deepcli | provider done | {:.1}s | tool calls {tool_calls}",
            *elapsed_ms as f64 / 1000.0
        )),
        RuntimeProgress::ToolStarted { tool, detail } => {
            Some(native_tool_progress_line("run", tool, detail.as_deref()))
        }
        RuntimeProgress::ToolCompleted { tool, ok, summary } => {
            let status = if *ok { "ok" } else { "failed" };
            Some(native_tool_progress_line(status, tool, Some(summary)))
        }
    }
}

fn native_tool_progress_line(status: &str, tool: &str, detail: Option<&str>) -> String {
    let mut line = format!("deepcli | tool {status} | {tool}");
    if let Some(detail) = detail.and_then(native_progress_detail) {
        line.push_str(" | ");
        line.push_str(&detail);
    }
    line
}

fn native_progress_detail(value: &str) -> Option<String> {
    let detail = value.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(compact_native_progress_detail(detail))
}

fn compact_native_progress_detail(value: &str) -> String {
    let mut compacted = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= NATIVE_PROGRESS_DETAIL_CHARS {
            compacted.push_str("...");
            return compacted;
        }
        compacted.push(ch);
    }
    compacted
}

fn finish_native_stream_line(render_state: &mut NativeRenderState) -> io::Result<()> {
    if render_state.assistant_open {
        println!();
        io::stdout().flush()?;
        render_state.assistant_open = false;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_provider_progress_uses_compact_status_lines() {
        let started = RuntimeProgress::ProviderTurnStarted {
            iteration: 2,
            max_iterations: 8,
            message_count: 14,
            tool_count: 9,
            request_kib: 128,
            compacted: true,
        };
        let completed = RuntimeProgress::ProviderTurnCompleted {
            elapsed_ms: 1250,
            tool_calls: 3,
        };

        assert_eq!(
            native_progress_line(&started),
            Some(
                "deepcli | provider 2/8 | messages 14 | tools 9 | request 128 KiB | compacted"
                    .to_string()
            )
        );
        assert_eq!(
            native_progress_line(&completed),
            Some("deepcli | provider done | 1.2s | tool calls 3".to_string())
        );
    }

    #[test]
    fn native_tool_progress_is_visible_and_single_line() {
        let started = RuntimeProgress::ToolStarted {
            tool: "git_status".to_string(),
            detail: Some("git status --short".to_string()),
        };
        let completed = RuntimeProgress::ToolCompleted {
            tool: "git_diff".to_string(),
            ok: true,
            summary: "diff --git a/README.md b/README.md\n@@ -1 +1 @@".to_string(),
        };

        assert_eq!(
            native_progress_line(&started),
            Some("deepcli | tool run | git_status | git status --short".to_string())
        );
        assert_eq!(
            native_progress_line(&completed),
            Some("deepcli | tool ok | git_diff | diff --git a/README.md b/README.md".to_string())
        );
    }
}
