#[cfg(test)]
use crate::commands::CommandRouter;
use crate::runtime::{AgentRuntime, RuntimeProgress};
#[cfg(test)]
use crate::session::SessionMessage;
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyModifiers, KeyboardEnhancementFlags, MouseButton, MouseEvent,
        MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};
use std::io::{self, Stdout, Write};
#[cfg(test)]
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

mod approvals;
mod chat_history;
mod chat_view;
mod clipboard;
mod command_palette;
mod credential_prompt;
mod dashboard;
mod geometry;
mod input_submission;
mod message_box;
mod monitor;
mod monitor_changes;
mod monitor_health;
mod monitor_library;
mod monitor_output;
mod monitor_shell;
mod monitor_tools;
mod paste;
mod quick_actions;
mod resume_picker;
mod running_commands;
mod runtime_lifecycle;
mod scrolling;
mod session_projection;
mod text;
mod worker;
#[cfg(test)]
use approvals::blocker_count;
use approvals::{
    handle_approval_tab_key, handle_approvals_mouse_for_state, handle_side_question_prompt_key,
    SideQuestionPrompt,
};
#[cfg(test)]
use chat_history::session_messages_to_chat_lines;
use chat_history::{chat_lines_from_runtime, ChatLine};
use chat_view::{chat_ui_layout, clamp_transcript_scroll_to_area, render_chat_ui};
#[cfg(test)]
use chat_view::{format_transcript_text, message_box_cursor_position};
use clipboard::{handle_clipboard_key, handle_input_selection_mouse};
use command_palette::{
    clamp_selected_command, handle_command_palette_key, handle_command_palette_mouse_for_state,
    render_command_palette, slash_command_suggestions_for_state,
};
#[cfg(test)]
use command_palette::{
    command_palette_match_token, command_palette_matches_line_index, complete_selected_command,
    format_command_palette_text, running_safe_palette_priority, COMMAND_PALETTE_MATCH_LIMIT,
};
#[cfg(test)]
use credential_prompt::parse_tui_credential_set;
use credential_prompt::{
    credential_prompt_hidden_body, credential_prompt_hidden_cursor, handle_credential_prompt_key,
    CredentialPrompt,
};
pub use dashboard::{render_dashboard, TuiSnapshot};
use geometry::{rect_contains, rect_content_row_contains};
use input_submission::{apply_resume_result, submit_tui_input};
use message_box::{handle_prompt_input_key, MessageBox, MessageBoxAction};
#[cfg(test)]
use monitor::environment_quick_actions;
use monitor::MonitorTab;
#[cfg(test)]
use monitor::MonitorTier;
#[cfg(test)]
use monitor_changes::{
    append_workspace_changes_lines, parse_diff_sections, parse_git_status_snapshot,
    WorkspaceDiffSection, CHANGE_PATCH_SCROLL_STEP, WORKTREE_DIFF_SECTION_LINES,
};
use monitor_changes::{
    handle_changes_tab_key, refresh_workspace_changes_snapshot, scroll_change_patch_down,
    scroll_change_patch_up, select_change_patch_at_row, WorkspaceChangesSnapshot,
    CHANGE_PATCH_MOUSE_SCROLL_STEP,
};
#[cfg(test)]
use monitor_health::health_quick_actions_for_state;
use monitor_shell::select_monitor_tab_at_position;
#[cfg(test)]
use monitor_shell::{
    first_advanced_monitor_tab, format_monitor_tabs, format_task_monitor_text,
    format_task_overview_lines, monitor_quick_actions_for_tab, visible_panel_line_indices,
    MONITOR_ADVANCED_TOGGLE_LABEL,
};
#[cfg(test)]
use monitor_tools::selected_tool_panel_line;
use monitor_tools::{
    handle_tools_tab_key, move_selected_tool_by, toggle_selected_tool, toggle_tool_at_row,
    ToolLogItem, TOOL_MOUSE_SCROLL_STEP,
};
#[cfg(test)]
use monitor_tools::{tool_tab_lines, TOOL_KEY_SCROLL_STEP};
use paste::{handle_tui_paste, normalize_pasted_text};
#[cfg(test)]
use quick_actions::activate_selected_monitor_quick_action;
use quick_actions::{activate_monitor_quick_action_at_row, handle_monitor_quick_action_key};
#[cfg(test)]
use resume_picker::{format_resume_preview_text, resume_picker_layout};
use resume_picker::{handle_resume_picker_key, handle_resume_picker_mouse_for_state};
pub use resume_picker::{pick_resume_session, ResumeSelection};
use resume_picker::{render_resume_picker, ResumePicker};
#[cfg(test)]
use running_commands::{
    ensure_running_completion_is_observation_only, handle_running_tui_local_command,
    running_tui_supported_command_hint,
};
use runtime_lifecycle::stop_running_task;
use scrolling::{
    handle_result_scroll_key, handle_transcript_scroll_key, scroll_result_down,
    scroll_result_from_mouse, scroll_transcript, transcript_scroll_event, RESULT_MOUSE_SCROLL_STEP,
    TRANSCRIPT_MOUSE_SCROLL_STEP,
};
#[cfg(test)]
use scrolling::{RESULT_SCROLL_STEP, TRANSCRIPT_SCROLL_STEP};
use session_projection::{
    active_session_ref, header_status_for_state, session_monitor_for_state,
    sync_active_session_ref, workspace_for_state, ActiveSessionRef,
};
use text::{
    compact_ui_text, format_action_event, format_cache_hit_rate, format_latest_environment,
    format_optional_bytes, format_optional_u64, latest_action_result, latest_action_result_line,
    non_empty_output_lines, short_id,
};
use worker::{drain_done, drain_progress, WorkerDone};

