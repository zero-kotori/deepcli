use crate::runtime::{AgentRuntime, RuntimeProgress};
use anyhow::{anyhow, Result};
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use super::worker::WorkerDone;

const INPUT_PROMPT: &str = "> ";

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
        RuntimeProgress::AssistantDelta { .. }
        | RuntimeProgress::ProviderStreamStarted
        | RuntimeProgress::ProviderTurnStarted { .. }
        | RuntimeProgress::ProviderTurnCompleted { .. }
        | RuntimeProgress::ToolStarted { .. }
        | RuntimeProgress::ToolCompleted { .. } => None,
    }
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
    fn native_tool_progress_is_hidden_from_terminal_output() {
        let started = RuntimeProgress::ToolStarted {
            tool: "git_status".to_string(),
            detail: Some("git status --short".to_string()),
        };
        let completed = RuntimeProgress::ToolCompleted {
            tool: "git_diff".to_string(),
            ok: true,
            summary: "diff --git a/README.md b/README.md\n@@ -1 +1 @@".to_string(),
        };

        assert_eq!(native_progress_line(&started), None);
        assert_eq!(native_progress_line(&completed), None);
    }
}