const RESUME_PICKER_MOUSE_SCROLL_STEP: usize = 3;
const TUI_EVENT_BATCH_LIMIT: usize = 128;

pub async fn run_basic_repl(runtime: &mut AgentRuntime) -> Result<()> {
    println!("deepcli session {}", runtime.session_id());
    println!("Type /help for commands, Ctrl-D to exit.");
    let stdin = io::stdin();
    loop {
        print!("deepcli> ");
        io::stdout().flush()?;
        let mut line = String::new();
        let bytes = stdin.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        let input = line.trim_end();
        if input.is_empty() {
            continue;
        }
        if input.trim() == "/quit" {
            break;
        }
        let output = runtime.handle_input(input).await?;
        println!("{output}");
    }
    Ok(())
}

struct TuiState {
    runtime: Option<AgentRuntime>,
    active_session: Option<ActiveSessionRef>,
    input: MessageBox,
    chat: Vec<ChatLine>,
    transcript_scroll: usize,
    result_scroll: usize,
    workspace_changes: Option<WorkspaceChangesSnapshot>,
    workspace_changes_checked_at: Option<Instant>,
    tool_log: Vec<ToolLogItem>,
    resume_picker: Option<ResumePicker>,
    credential_prompt: Option<CredentialPrompt>,
    side_question_prompt: Option<SideQuestionPrompt>,
    selected_tool: Option<usize>,
    selected_command: usize,
    selected_change: usize,
    change_patch_scroll: usize,
    monitor_tab: MonitorTab,
    selected_approval: usize,
    running: bool,
    exit_requested: bool,
    last_event: String,
    streaming_assistant: Option<usize>,
    worker: Option<JoinHandle<()>>,
}

pub async fn run_tui(mut runtime: AgentRuntime) -> Result<()> {
    let (progress_tx, progress_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    runtime.set_progress_sender(Some(progress_tx.clone()));
    let chat = match chat_lines_from_runtime(&runtime) {
        Ok(chat) => chat,
        Err(error) => vec![ChatLine {
            role: "error".to_string(),
            content: format!("读取历史会话失败：{error}"),
        }],
    };
    let active_session = Some(active_session_ref(&runtime));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_tui_loop(
        &mut terminal,
        TuiState {
            runtime: Some(runtime),
            active_session,
            input: MessageBox::new(),
            chat,
            transcript_scroll: 0,
            result_scroll: 0,
            workspace_changes: None,
            workspace_changes_checked_at: None,
            tool_log: Vec::new(),
            resume_picker: None,
            credential_prompt: None,
            side_question_prompt: None,
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            streaming_assistant: None,
            worker: None,
        },
        progress_tx,
        progress_rx,
        done_tx,
        done_rx,
    )
    .await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;
    result
}

async fn run_tui_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    mut state: TuiState,
    progress_tx: Sender<RuntimeProgress>,
    progress_rx: Receiver<RuntimeProgress>,
    done_tx: Sender<WorkerDone>,
    done_rx: Receiver<WorkerDone>,
) -> Result<()> {
    while !state.exit_requested {
        drain_progress(&mut state, &progress_rx);
        drain_done(&mut state, &done_rx);
        refresh_workspace_changes_snapshot(&mut state);

        terminal.draw(|frame| render_chat_ui(frame, &state))?;

        if event::poll(Duration::from_millis(50))? {
            let mut processed_events = 0usize;
            while processed_events < TUI_EVENT_BATCH_LIMIT {
                match event::read()? {
                    Event::Key(key) => handle_tui_key(key, &mut state, &progress_tx, &done_tx)?,
                    Event::Paste(text) => handle_tui_paste(&mut state, &text),
                    Event::Mouse(mouse) => {
                        let size = terminal.size()?;
                        handle_tui_mouse(
                            &mut state,
                            mouse,
                            &progress_tx,
                            &done_tx,
                            Rect {
                                x: 0,
                                y: 0,
                                width: size.width,
                                height: size.height,
                            },
                        );
                    }
                    _ => {}
                }
                processed_events += 1;
                if state.exit_requested || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn handle_tui_mouse(
    state: &mut TuiState,
    mouse: MouseEvent,
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
    area: Rect,
) {
    let areas = chat_ui_layout(area);
    if handle_input_selection_mouse(state, mouse, areas.input) {
        return;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if rect_contains(areas.tools, mouse.column, mouse.row)
                && handle_tools_scroll_mouse(state, mouse, areas.tools, true)
            {
                return;
            }
            scroll_transcript(state, TRANSCRIPT_MOUSE_SCROLL_STEP);
            clamp_transcript_scroll_to_area(state, areas.transcript);
            state.last_event = transcript_scroll_event(state);
        }
        MouseEventKind::ScrollDown => {
            if rect_contains(areas.tools, mouse.column, mouse.row)
                && handle_tools_scroll_mouse(state, mouse, areas.tools, false)
            {
                return;
            }
            state.transcript_scroll = state
                .transcript_scroll
                .saturating_sub(TRANSCRIPT_MOUSE_SCROLL_STEP);
            clamp_transcript_scroll_to_area(state, areas.transcript);
            state.last_event = transcript_scroll_event(state);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if handle_resume_picker_mouse_for_state(state, mouse, areas.tools) {
                return;
            }
            if handle_command_palette_mouse_for_state(state, mouse, areas.tools) {
                return;
            }
            if select_monitor_tab_at_position(state, areas.tools, mouse.column, mouse.row) {
                return;
            }
            if handle_approvals_mouse_for_state(state, mouse, areas.tools) {
                return;
            }
            if select_change_patch_at_row(state, areas.tools, mouse.column, mouse.row) {
                return;
            }
            if activate_monitor_quick_action_at_row(
                state,
                areas.tools,
                mouse.row,
                progress_tx,
                done_tx,
            ) {
                return;
            }
            toggle_tool_at_row(state, areas.tools, mouse.row);
        }
        _ => {}
    }
}

fn handle_tools_scroll_mouse(
    state: &mut TuiState,
    mouse: MouseEvent,
    tools_area: Rect,
    upward: bool,
) -> bool {
    if handle_resume_picker_mouse_for_state(state, mouse, tools_area) {
        return true;
    }
    if handle_command_palette_mouse_for_state(state, mouse, tools_area) {
        return true;
    }
    if handle_approvals_mouse_for_state(state, mouse, tools_area) {
        return true;
    }
    if state.monitor_tab == MonitorTab::Tools {
        if upward {
            move_selected_tool_by(state, false, TOOL_MOUSE_SCROLL_STEP);
        } else {
            move_selected_tool_by(state, true, TOOL_MOUSE_SCROLL_STEP);
        }
        true
    } else if state.monitor_tab == MonitorTab::Changes {
        if upward {
            scroll_change_patch_up(state, CHANGE_PATCH_MOUSE_SCROLL_STEP);
        } else {
            scroll_change_patch_down(state, CHANGE_PATCH_MOUSE_SCROLL_STEP);
        }
        true
    } else if state.monitor_tab == MonitorTab::Result && state.input.buffer().trim().is_empty() {
        if upward {
            scroll_result_from_mouse(state, RESULT_MOUSE_SCROLL_STEP);
        } else {
            scroll_result_down(state, RESULT_MOUSE_SCROLL_STEP);
        }
        true
    } else {
        false
    }
}

fn handle_tui_key(
    key: KeyEvent,
    state: &mut TuiState,
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
) -> Result<()> {
    let mut stdout = io::stdout();
    handle_tui_key_with_clipboard_writer(key, state, progress_tx, done_tx, &mut stdout)
}

fn handle_tui_key_with_clipboard_writer<W: Write>(
    key: KeyEvent,
    state: &mut TuiState,
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
    clipboard_writer: &mut W,
) -> Result<()> {
    if handle_clipboard_key(key, state, clipboard_writer)? {
        return Ok(());
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'd'))
    {
        if state.credential_prompt.take().is_some() {
            state.last_event = "credential prompt cancelled".to_string();
            return Ok(());
        }
        if state.side_question_prompt.take().is_some() {
            state.last_event = "btw answer cancelled".to_string();
            return Ok(());
        }
        if state.running {
            stop_running_task(state, false, "keyboard interrupt");
        } else {
            state.exit_requested = true;
        }
        return Ok(());
    }
    if key.code == KeyCode::Esc {
        if state.credential_prompt.take().is_some() {
            state.last_event = "credential prompt cancelled".to_string();
            return Ok(());
        }
        if state.side_question_prompt.take().is_some() {
            state.last_event = "btw answer cancelled".to_string();
            return Ok(());
        }
        if state.resume_picker.is_some() {
            state.resume_picker = None;
            return Ok(());
        }
        if slash_command_suggestions_for_state(state.input.buffer(), state.running).is_some() {
            state.input.clear();
            state.selected_command = 0;
            state.last_event = "command help dismissed".to_string();
            return Ok(());
        }
        if state.running {
            stop_running_task(state, false, "escape");
        } else {
            state.exit_requested = true;
        }
        return Ok(());
    }
    if state.credential_prompt.is_some() {
        handle_credential_prompt_key(key, state);
        return Ok(());
    }
    if state.side_question_prompt.is_some() {
        handle_side_question_prompt_key(key, state);
        return Ok(());
    }
    if state.resume_picker.is_some() {
        handle_resume_picker_key(key, state);
        return Ok(());
    }
    if handle_command_palette_key(key, state) {
        return Ok(());
    }
    if handle_approval_tab_key(key, state) {
        return Ok(());
    }
    if handle_changes_tab_key(key, state) {
        return Ok(());
    }
    if handle_tools_tab_key(key, state) {
        return Ok(());
    }
    if handle_monitor_quick_action_key(key, state, progress_tx, done_tx) {
        return Ok(());
    }
    if handle_result_scroll_key(key, state) {
        return Ok(());
    }
    if handle_transcript_scroll_key(key, state) {
        return Ok(());
    }
    match key.code {
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            cycle_monitor_tab(state, true);
        }
        KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
            cycle_monitor_tab(state, true);
        }
        KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
            cycle_monitor_tab(state, false);
        }
        KeyCode::Tab => {
            if state.monitor_tab != MonitorTab::Tools {
                cycle_monitor_tab(state, true);
                return Ok(());
            }
            if state.tool_log.is_empty() {
                return Ok(());
            }
            let next = state
                .selected_tool
                .map(|index| (index + 1) % state.tool_log.len())
                .unwrap_or(0);
            state.selected_tool = Some(next);
        }
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
            toggle_selected_tool(state);
        }
        _ => match state.input.handle_key(key) {
            MessageBoxAction::Submitted(input) => {
                state.selected_command = 0;
                submit_tui_input(state, input, progress_tx.clone(), done_tx.clone())
            }
            MessageBoxAction::Inserted => clamp_selected_command(state),
            MessageBoxAction::Noop => {}
        },
    }
    Ok(())
}

fn cycle_monitor_tab(state: &mut TuiState, forward: bool) {
    state.monitor_tab = if forward {
        state.monitor_tab.next()
    } else {
        state.monitor_tab.previous()
    };
    state.selected_command = 0;
    state.last_event = format!("monitor tab: {}", state.monitor_tab.label());
}

#[cfg(test)]
mod tests;
