use crate::agents::AgentStore;
use crate::prompts::PromptStore;
use crate::skills::SkillStore;

use super::*;
use crate::config::AppConfig;
use crate::permissions::{DecisionOutcome, PermissionDecision, RiskLevel};
use crate::runtime::{
    RuntimeOptions, SessionMonitor, SessionObservation, SessionObservationApproval,
    SessionObservationEnvironment, SessionObservationEvent, SessionObservationQuestion,
    SessionObservationTest, SessionObservationUsage,
};
use crate::session::{
    ApprovalStatus, SessionState, SessionStore, SideQuestionStatus, ToolCallRecord, ToolCallStatus,
};
use chrono::Utc;
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn test_tui_state() -> TuiState {
    TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
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
    }
}

#[test]
fn ui_dialog_shell_replaces_input_area_and_esc_closes_first() {
    let backend = TestBackend::new(90, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = test_tui_state();
    state.input.set_buffer("draft input".to_string());
    state.dialog = Some(TuiDialog::notice(
        DialogKind::Settings,
        "Settings",
        "agent.providerTurnTimeoutSeconds = 600",
    ));

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Settings"));
    assert!(rendered.contains("agent.providerTurnTimeoutSeconds"));
    assert!(!rendered.contains("Message Box"));
    assert!(!rendered.contains("draft input"));

    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();
    let mut clipboard = Vec::new();
    handle_tui_key_with_clipboard_writer(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &mut state,
        &progress_tx,
        &done_tx,
        &mut clipboard,
    )
    .unwrap();

    assert!(state.dialog.is_none());
    assert!(!state.exit_requested);
    assert_eq!(state.input.buffer(), "draft input");
}

#[test]
fn shift_enter_inserts_newline_and_enter_submits() {
    let mut box_state = MessageBox::new();
    box_state.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    box_state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
    box_state.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
    assert_eq!(box_state.buffer(), "a\nb");
    let action = box_state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(action, MessageBoxAction::Submitted("a\nb".to_string()));
    assert_eq!(box_state.buffer(), "");
}

#[test]
fn message_box_supports_cursor_editing_shortcuts() {
    let mut box_state = MessageBox::new();
    for ch in "abc".chars() {
        box_state.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    box_state.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    box_state.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    box_state.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
    assert_eq!(box_state.buffer(), "aXbc");
    assert_eq!(box_state.cursor(), 2);

    box_state.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert_eq!(box_state.buffer(), "aXc");
    box_state.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(box_state.buffer(), "ac");
    assert_eq!(box_state.cursor(), 1);

    box_state.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    box_state.handle_key(KeyEvent::new(KeyCode::Char('你'), KeyModifiers::NONE));
    assert_eq!(box_state.buffer(), "你ac");
    assert_eq!(box_state.cursor(), "你".len());

    box_state.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    box_state.handle_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
    assert_eq!(box_state.buffer(), "你ac!");
    box_state.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    box_state.handle_key(KeyEvent::new(KeyCode::Char('>'), KeyModifiers::NONE));
    assert_eq!(box_state.buffer(), ">你ac!");
    box_state.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    box_state.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(box_state.buffer(), "");

    for ch in "abc".chars() {
        box_state.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    box_state.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    box_state.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert_eq!(box_state.buffer(), "ab");
}

#[test]
fn message_box_history_restores_cursor_to_end() {
    let mut box_state = MessageBox::new();
    for ch in "first".chars() {
        box_state.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    box_state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    box_state.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));

    assert_eq!(box_state.buffer(), "first");
    assert_eq!(box_state.cursor(), "first".len());
}

#[test]
fn message_box_inserts_pasted_text_at_cursor() {
    let mut box_state = MessageBox::new();
    for ch in "ac".chars() {
        box_state.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    box_state.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    box_state.insert_str("b\n你");

    assert_eq!(box_state.buffer(), "ab\n你c");
    assert_eq!(box_state.cursor(), "ab\n你".len());
}

#[test]
fn tui_paste_inserts_into_message_box_and_normalizes_newlines() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
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
    };
    state
        .input
        .handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    handle_tui_paste(&mut state, "b\r\nc\rd");

    assert_eq!(state.input.buffer(), "ab\nc\nd");
    assert_eq!(state.input.cursor(), "ab\nc\nd".len());
    assert_eq!(state.last_event, "pasted 5 char(s)");
}

#[test]
fn tui_paste_targets_active_prompt() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: Some(CredentialPrompt {
            provider: "deepseek".to_string(),
            force: false,
            input: MessageBox::new(),
        }),
        side_question_prompt: None,
        dialog: None,
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
    };

    for ch in "seet".chars() {
        handle_credential_prompt_key(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            &mut state,
        );
    }
    handle_credential_prompt_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &mut state);
    handle_credential_prompt_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &mut state);
    handle_tui_paste(&mut state, "cr");

    assert_eq!(
        state.credential_prompt.as_ref().unwrap().input.buffer(),
        "secret"
    );
    assert_eq!(
        state.credential_prompt.as_ref().unwrap().input.cursor(),
        "secr".len()
    );
    assert_eq!(
        credential_prompt_hidden_body(state.credential_prompt.as_ref().unwrap()),
        "******"
    );
    assert_eq!(
        credential_prompt_hidden_cursor(state.credential_prompt.as_ref().unwrap()),
        "****".len()
    );
    assert_eq!(state.input.buffer(), "");
    assert_eq!(state.last_event, "pasted 2 hidden char(s)");
}

#[test]
fn tui_ctrl_c_copies_selected_message_box_text_without_exiting() {
    let mut state = test_tui_state();
    state.input.set_buffer("hello world".to_string());
    for _ in 0..5 {
        state
            .input
            .handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    }
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();
    let mut output = Vec::new();

    handle_tui_key_with_clipboard_writer(
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &mut state,
        &progress_tx,
        &done_tx,
        &mut output,
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\u{1b}]52;c;d29ybGQ=\u{7}"
    );
    assert!(!state.exit_requested);
    assert_eq!(
        state.last_event,
        "copied selected text to clipboard (5 chars)"
    );
}

#[test]
fn tui_terminal_setup_captures_scroll_without_all_motion_tracking() {
    let setup = tui_terminal_setup_commands();
    let teardown = tui_terminal_teardown_commands();
    let setup_debug = format!("{setup:?}");
    let teardown_debug = format!("{teardown:?}");
    let mut setup_output = Vec::new();
    let mut teardown_output = Vec::new();

    assert_eq!(
        setup,
        &[
            TuiTerminalCommand::EnterAlternateScreen,
            TuiTerminalCommand::EnableMouseScrollCapture,
            TuiTerminalCommand::EnableBracketedPaste,
        ]
    );
    assert!(setup_debug.contains("EnableMouseScrollCapture"));
    assert!(!setup_debug.contains("KeyboardEnhancement"));
    assert_eq!(
        teardown,
        &[
            TuiTerminalCommand::LeaveAlternateScreen,
            TuiTerminalCommand::DisableMouseScrollCapture,
            TuiTerminalCommand::DisableBracketedPaste,
        ]
    );
    assert!(teardown_debug.contains("DisableMouseScrollCapture"));
    assert!(!teardown_debug.contains("KeyboardEnhancement"));

    apply_tui_terminal_setup(&mut setup_output).unwrap();
    let setup_output = String::from_utf8(setup_output).unwrap();
    assert!(setup_output.contains("\u{1b}[?1000h"));
    assert!(setup_output.contains("\u{1b}[?1002h"));
    assert!(setup_output.contains("\u{1b}[?1006h"));
    assert!(!setup_output.contains("\u{1b}[?1003h"));
    assert!(!setup_output.contains("\u{1b}[>"));

    apply_tui_terminal_teardown(&mut teardown_output).unwrap();
    let teardown_output = String::from_utf8(teardown_output).unwrap();
    assert!(teardown_output.contains("\u{1b}[?1006l"));
    assert!(teardown_output.contains("\u{1b}[?1003l"));
    assert!(teardown_output.contains("\u{1b}[?1002l"));
    assert!(teardown_output.contains("\u{1b}[?1000l"));
    assert!(!teardown_output.contains("\u{1b}[>"));
}

#[test]
fn tui_ctrl_c_without_selection_keeps_interrupt_semantics() {
    let mut state = test_tui_state();
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    handle_tui_key(
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &mut state,
        &progress_tx,
        &done_tx,
    )
    .unwrap();

    assert!(state.exit_requested);
}

#[test]
fn tui_clipboard_copies_side_question_selected_text() {
    let mut state = test_tui_state();
    state.side_question_prompt = Some(SideQuestionPrompt {
        id: "q1".to_string(),
        question: "Need detail?".to_string(),
        input: MessageBox::new(),
    });
    let prompt = state.side_question_prompt.as_mut().unwrap();
    prompt.input.set_buffer("answer draft".to_string());
    for _ in 0..5 {
        prompt
            .input
            .handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    }
    let mut output = Vec::new();

    assert!(clipboard::handle_clipboard_key(
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &mut state,
        &mut output,
    )
    .unwrap());

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\u{1b}]52;c;ZHJhZnQ=\u{7}"
    );
    assert_eq!(
        state.last_event,
        "copied selected text to clipboard (5 chars)"
    );
}

#[test]
fn tui_mouse_drag_selects_message_box_text_for_clipboard() {
    let mut state = test_tui_state();
    state.input.set_buffer("hello world".to_string());
    let area = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 24,
    };
    let layout = chat_ui_layout(area);
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();
    let start_column = layout.input.x + 1;
    let row = layout.input.y + 1;

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: start_column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: start_column + 5,
            row,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: start_column + 5,
            row,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    let mut output = Vec::new();

    assert!(clipboard::handle_clipboard_key(
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &mut state,
        &mut output,
    )
    .unwrap());
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\u{1b}]52;c;aGVsbG8=\u{7}"
    );
}

#[test]
fn transcript_format_respects_scroll_offset() {
    let chat = (0..6)
        .map(|index| ChatLine {
            role: "deepcli".to_string(),
            content: format!("message-{index}"),
        })
        .collect::<Vec<_>>();

    let latest = format_transcript_text(&chat, 0, 3);
    assert!(!latest.contains("message-0"));
    assert!(latest.contains("message-3"));
    assert!(latest.contains("message-5"));

    let older = format_transcript_text(&chat, 2, 3);
    assert!(older.contains("message-1"));
    assert!(older.contains("message-3"));
    assert!(!older.contains("message-5"));
}

#[test]
fn transcript_render_keeps_latest_message_visible_after_long_output() {
    let backend = TestBackend::new(96, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = test_tui_state();
    let long_output = (0..24)
        .map(|index| format!("older-output-line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    state.chat = vec![
        ChatLine {
            role: "deepcli".to_string(),
            content: long_output,
        },
        ChatLine {
            role: "你".to_string(),
            content: "LATEST_USER_PROMPT_VISIBLE".to_string(),
        },
    ];

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("LATEST_USER_PROMPT_VISIBLE"));
}

#[test]
fn transcript_render_scrolls_within_long_single_message() {
    let backend = TestBackend::new(96, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = test_tui_state();
    state.chat = vec![ChatLine {
        role: "deepcli".to_string(),
        content: (0..36)
            .map(|index| format!("long-message-line-{index}"))
            .collect::<Vec<_>>()
            .join("\n"),
    }];

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let latest = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(latest.contains("long-message-line-35"));
    assert!(!latest.contains("long-message-line-0"));

    state.transcript_scroll = 8;
    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let scrolled = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(scrolled.contains("long-message-line-14"));
    assert!(!scrolled.contains("long-message-line-35"));
}

#[test]
fn chat_ui_layout_reserves_no_status_or_task_monitor_rows() {
    let area = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 32,
    };
    let layout = chat_ui_layout(area);

    assert_eq!(layout.header.height, 0);
    assert_eq!(layout.tools.height, 0);
    assert_eq!(layout.transcript.y, 0);
    assert_eq!(layout.transcript.height, 27);
    assert_eq!(layout.input.y, 27);
    assert_eq!(layout.input.height, 5);
}

#[test]
fn chat_ui_render_omits_status_and_task_monitor_panels() {
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = test_tui_state();
    state.chat.push(ChatLine {
        role: "deepcli".to_string(),
        content: "VISIBLE_TRANSCRIPT".to_string(),
    });

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Messages"));
    assert!(rendered.contains("Message Box"));
    assert!(rendered.contains("VISIBLE_TRANSCRIPT"));
    assert!(!rendered.contains("Status"));
    assert!(!rendered.contains("Task Monitor"));
    assert!(!rendered.contains("provider=deepseek"));
}

#[test]
fn running_prompt_submission_echoes_user_text_before_defer_notice() {
    let mut state = test_tui_state();
    state.running = true;
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    submit_tui_input(
        &mut state,
        "为我检查一下当前项目有没有什么问题".to_string(),
        progress_tx,
        done_tx,
    );

    assert_eq!(state.chat[0].role, "你");
    assert_eq!(state.chat[0].content, "为我检查一下当前项目有没有什么问题");
    assert_eq!(state.chat[1].role, "deepcli");
    assert_eq!(state.last_event, "input deferred while running");
}

#[test]
fn worker_done_returns_transcript_to_latest_output() {
    let dir = tempdir().unwrap();
    let runtime = AgentRuntime::new(
        AppConfig::default(),
        RuntimeOptions {
            workspace: dir.path().to_path_buf(),
            provider: None,
            model: None,
            assume_yes: true,
            resume_session: None,
            stream_output: false,
        },
    )
    .unwrap();
    let mut state = test_tui_state();
    state.running = true;
    state.transcript_scroll = TRANSCRIPT_SCROLL_STEP;
    let (done_tx, done_rx) = mpsc::channel();
    done_tx
        .send(WorkerDone {
            runtime,
            result: Ok("latest output".to_string()),
        })
        .unwrap();

    drain_done(&mut state, &done_rx);

    assert_eq!(state.transcript_scroll, 0);
    assert_eq!(state.chat.last().unwrap().content, "latest output");
}

#[test]
fn transcript_scroll_keys_move_history_window() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: (0..40)
            .map(|index| ChatLine {
                role: "deepcli".to_string(),
                content: format!("message-{index}"),
            })
            .collect(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
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
    };

    assert!(handle_transcript_scroll_key(
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.transcript_scroll, TRANSCRIPT_SCROLL_STEP);
    assert!(state.last_event.contains("messages scrolled back"));

    assert!(handle_transcript_scroll_key(
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.transcript_scroll, 0);

    assert!(handle_transcript_scroll_key(
        KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL),
        &mut state
    ));
    assert_eq!(state.transcript_scroll, state.chat.len() - 1);

    assert!(handle_transcript_scroll_key(
        KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL),
        &mut state
    ));
    assert_eq!(state.transcript_scroll, 0);
}

#[test]
fn transcript_mouse_wheel_scrolls_messages_and_unhandled_tool_area() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: (0..40)
            .map(|index| ChatLine {
                role: "deepcli".to_string(),
                content: format!("message-{index}"),
            })
            .collect(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
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
    };
    let area = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 30,
    };
    let layout = chat_ui_layout(area);
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: layout.transcript.x + 1,
            row: layout.transcript.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    assert_eq!(state.transcript_scroll, TRANSCRIPT_MOUSE_SCROLL_STEP);

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: layout.transcript.x + 1,
            row: layout.transcript.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    assert_eq!(state.transcript_scroll, 0);

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: layout.input.x + 1,
            row: layout.input.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    assert_eq!(state.transcript_scroll, TRANSCRIPT_MOUSE_SCROLL_STEP);
}

#[test]
fn overview_mouse_wheel_falls_back_to_transcript_scroll() {
    let mut state = test_tui_state();
    state.chat = (0..40)
        .map(|index| ChatLine {
            role: "deepcli".to_string(),
            content: format!("message-{index}"),
        })
        .collect();
    state.monitor_tab = MonitorTab::Overview;
    let area = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 30,
    };
    let layout = chat_ui_layout(area);
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: layout.input.x + 1,
            row: layout.input.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );

    assert_eq!(state.transcript_scroll, TRANSCRIPT_MOUSE_SCROLL_STEP);

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: layout.input.x + 1,
            row: layout.input.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );

    assert_eq!(state.transcript_scroll, 0);
}

#[test]
fn transcript_mouse_wheel_clamps_to_renderable_history() {
    let mut state = test_tui_state();
    state.chat = vec![ChatLine {
        role: "deepcli".to_string(),
        content: (0..36)
            .map(|index| format!("long-message-line-{index}"))
            .collect::<Vec<_>>()
            .join("\n"),
    }];
    let area = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 24,
    };
    let layout = chat_ui_layout(area);
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    for _ in 0..10 {
        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: layout.transcript.x + 1,
                row: layout.transcript.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
    }

    assert!(state.transcript_scroll > TRANSCRIPT_MOUSE_SCROLL_STEP);
    let max_scroll = state.transcript_scroll;

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: layout.transcript.x + 1,
            row: layout.transcript.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );

    assert_eq!(
        state.transcript_scroll,
        max_scroll.saturating_sub(TRANSCRIPT_MOUSE_SCROLL_STEP)
    );
}

#[test]
fn result_scroll_keys_move_output_window() {
    let content = (0..9)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: vec![ChatLine {
            role: "deepcli".to_string(),
            content,
        }],
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Result,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    let latest = format_task_monitor_text(&state, None, 12);
    assert!(latest.contains("line-8"));
    assert!(!latest.lines().any(|line| line == "  line-0"));

    assert!(handle_result_scroll_key(
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.result_scroll, RESULT_SCROLL_STEP);
    assert!(state.last_event.contains("result output scrolled back"));
    let scrolled = format_task_monitor_text(&state, None, 12);
    assert!(scrolled.contains("above=3 below=4"));
    assert!(scrolled.contains("line-3"));
    assert!(scrolled.contains("line-4"));
    assert!(!scrolled.lines().any(|line| line == "  line-0"));

    assert!(handle_result_scroll_key(
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.result_scroll, 0);
    assert_eq!(state.last_event, "result output at latest");
}

#[test]
fn result_mouse_wheel_scrolls_result_tab_tools_area_only() {
    let content = (0..8)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: vec![ChatLine {
            role: "deepcli".to_string(),
            content,
        }],
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Result,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let tools_area = Rect {
        x: 0,
        y: 3,
        width: 100,
        height: 9,
    };

    assert!(handle_tools_scroll_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: tools_area.x + 1,
            row: tools_area.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        tools_area,
        true
    ));
    assert_eq!(state.result_scroll, RESULT_MOUSE_SCROLL_STEP);
    assert_eq!(state.transcript_scroll, 0);

    assert!(handle_tools_scroll_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: tools_area.x + 1,
            row: tools_area.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        tools_area,
        false
    ));
    assert_eq!(state.result_scroll, 0);
}

#[test]
fn changes_mouse_wheel_scrolls_selected_patch_in_tools_area() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: Some(WorkspaceChangesSnapshot {
            available: true,
            detail: None,
            changed: 1,
            staged: 0,
            unstaged: 1,
            untracked: 0,
            paths: vec!["src/ui.rs".to_string()],
            diff_preview: Vec::new(),
            diff_preview_truncated: false,
            diff_sections: vec![WorkspaceDiffSection {
                label: "unstaged".to_string(),
                path: "src/ui.rs".to_string(),
                lines: (0..20).map(|index| format!("ui-line-{index}")).collect(),
                truncated: false,
            }],
        }),
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Changes,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let tools_area = Rect {
        x: 0,
        y: 3,
        width: 100,
        height: 9,
    };

    assert!(handle_tools_scroll_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: tools_area.x + 1,
            row: tools_area.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        tools_area,
        false
    ));
    assert_eq!(state.change_patch_scroll, CHANGE_PATCH_MOUSE_SCROLL_STEP);
    assert_eq!(state.result_scroll, 0);

    assert!(handle_tools_scroll_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: tools_area.x + 1,
            row: tools_area.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        tools_area,
        true
    ));
    assert_eq!(state.change_patch_scroll, 0);
}

#[test]
fn changes_mouse_click_selects_patch_from_worktree_file_list() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: Some(WorkspaceChangesSnapshot {
            available: true,
            detail: None,
            changed: 3,
            staged: 1,
            unstaged: 1,
            untracked: 1,
            paths: vec![
                "src/lib.rs".to_string(),
                "src/ui.rs".to_string(),
                "notes.md".to_string(),
            ],
            diff_preview: Vec::new(),
            diff_preview_truncated: false,
            diff_sections: vec![
                WorkspaceDiffSection {
                    label: "unstaged".to_string(),
                    path: "src/lib.rs".to_string(),
                    lines: vec!["diff --git a/src/lib.rs b/src/lib.rs".to_string()],
                    truncated: false,
                },
                WorkspaceDiffSection {
                    label: "staged".to_string(),
                    path: "src/ui.rs".to_string(),
                    lines: vec!["diff --git a/src/ui.rs b/src/ui.rs".to_string()],
                    truncated: false,
                },
            ],
        }),
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 3,
        monitor_tab: MonitorTab::Changes,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let tools_area = Rect {
        x: 0,
        y: 3,
        width: 100,
        height: 12,
    };

    assert!(select_change_patch_at_row(
        &mut state,
        tools_area,
        tools_area.x + 2,
        tools_area.y + 1 + 4,
    ));
    assert_eq!(state.selected_change, 1);
    assert_eq!(state.change_patch_scroll, 0);
    assert!(state.last_event.contains("src/ui.rs"));
    state.workspace_changes.as_mut().unwrap().paths =
        vec!["src/lib.rs".to_string(), "notes.md".to_string()];

    assert!(select_change_patch_at_row(
        &mut state,
        tools_area,
        tools_area.x + 2,
        tools_area.y + 1 + 4,
    ));
    assert_eq!(state.selected_change, 1);
    assert!(
        state.last_event.contains("no patch for notes.md"),
        "last_event={}",
        state.last_event
    );
}

#[test]
fn message_box_render_places_terminal_cursor_at_input_cursor() {
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut input = MessageBox::new();
    for ch in "abc".chars() {
        input.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    input.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    let state = TuiState {
        runtime: None,
        active_session: None,
        input,
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
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
    };
    let input_area = chat_ui_layout(Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 24,
    })
    .input;
    let expected =
        message_box_cursor_position(state.input.buffer(), state.input.cursor(), input_area);

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    terminal.backend_mut().assert_cursor_position(expected);
}

#[test]
fn message_box_render_scrolls_to_cursor_for_long_input() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = test_tui_state();
    state
        .input
        .insert_str("line-0\nline-1\nline-2\nTAIL_VISIBLE_AT_CURSOR");

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("TAIL_VISIBLE_AT_CURSOR"));
}

#[test]
fn tui_loop_batches_pending_terminal_events_before_redraw() {
    let source =
        fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui.rs"))
            .unwrap();

    assert!(source.contains("TUI_EVENT_BATCH_LIMIT"));
    assert!(source.contains("processed_events < TUI_EVENT_BATCH_LIMIT"));
    assert!(source.contains("event::poll(Duration::ZERO)"));
}

#[test]
fn chat_view_visible_render_avoids_full_transcript_formatting_on_input_frames() {
    let source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/chat_view.rs"),
    )
    .unwrap();
    let visible_start = source
        .find("fn format_visible_transcript_text")
        .expect("visible transcript formatter exists");
    let title_start = source
        .find("fn format_messages_title")
        .expect("messages title formatter exists");
    let visible_body = &source[visible_start..title_start];

    assert!(!visible_body.contains("format_full_transcript_text("));
}

#[test]
fn tui_credentials_set_uses_hidden_prompt_path() {
    let spec = parse_tui_credential_set("/credentials set deepseek --force")
        .unwrap()
        .unwrap();
    assert_eq!(spec.provider, "deepseek");
    assert!(spec.force);

    let error = parse_tui_credential_set("/credentials set deepseek --stdin")
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("隐藏输入框"));
    assert!(parse_tui_credential_set("/credentials status").is_none());
}

#[test]
fn slash_command_palette_marks_only_supported_running_commands() {
    for command in [
        "/version",
        "/quickstart",
        "/compiler",
        "/accept",
        "/gate",
        "/verify",
        "/handoff",
    ] {
        let query = &command[..command.len().min(4)];
        let suggestions = slash_command_suggestions_for_state(query, true).unwrap();
        let summary = suggestions
            .iter()
            .find(|summary| summary.name == command)
            .unwrap_or_else(|| panic!("{command} should stay discoverable in running mode"));
        assert!(
            !summary.running_safe,
            "{command} is not handled by the running TUI command dispatcher"
        );
    }

    for command in [
        "/help",
        "/recipes",
        "/scorecard",
        "/opportunities",
        "/benchmark",
        "/round",
        "/selftest",
        "/preflight",
        "/completion",
        "/status",
        "/usage",
        "/trace",
        "/logs",
        "/privacy",
        "/fork",
        "/approval",
        "/session",
        "/cleanup",
        "/btw",
        "/git",
        "/stop",
        "/quit",
        "/terminal",
    ] {
        let query = &command[..command.len().min(4)];
        let suggestions = slash_command_suggestions_for_state(query, true).unwrap();
        let summary = suggestions
            .iter()
            .find(|summary| summary.name == command)
            .unwrap_or_else(|| panic!("{command} should be discoverable in running mode"));
        assert!(
            summary.running_safe,
            "{command} should be marked as supported by the running TUI command dispatcher"
        );
    }
}

#[test]
fn running_safe_palette_priority_covers_registry_running_safe_commands() {
    let unsupported_priority = running_safe_palette_priority("/version");
    assert_eq!(unsupported_priority, usize::MAX);
    for summary in CommandRouter::help_summaries()
        .into_iter()
        .filter(|summary| summary.running_safe)
    {
        assert!(
            running_safe_palette_priority(summary.name) < unsupported_priority,
            "{} is marked running-safe but has no explicit running palette priority",
            summary.name
        );
    }
}

#[test]
fn running_tui_unsupported_hint_covers_registry_running_safe_commands() {
    let hint = running_tui_supported_command_hint();
    for summary in CommandRouter::help_summaries()
        .into_iter()
        .filter(|summary| summary.running_safe)
    {
        assert!(
            hint.contains(summary.name),
            "{} is marked running-safe but missing from the running TUI hint: {hint}",
            summary.name
        );
    }
}

#[test]
fn slash_command_palette_filters_formats_and_completes() {
    let usage_suggestions = slash_command_suggestions_for_state("/us", false).unwrap();
    assert_eq!(usage_suggestions[0].name, "/usage");
    assert!(usage_suggestions[0].running_safe);
    let usage_text = format_command_palette_text(&usage_suggestions, 0, 20, false);
    assert!(usage_text.contains(">/usage (run)"));
    assert!(usage_text.contains("running-safe: yes"));

    let running_suggestions = slash_command_suggestions_for_state("/", true).unwrap();
    assert_eq!(running_suggestions[0].name, "/help");
    assert!(running_suggestions
        .iter()
        .any(|summary| summary.name == "/status" && summary.running_safe));
    assert!(running_suggestions
        .iter()
        .any(|summary| summary.name == "/usage" && summary.running_safe));
    assert_eq!(
        running_suggestions
            .iter()
            .take_while(|summary| summary.running_safe)
            .count(),
        COMMAND_PALETTE_MATCH_LIMIT
    );
    assert!(running_suggestions[0..COMMAND_PALETTE_MATCH_LIMIT]
        .iter()
        .all(|summary| summary.running_safe));
    assert!(running_suggestions
        .iter()
        .any(|summary| summary.name == "/privacy" && summary.running_safe));
    assert!(running_suggestions
        .iter()
        .any(|summary| summary.name == "/logs" && summary.running_safe));
    assert!(running_suggestions
        .iter()
        .any(|summary| summary.name == "/round" && summary.running_safe));
    assert!(running_suggestions
        .iter()
        .any(|summary| summary.name == "/opportunities" && summary.running_safe));
    assert!(running_suggestions
        .iter()
        .any(|summary| summary.name == "/selftest" && summary.running_safe));
    assert!(running_suggestions
        .iter()
        .any(|summary| summary.name == "/completion" && summary.running_safe));
    let running_text = format_command_palette_text(&running_suggestions, 0, 20, true);
    assert!(running_text.contains("running mode: (run) commands execute now"));
    let version_suggestions = slash_command_suggestions_for_state("/ver", true).unwrap();
    assert_eq!(version_suggestions[0].name, "/version");
    assert!(!version_suggestions[0].running_safe);
    let approval_suggestions = slash_command_suggestions_for_state("/app", true).unwrap();
    assert!(approval_suggestions
        .iter()
        .any(|summary| summary.name == "/approval" && summary.running_safe));
    let handoff_suggestions = slash_command_suggestions_for_state("/han", true).unwrap();
    assert!(handoff_suggestions
        .iter()
        .any(|summary| summary.name == "/handoff" && !summary.running_safe));

    let compiler_suggestions = slash_command_suggestions_for_state("/com", true).unwrap();
    assert!(compiler_suggestions
        .iter()
        .any(|summary| summary.name == "/compiler" && !summary.running_safe));
    let model_suggestions = slash_command_suggestions_for_state("/mo", false).unwrap();
    assert!(model_suggestions
        .iter()
        .any(|summary| summary.name == "/model"));
    let session_suggestions = slash_command_suggestions_for_state("/se", true).unwrap();
    assert!(session_suggestions
        .iter()
        .any(|summary| summary.name == "/session" && summary.running_safe));
    let btw_suggestions = slash_command_suggestions_for_state("/bt", true).unwrap();
    assert!(btw_suggestions
        .iter()
        .any(|summary| summary.name == "/btw" && summary.running_safe));
    let terminal_suggestions = slash_command_suggestions_for_state("/ter", true).unwrap();
    assert_eq!(terminal_suggestions[0].name, "/terminal");
    assert!(terminal_suggestions[0].running_safe);
    let stop_suggestions = slash_command_suggestions_for_state("/st", true).unwrap();
    assert!(stop_suggestions
        .iter()
        .any(|summary| summary.name == "/stop"));
    let exact_stop_suggestions = slash_command_suggestions_for_state("/stop", true).unwrap();
    assert_eq!(exact_stop_suggestions[0].name, "/stop");
    let quit_suggestions = slash_command_suggestions_for_state("/qu", true).unwrap();
    assert!(quit_suggestions
        .iter()
        .any(|summary| summary.name == "/quit"));
    let exact_quit_suggestions = slash_command_suggestions_for_state("/quit", true).unwrap();
    assert_eq!(exact_quit_suggestions[0].name, "/quit");

    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
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
    };
    let suggestions = slash_command_suggestions_for_state("/compi", false).unwrap();
    state.input.set_buffer("/compi".to_string());
    complete_selected_command(&mut state, &suggestions);
    assert_eq!(state.input.buffer(), "/compiler ");
    assert_eq!(state.last_event, "completed /compiler");
}

#[test]
fn slash_command_input_mouse_events_do_not_open_or_complete_palette() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: vec![ChatLine {
            role: "deepcli".to_string(),
            content: (0..16)
                .map(|index| format!("line-{index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }],
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Result,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    state.input.set_buffer("/".to_string());
    state.transcript_scroll = TRANSCRIPT_MOUSE_SCROLL_STEP;
    let area = Rect {
        x: 0,
        y: 0,
        width: 140,
        height: 24,
    };
    let layout = chat_ui_layout(area);
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: layout.input.x + 2,
            row: layout.input.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    assert_eq!(state.selected_command, 0);
    assert_eq!(state.input.buffer(), "/");
    assert_eq!(state.result_scroll, 0);
    assert_eq!(state.transcript_scroll, 0);

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: layout.input.x + 2,
            row: layout.input.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    assert_eq!(state.input.buffer(), "/");
    assert_eq!(state.selected_command, 0);
    assert_ne!(state.last_event, "completed /help");
}

#[test]
fn slash_command_input_stays_in_message_box_without_auto_popup() {
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: vec![ToolLogItem {
            title: "tool: read_file".to_string(),
            detail: "details".to_string(),
            expanded: false,
        }],
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: Some(0),
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
    };
    state.input.set_buffer("/doctor".to_string());

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Message Box"));
    assert!(rendered.contains("/doctor"));
    assert!(!rendered.contains("Command Help"));
    assert!(!rendered.contains("matches:"));
    assert!(!rendered.contains("tool: read_file"));
}

#[test]
fn task_overview_formats_plan_tests_and_blockers() {
    let state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: vec![ChatLine {
            role: "deepcli".to_string(),
            content: "verify complete\nall checks passed".to_string(),
        }],
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: vec![
            ToolLogItem {
                title: "tool: read_file".to_string(),
                detail: "done".to_string(),
                expanded: false,
            },
            ToolLogItem {
                title: "tool: run_tests [failed]".to_string(),
                detail: "tests failed".to_string(),
                expanded: false,
            },
        ],
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "deepcli: tool run_tests failed".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let observation = SessionObservation {
        state: "Testing".to_string(),
        plan_total: 4,
        plan_completed: 2,
        plan_in_progress: 1,
        plan_failed: 1,
        current_step: Some("repair failing compiler tests".to_string()),
        latest_test: Some(SessionObservationTest {
            command: "cargo test --all-targets".to_string(),
            passed: false,
            exit_code: Some(101),
        }),
        pending_approvals: 1,
        open_questions: 2,
        tool_calls: 3,
        failed_tools: 1,
    };

    let actions = monitor_quick_actions_for_tab(&state, None);
    let overview = format_task_overview_lines(&state, Some(&observation), &actions, 0).join("\n");
    assert!(overview.contains("state=Testing ui=running"));
    assert!(overview.contains("plan=2/4 running=1 failed=1"));
    assert!(overview.contains("approvals=1 btw=2"));
    assert!(overview.contains("current=repair failing compiler tests"));
    assert!(overview.contains("test=fail code=101 cargo test --all-targets"));
    assert!(overview.contains("tools=3 failed_tools=1"));
    assert!(overview.contains("last output: ok verify complete"));
    assert!(overview.contains("> /status"));
    assert!(!overview.contains("/status --json"));
}

#[test]
fn tool_started_progress_surfaces_command_detail() {
    let mut state = test_tui_state();
    let (progress_tx, progress_rx) = mpsc::channel();

    progress_tx
        .send(RuntimeProgress::ToolStarted {
            tool: "run_tests".to_string(),
            detail: Some("cargo test 2>&1".to_string()),
        })
        .unwrap();
    drain_progress(&mut state, &progress_rx);

    assert!(state.last_event.contains("run_tests"));
    assert!(state.last_event.contains("cargo test 2>&1"));
    assert_eq!(state.tool_log.len(), 1);
    assert_eq!(state.tool_log[0].title, "tool: run_tests");
    assert!(state.tool_log[0].detail.contains("cargo test 2>&1"));
    assert_eq!(state.chat.len(), 1);
    assert_eq!(state.chat[0].role, "deepcli");
    assert!(state.chat[0].content.contains("running tool run_tests"));
    assert!(state.chat[0].content.contains("cargo test 2>&1"));
}

#[test]
fn assistant_delta_progress_updates_messages_immediately() {
    let mut state = test_tui_state();
    let (progress_tx, progress_rx) = mpsc::channel();

    progress_tx
        .send(RuntimeProgress::AssistantDelta {
            delta: "正在检查".to_string(),
        })
        .unwrap();
    drain_progress(&mut state, &progress_rx);

    assert_eq!(state.chat.len(), 1);
    assert_eq!(state.chat[0].role, "deepcli");
    assert_eq!(state.chat[0].content, "正在检查");
    assert_eq!(state.transcript_scroll, 0);

    progress_tx
        .send(RuntimeProgress::AssistantDelta {
            delta: "当前项目".to_string(),
        })
        .unwrap();
    drain_progress(&mut state, &progress_rx);

    assert_eq!(state.chat.len(), 1);
    assert_eq!(state.chat[0].content, "正在检查当前项目");
}

#[test]
fn worker_done_reuses_streamed_assistant_message() {
    let dir = tempdir().unwrap();
    let runtime = AgentRuntime::new(
        AppConfig::default(),
        RuntimeOptions {
            workspace: dir.path().to_path_buf(),
            provider: None,
            model: None,
            assume_yes: true,
            resume_session: None,
            stream_output: false,
        },
    )
    .unwrap();
    let mut state = test_tui_state();
    state.running = true;
    let (progress_tx, progress_rx) = mpsc::channel();
    progress_tx
        .send(RuntimeProgress::AssistantDelta {
            delta: "partial".to_string(),
        })
        .unwrap();
    drain_progress(&mut state, &progress_rx);
    let (done_tx, done_rx) = mpsc::channel();
    done_tx
        .send(WorkerDone {
            runtime,
            result: Ok("partial final usage".to_string()),
        })
        .unwrap();

    drain_done(&mut state, &done_rx);

    assert_eq!(state.chat.len(), 1);
    assert_eq!(state.chat[0].role, "deepcli");
    assert_eq!(state.chat[0].content, "partial final usage");
}

#[test]
fn running_overview_quick_actions_use_running_safe_commands() {
    let dir = tempdir().unwrap();
    let session = SessionStore::new(dir.path())
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    let mut state = test_tui_state();
    state.active_session = Some(ActiveSessionRef {
        workspace: dir.path().to_path_buf(),
        session_id: session.id().to_string(),
    });
    state.running = true;
    state.monitor_tab = MonitorTab::Overview;
    let actions = monitor_quick_actions_for_tab(&state, None);
    let commands = actions
        .iter()
        .map(|action| action.command.as_str())
        .collect::<Vec<_>>();

    assert_eq!(commands[0], "/status");
    assert!(!commands.contains(&"/status --json"));
    assert!(!commands.contains(&"/next --json"));

    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();
    assert!(handle_monitor_quick_action_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
        &progress_tx,
        &done_tx
    ));
    assert_eq!(state.chat.last().unwrap().role, "deepcli");
    assert!(state.last_event.starts_with("running command ok"));
}

#[test]
fn task_monitor_tabs_format_usage_tests_environment_approvals_and_trace() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Tests,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let monitor = SessionMonitor {
        observation: SessionObservation {
            state: "Testing".to_string(),
            plan_total: 1,
            plan_completed: 0,
            plan_in_progress: 1,
            plan_failed: 0,
            current_step: Some("verify".to_string()),
            latest_test: None,
            pending_approvals: 1,
            open_questions: 1,
            tool_calls: 0,
            failed_tools: 0,
        },
        usage: SessionObservationUsage {
            provider_turns_started: 2,
            provider_turns_completed: 1,
            provider_average_elapsed_ms: Some(45_000),
            provider_max_elapsed_ms: Some(45_000),
            provider_tool_calls: 3,
            compacted_turns: 1,
            prompt_tokens: Some(100),
            completion_tokens: Some(20),
            total_tokens: Some(120),
            prompt_cache_hit_tokens: Some(10),
            prompt_cache_miss_tokens: Some(90),
            latest_request_bytes: Some(700_000),
            max_request_bytes: Some(700_000),
        },
        recent_tests: vec![SessionObservationTest {
            command: "cargo test".to_string(),
            passed: true,
            exit_code: Some(0),
        }],
        recent_environment: vec![SessionObservationEnvironment {
            tool: "check_environment".to_string(),
            target: "docker".to_string(),
            status: "needs_setup".to_string(),
            ready: Some(false),
            detail: "recommended: /install docker --smoke".to_string(),
        }],
        pending_approvals: vec![SessionObservationApproval {
            id: "12345678-aaaa-bbbb-cccc-123456789abc".to_string(),
            tool: "write_file".to_string(),
            risk: "Medium".to_string(),
            reason: "write requires approval".to_string(),
        }],
        open_questions: vec![SessionObservationQuestion {
            id: "87654321-aaaa-bbbb-cccc-123456789abc".to_string(),
            question: "switch model?".to_string(),
        }],
        recent_events: vec![SessionObservationEvent {
            event_type: "test_run".to_string(),
            created_at: "10:11:12".to_string(),
        }],
    };

    let tests = format_task_monitor_text(&state, Some(&monitor), 9);
    assert!(tests.contains("[Tests]"));
    assert!(tests.contains("test=pass code=0 cargo test"));
    assert!(tests.contains("/accept --json"));
    assert!(tests.contains("/gate --json"));

    state.monitor_tab = MonitorTab::Session;
    let session = format_task_monitor_text(&state, Some(&monitor), 12);
    assert!(session.contains("[Session]"));
    assert!(session.contains("session: state=Testing plan=0/1 running=1 failed=0 current=verify"));
    assert!(session.contains("queues: approvals=1 btw=1 tools=0 failed_tools=0"));
    assert!(session.contains("10:11:12 test_run"));
    assert!(session.contains("/goal status --json"));

    state.monitor_tab = MonitorTab::Result;
    state.chat = vec![ChatLine {
        role: "error".to_string(),
        content: "verify failed\nmissing strong test evidence".to_string(),
    }];
    let result = format_task_monitor_text(&state, Some(&monitor), 12);
    assert!(result.contains("[Result]"));
    assert!(result.contains("status: error"));
    assert!(result.contains("summary: verify failed"));
    assert!(result.contains("missing strong test evidence"));
    assert!(result.contains("/session history --limit 5"));
    state.chat.clear();

    state.monitor_tab = MonitorTab::Changes;
    let changes = format_task_monitor_text(&state, Some(&monitor), 9);
    assert!(changes.contains("[Changes]"));
    assert!(changes.contains("changes unavailable: no active session"));
    assert!(changes.contains("/diff --stat"));

    state.monitor_tab = MonitorTab::Usage;
    let usage = format_task_monitor_text(&state, Some(&monitor), 9);
    assert!(usage.contains("[Usage]"));
    assert!(usage.contains("provider turns: started=2 completed=1 avg=45000ms"));
    assert!(usage.contains("tokens: prompt=100 completion=20 total=120"));
    assert!(usage.contains("compacted_turns=1"));
    assert!(usage.contains("hit_rate=10.0%"));
    assert!(usage.contains("/trace --limit 30"));

    state.monitor_tab = MonitorTab::Context;
    let context = format_task_monitor_text(&state, Some(&monitor), 12);
    assert!(context.contains("[Context]"));
    assert!(context.contains("context cache: hit=10 miss=90 hit_rate=10.0%"));
    assert!(context.contains("request: latest=684KiB max=684KiB compacted_turns=1"));
    assert!(context.contains("environment: check_environment target=docker status=needs_setup"));
    assert!(context.contains("/context"));

    state.monitor_tab = MonitorTab::Environment;
    let environment = format_task_monitor_text(&state, Some(&monitor), 14);
    assert!(environment.contains("[Environment]"));
    assert!(environment.contains("check_environment target=docker status=needs_setup"));
    assert!(environment.contains("/doctor docker --json"));
    assert!(environment.contains("/install docker --smoke (edit)"));
    assert!(!environment.contains("/env plan docker"));
    assert!(!environment.contains("/env test docker"));
    assert!(environment.contains("/accept --env-check docker --json"));
    assert!(environment.contains("/gate --env-check docker --json"));

    state.monitor_tab = MonitorTab::Deliver;
    let deliver = format_task_monitor_text(&state, Some(&monitor), 16);
    assert!(deliver.contains("[Deliver]"));
    assert!(deliver.contains("plan: pending 0/1 running=1"));
    assert!(deliver.contains("tests: ok cargo test"));
    assert!(deliver.contains("environment: needs_setup target=docker"));
    assert!(deliver.contains("blockers: approvals=1 btw=1 failed_tools=0"));
    assert!(deliver.contains("/accept --env-check docker --json"));
    assert!(deliver.contains("/gate --env-check docker --json"));
    assert!(deliver.contains("/handoff --env-check docker --format pr"));

    state.monitor_tab = MonitorTab::Approvals;
    let approvals = format_task_monitor_text(&state, Some(&monitor), 9);
    assert!(approvals.contains("[Approvals]"));
    assert!(approvals.contains("pending approvals: 1"));
    assert!(approvals.contains("12345678 write_file risk=Medium"));
    assert!(approvals.contains("open btw questions: 1"));
    assert!(approvals.contains("87654321 switch model?"));

    state.monitor_tab = MonitorTab::Trace;
    let trace = format_task_monitor_text(&state, Some(&monitor), 9);
    assert!(trace.contains("[Trace]"));
    assert!(trace.contains("10:11:12 test_run"));
}

#[test]
fn changes_tab_surfaces_session_diff_records_and_actions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session
            .save_diff(
                "src/lib.rs",
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,2 @@\n-old\n+new\n+extra\n",
            )
            .unwrap();
    session
            .save_diff(
                "src/ui.rs",
                "diff --git a/src/ui.rs b/src/ui.rs\n--- a/src/ui.rs\n+++ b/src/ui.rs\n@@ -10,0 +11 @@\n+changes tab\n",
            )
            .unwrap();

    let state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: Some(WorkspaceChangesSnapshot {
            available: true,
            detail: None,
            changed: 3,
            staged: 1,
            unstaged: 1,
            untracked: 1,
            paths: vec![
                "src/lib.rs".to_string(),
                "src/ui.rs".to_string(),
                "notes.md".to_string(),
            ],
            diff_preview: vec![
                "unstaged diff:".to_string(),
                "diff --git a/src/lib.rs b/src/lib.rs".to_string(),
                "-old".to_string(),
                "+new".to_string(),
            ],
            diff_preview_truncated: false,
            diff_sections: vec![
                WorkspaceDiffSection {
                    label: "unstaged".to_string(),
                    path: "src/lib.rs".to_string(),
                    lines: vec![
                        "diff --git a/src/lib.rs b/src/lib.rs".to_string(),
                        "-old".to_string(),
                        "+new".to_string(),
                    ],
                    truncated: false,
                },
                WorkspaceDiffSection {
                    label: "staged".to_string(),
                    path: "src/ui.rs".to_string(),
                    lines: vec![
                        "diff --git a/src/ui.rs b/src/ui.rs".to_string(),
                        "+changes tab".to_string(),
                    ],
                    truncated: false,
                },
            ],
        }),
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Changes,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    let changes = format_task_monitor_text(&state, None, 32);
    assert!(changes.contains("[Changes]"));
    assert!(changes.contains("worktree: dirty changed=3 staged=1 unstaged=1 untracked=1"));
    assert!(changes.contains("notes.md"));
    assert!(changes.contains("selected patch: 1/2 unstaged src/lib.rs"));
    assert!(changes.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    assert!(changes.contains("diff_records=2 showing=2"));
    assert!(changes.contains("recent summary: files=2 +3 -1"));
    assert!(changes.contains("src/lib.rs"));
    assert!(changes.contains("src/ui.rs"));
    assert!(changes.contains("/diff --stat"));
    assert!(changes.contains("/review"));
    assert!(changes.contains("/handoff --format pr"));
}

#[test]
fn git_status_snapshot_counts_worktree_states_and_paths() {
    let snapshot = parse_git_status_snapshot(
        " M src/lib.rs\nA  src/main.rs\n?? notes.md\nR  old.rs -> new.rs\n",
    );

    assert!(snapshot.available);
    assert_eq!(snapshot.changed, 4);
    assert_eq!(snapshot.staged, 2);
    assert_eq!(snapshot.unstaged, 1);
    assert_eq!(snapshot.untracked, 1);
    assert_eq!(
        snapshot.paths,
        vec![
            "src/lib.rs".to_string(),
            "src/main.rs".to_string(),
            "notes.md".to_string(),
            "new.rs".to_string(),
        ]
    );

    let clean = parse_git_status_snapshot("");
    assert_eq!(clean.changed, 0);
    assert!(clean.paths.is_empty());
}

#[test]
fn changes_patch_preview_formats_truncation_and_untracked_only() {
    let mut lines = Vec::new();
    append_workspace_changes_lines(
        &mut lines,
        Some(&WorkspaceChangesSnapshot {
            available: true,
            detail: None,
            changed: 1,
            staged: 0,
            unstaged: 0,
            untracked: 1,
            paths: vec!["notes.md".to_string()],
            diff_preview: Vec::new(),
            diff_preview_truncated: false,
            diff_sections: Vec::new(),
        }),
        0,
        0,
    );
    let rendered = lines.join("\n");
    assert!(rendered.contains("worktree patch: none (untracked files only)"));

    let mut lines = Vec::new();
    append_workspace_changes_lines(
        &mut lines,
        Some(&WorkspaceChangesSnapshot {
            available: true,
            detail: None,
            changed: 1,
            staged: 0,
            unstaged: 1,
            untracked: 0,
            paths: vec!["src/lib.rs".to_string()],
            diff_preview: vec![
                "unstaged diff:".to_string(),
                "diff --git a/src/lib.rs b/src/lib.rs".to_string(),
            ],
            diff_preview_truncated: true,
            diff_sections: Vec::new(),
        }),
        0,
        0,
    );
    let rendered = lines.join("\n");
    assert!(rendered.contains("worktree patch preview (truncated):"));
    assert!(rendered.contains("diff --git a/src/lib.rs b/src/lib.rs"));
}

#[test]
fn changes_tab_keys_select_and_scroll_file_patch() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: Some(WorkspaceChangesSnapshot {
            available: true,
            detail: None,
            changed: 2,
            staged: 1,
            unstaged: 1,
            untracked: 0,
            paths: vec!["src/lib.rs".to_string(), "src/ui.rs".to_string()],
            diff_preview: Vec::new(),
            diff_preview_truncated: false,
            diff_sections: vec![
                WorkspaceDiffSection {
                    label: "unstaged".to_string(),
                    path: "src/lib.rs".to_string(),
                    lines: vec![
                        "diff --git a/src/lib.rs b/src/lib.rs".to_string(),
                        "+lib".to_string(),
                    ],
                    truncated: false,
                },
                WorkspaceDiffSection {
                    label: "staged".to_string(),
                    path: "src/ui.rs".to_string(),
                    lines: (0..24).map(|index| format!("ui-line-{index}")).collect(),
                    truncated: false,
                },
            ],
        }),
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Changes,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_changes_tab_key(
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.selected_change, 1);
    assert_eq!(state.change_patch_scroll, 0);
    assert!(state.last_event.contains("src/ui.rs"));

    assert!(handle_changes_tab_key(
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.change_patch_scroll, CHANGE_PATCH_SCROLL_STEP);
    let rendered = format_task_monitor_text(&state, None, 34);
    assert!(rendered.contains("selected patch: 2/2 staged src/ui.rs"));
    assert!(rendered.contains("[above: 8 line(s)]"));
    assert!(rendered.contains("ui-line-8"));

    assert!(handle_changes_tab_key(
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.change_patch_scroll, 0);

    assert!(handle_changes_tab_key(
        KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.selected_change, 0);
}

#[test]
fn diff_dialog_opens_selected_change_patch_and_scrolls() {
    let mut state = test_tui_state();
    state.monitor_tab = MonitorTab::Changes;
    state.workspace_changes = Some(WorkspaceChangesSnapshot {
        available: true,
        detail: None,
        changed: 2,
        staged: 1,
        unstaged: 1,
        untracked: 0,
        paths: vec!["src/lib.rs".to_string(), "src/ui.rs".to_string()],
        diff_preview: Vec::new(),
        diff_preview_truncated: false,
        diff_sections: vec![
            WorkspaceDiffSection {
                label: "unstaged".to_string(),
                path: "src/lib.rs".to_string(),
                lines: (0..20).map(|index| format!("lib-line-{index}")).collect(),
                truncated: false,
            },
            WorkspaceDiffSection {
                label: "staged".to_string(),
                path: "src/ui.rs".to_string(),
                lines: (0..20).map(|index| format!("ui-line-{index}")).collect(),
                truncated: true,
            },
        ],
    });

    assert!(handle_changes_tab_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state
    ));
    assert!(matches!(state.dialog, Some(TuiDialog::Diff(_))));
    let body = dialog_body_for_state(&state, 8).unwrap();
    assert!(body.contains("unstaged src/lib.rs"));
    assert!(body.contains("lib-line-0"));

    handle_dialog_key(
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
        &mut state,
    );
    let switched = dialog_body_for_state(&state, 8).unwrap();
    assert!(switched.contains("staged src/ui.rs"));
    assert!(switched.contains("truncated"));

    handle_dialog_key(
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        &mut state,
    );
    let scrolled = dialog_body_for_state(&state, 8).unwrap();
    assert!(scrolled.contains("[above:"));
}

#[test]
fn diff_sections_split_by_file_and_cap_long_sections() {
    let mut diff =
        String::from("diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n");
    for index in 0..(WORKTREE_DIFF_SECTION_LINES + 5) {
        diff.push_str(&format!("+line-{index}\n"));
    }
    diff.push_str("diff --git a/src/ui.rs b/src/ui.rs\n--- a/src/ui.rs\n+++ b/src/ui.rs\n+ui\n");

    let sections = parse_diff_sections("unstaged", &diff);

    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].label, "unstaged");
    assert_eq!(sections[0].path, "src/lib.rs");
    assert!(sections[0].truncated);
    assert_eq!(sections[0].lines.len(), WORKTREE_DIFF_SECTION_LINES);
    assert_eq!(sections[1].path, "src/ui.rs");
    assert!(!sections[1].truncated);
}

#[test]
fn monitor_tab_cycles_without_touching_message_input() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
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
    };
    state.input.set_buffer("hello".to_string());

    cycle_monitor_tab(&mut state, true);
    assert_eq!(state.monitor_tab, MonitorTab::Changes);
    cycle_monitor_tab(&mut state, true);
    assert_eq!(state.monitor_tab, MonitorTab::Tools);
    cycle_monitor_tab(&mut state, true);
    assert_eq!(state.monitor_tab, MonitorTab::Tests);
    cycle_monitor_tab(&mut state, true);
    assert_eq!(state.monitor_tab, MonitorTab::Session);
    cycle_monitor_tab(&mut state, true);
    assert_eq!(state.monitor_tab, MonitorTab::Approvals);
    cycle_monitor_tab(&mut state, true);
    assert_eq!(state.monitor_tab, MonitorTab::Context);
    cycle_monitor_tab(&mut state, true);
    assert_eq!(state.monitor_tab, MonitorTab::Result);
    cycle_monitor_tab(&mut state, true);
    assert_eq!(state.monitor_tab, MonitorTab::Usage);
    assert_eq!(state.input.buffer(), "hello");
    cycle_monitor_tab(&mut state, false);
    assert_eq!(state.monitor_tab, MonitorTab::Result);
}

#[test]
fn monitor_tabs_lead_with_core_views_then_advanced() {
    let tabs = MonitorTab::all();
    let core: Vec<MonitorTab> = tabs
        .iter()
        .copied()
        .filter(|tab| tab.tier() == MonitorTier::Core)
        .collect();
    assert_eq!(
        core,
        vec![
            MonitorTab::Overview,
            MonitorTab::Changes,
            MonitorTab::Tools,
            MonitorTab::Tests,
            MonitorTab::Session,
            MonitorTab::Approvals,
            MonitorTab::Context,
        ]
    );
    // core tabs occupy the front of the strip with no advanced tab interleaved.
    let first_advanced = tabs
        .iter()
        .position(|tab| tab.tier() == MonitorTier::Advanced)
        .unwrap();
    assert!(tabs[..first_advanced]
        .iter()
        .all(|tab| tab.tier() == MonitorTier::Core));
    assert!(tabs[first_advanced..]
        .iter()
        .all(|tab| tab.tier() == MonitorTier::Advanced));
}

#[test]
fn monitor_tab_metadata_is_projection_source() {
    let metadata = MonitorTab::metadata();
    let tabs = MonitorTab::all();
    assert_eq!(metadata.len(), tabs.len());

    let projected_tabs: Vec<MonitorTab> = metadata.iter().map(|entry| entry.tab).collect();
    assert_eq!(projected_tabs, tabs);

    for entry in metadata {
        assert_eq!(entry.tab.label(), entry.label);
        assert_eq!(entry.tab.tier(), entry.tier);
    }
}

#[test]
fn monitor_static_quick_actions_are_projection_source() {
    let projections = MonitorTab::static_quick_action_metadata();
    let projected_tabs: Vec<MonitorTab> = projections.iter().map(|entry| entry.tab).collect();
    assert_eq!(
        projected_tabs,
        vec![
            MonitorTab::Overview,
            MonitorTab::Result,
            MonitorTab::Changes,
            MonitorTab::Usage,
            MonitorTab::Tests,
            MonitorTab::Session,
            MonitorTab::Approvals,
            MonitorTab::Context,
            MonitorTab::Trace,
        ]
    );

    for dynamic_tab in [
        MonitorTab::Tools,
        MonitorTab::Health,
        MonitorTab::Library,
        MonitorTab::Deliver,
        MonitorTab::Environment,
    ] {
        assert!(dynamic_tab.static_quick_actions().is_none());
    }

    let mut state = test_tui_state();
    for projection in projections {
        state.monitor_tab = projection.tab;
        assert_eq!(
            monitor_quick_actions_for_tab(&state, None),
            projection.actions()
        );
    }
}

#[test]
fn monitor_tab_next_previous_cover_full_cycle_from_ordering() {
    let tabs = MonitorTab::all();
    // next() walks the full ordering and wraps.
    let mut tab = tabs[0];
    for expected in tabs.iter().skip(1).chain(std::iter::once(&tabs[0])) {
        tab = tab.next();
        assert_eq!(tab, *expected);
    }
    // previous() is the exact inverse.
    for window in tabs {
        assert_eq!(window.next().previous(), window);
    }
}

#[test]
fn monitor_tab_strip_collapses_advanced_until_active() {
    // From a core tab the advanced diagnostics are collapsed behind a toggle.
    let collapsed = format_monitor_tabs(MonitorTab::Overview);
    assert!(collapsed.contains("[Overview]"));
    assert!(collapsed.contains(MONITOR_ADVANCED_TOGGLE_LABEL));
    assert!(!collapsed.contains("Result"));
    assert!(!collapsed.contains('|'));

    // Once an advanced tab is active the full advanced group is revealed
    // after the separator, and the toggle disappears.
    let expanded = format_monitor_tabs(MonitorTab::Result);
    assert!(expanded.contains("[Result]"));
    assert!(!expanded.contains(MONITOR_ADVANCED_TOGGLE_LABEL));
    let approvals = expanded.find("Approvals").unwrap();
    let bar = expanded.find('|').unwrap();
    let result = expanded.find("Result").unwrap();
    assert!(approvals < bar && bar < result);
}

#[test]
fn monitor_advanced_toggle_enters_first_advanced_tab() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 3,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let tools_area = Rect {
        x: 0,
        y: 3,
        width: 120,
        height: 9,
    };
    let tabs = format_monitor_tabs(state.monitor_tab);
    let toggle_offset = tabs.find(MONITOR_ADVANCED_TOGGLE_LABEL).unwrap() as u16;

    assert!(select_monitor_tab_at_position(
        &mut state,
        tools_area,
        tools_area.x + 1 + toggle_offset,
        tools_area.y + 1,
    ));
    assert_eq!(state.monitor_tab, first_advanced_monitor_tab());
    assert_eq!(state.monitor_tab, MonitorTab::Result);
    assert_eq!(state.selected_command, 0);

    // From the expanded advanced group, clicking a core tab collapses again.
    let expanded = format_monitor_tabs(state.monitor_tab);
    let overview_offset = expanded.find("Overview").unwrap() as u16;
    assert!(select_monitor_tab_at_position(
        &mut state,
        tools_area,
        tools_area.x + 1 + overview_offset,
        tools_area.y + 1,
    ));
    assert_eq!(state.monitor_tab, MonitorTab::Overview);
}

#[test]
fn monitor_tab_hit_test_selects_visible_tab() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 2,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let area = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 24,
    };
    let tools_area = Rect {
        x: 0,
        y: 3,
        width: 120,
        height: 9,
    };
    let tabs = format_monitor_tabs(state.monitor_tab);
    let changes_offset = tabs.find("Changes").unwrap() as u16;

    assert!(select_monitor_tab_at_position(
        &mut state,
        tools_area,
        tools_area.x + 1 + changes_offset,
        tools_area.y + 1,
    ));
    assert_eq!(state.monitor_tab, MonitorTab::Changes);
    assert_eq!(state.selected_command, 0);
    assert_eq!(state.last_event, "monitor tab: Changes");

    state.input.set_buffer("/he".to_string());
    let layout = chat_ui_layout(area);
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();
    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: layout.input.x + 1,
            row: layout.input.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    assert_eq!(state.monitor_tab, MonitorTab::Changes);
}

#[test]
fn health_tab_surfaces_model_credentials_and_config_actions() {
    let dir = tempdir().unwrap();
    let deepcli_dir = dir.path().join(".deepcli");
    fs::create_dir_all(deepcli_dir.join("credentials")).unwrap();
    fs::write(
        deepcli_dir.join("config.json"),
        r#"{
              "defaultProvider": "healthtest",
              "providers": {
                "healthtest": {
                  "type": "deepseek",
                  "credentialsFile": ".deepcli/credentials/healthtest-credentials.json",
                  "acceptanceModel": "acceptance-model",
                  "capabilities": ["tools"]
                }
              }
            }"#,
    )
    .unwrap();
    fs::write(
        deepcli_dir.join("credentials/healthtest-credentials.json"),
        r#"{"provider":"healthtest","model":"runtime-model","apiKey":"health-secret"}"#,
    )
    .unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "healthtest".to_string(),
            Some("session-model".to_string()),
        )
        .unwrap();
    let state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Health,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    let health = format_task_monitor_text(&state, None, 14);
    assert!(health.contains("[Health]"));
    assert!(health.contains("provider: active=healthtest model=session-model default=healthtest"));
    assert!(health.contains("credentials: api_key=configured file=present env=missing"));
    assert!(health.contains("runtime: type=deepseek model=runtime-model"));
    assert!(health.contains("config: project=present"));
    assert!(health.contains("/credentials status healthtest --json"));
    assert!(!health.contains("/credentials set healthtest"));
    assert!(!health.contains("health-secret"));
}

#[test]
fn health_tab_surfaces_missing_credentials_repair_action_and_opens_prompt() {
    let dir = tempdir().unwrap();
    let deepcli_dir = dir.path().join(".deepcli");
    fs::create_dir_all(deepcli_dir.join("credentials")).unwrap();
    fs::write(
        deepcli_dir.join("config.json"),
        r#"{
              "defaultProvider": "healthtest",
              "providers": {
                "healthtest": {
                  "type": "deepseek",
                  "credentialsFile": ".deepcli/credentials/healthtest-credentials.json",
                  "acceptanceModel": "acceptance-model",
                  "capabilities": ["tools"]
                }
              }
            }"#,
    )
    .unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "healthtest".to_string(),
            Some("session-model".to_string()),
        )
        .unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Health,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    let health = format_task_monitor_text(&state, None, 14);
    assert!(health.contains("credentials: api_key=missing file=missing env=missing"));
    assert!(health.contains("/credentials set healthtest"));

    let actions = health_quick_actions_for_state(&state);
    state.selected_command = actions
        .iter()
        .position(|action| action.command == "/credentials set healthtest")
        .unwrap();
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();
    activate_selected_monitor_quick_action(&mut state, &actions, &progress_tx, &done_tx);

    assert_eq!(state.last_event, "credential prompt opened");
    let prompt = state.credential_prompt.as_ref().unwrap();
    assert_eq!(prompt.provider, "healthtest");
    assert!(!prompt.force);
}

#[test]
fn library_tab_surfaces_prompt_skill_and_agent_inventory() {
    let dir = tempdir().unwrap();
    PromptStore::new(dir.path())
        .save("aaa-custom", "Custom prompt body")
        .unwrap();
    SkillStore::new(dir.path())
        .generate("compiler", "SysY compiler workflow")
        .unwrap();
    AgentStore::new(dir.path())
        .create_subagent_task(None, "inspect parser module", 1, vec![PathBuf::from("src")])
        .unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    let state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Library,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    let library = format_task_monitor_text(&state, None, 18);
    assert!(library.contains("[Library]"));
    assert!(library.contains("prompts: total=4 custom=1 builtins=3"));
    assert!(library.contains("prompt aaa-custom - Custom project prompt"));
    assert!(library.contains("skills: total=1"));
    assert!(library.contains("skill compiler - SysY compiler workflow"));
    assert!(library.contains("agents: total=1"));
    assert!(library.contains("inspect parser module"));
    assert!(library.contains("/prompt render <name> --file path"));
    assert!(library.contains("/skill list --json"));
    assert!(library.contains("/agent list --json"));
}

#[test]
fn agent_editor_dialog_saves_queued_task_descriptor() {
    let dir = tempdir().unwrap();
    let store = AgentStore::new(dir.path());
    let task = store
        .create_subagent_task(None, "inspect parser module", 1, vec![PathBuf::from("src")])
        .unwrap();
    let session = SessionStore::new(dir.path())
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    let mut state = test_tui_state();
    state.active_session = Some(ActiveSessionRef {
        workspace: dir.path().to_path_buf(),
        session_id: session.id().to_string(),
    });

    open_agent_editor_dialog(&mut state, task.id).unwrap();
    assert!(matches!(state.dialog, Some(TuiDialog::AgentEditor(_))));
    replace_dialog_field(&mut state, "task", "inspect lexer module").unwrap();
    replace_dialog_field(&mut state, "write_scope", "src/lexer.rs").unwrap();
    replace_dialog_field(&mut state, "read_scope", "src\nREADME.md").unwrap();
    replace_dialog_field(&mut state, "allowed_tools", "read_file\nrun_shell").unwrap();
    replace_dialog_field(&mut state, "context", "focus on token handling").unwrap();
    handle_dialog_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
        &mut state,
    );

    assert_eq!(state.last_event, "agent task saved");
    assert!(state.dialog.is_none());
    let updated = store.load(task.id).unwrap();
    assert_eq!(updated.task, "inspect lexer module");
    assert_eq!(updated.write_scope, vec![PathBuf::from("src/lexer.rs")]);
    assert_eq!(
        updated.read_scope,
        vec![PathBuf::from("src"), PathBuf::from("README.md")]
    );
    assert_eq!(updated.allowed_tools, vec!["read_file", "run_shell"]);
    assert_eq!(updated.context.as_deref(), Some("focus on token handling"));
}

#[test]
fn settings_dialog_validates_and_persists_whitelisted_config() {
    let dir = tempdir().unwrap();
    let session = SessionStore::new(dir.path())
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    let mut state = test_tui_state();
    state.active_session = Some(ActiveSessionRef {
        workspace: dir.path().to_path_buf(),
        session_id: session.id().to_string(),
    });

    open_settings_dialog(&mut state).unwrap();
    assert!(matches!(state.dialog, Some(TuiDialog::Settings(_))));
    let body = dialog_body_for_state(&state, 16).unwrap();
    assert!(body.contains("agent.providerTurnTimeoutSeconds"));
    assert!(!body.contains("agent.maxToolIterations"));
    assert!(!body.contains("apiKey"));

    replace_dialog_field(&mut state, "agent.providerTurnTimeoutSeconds", "0").unwrap();
    handle_dialog_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
        &mut state,
    );
    assert!(state.dialog.is_some());
    assert!(state.last_event.contains("settings save failed"));

    replace_dialog_field(&mut state, "agent.providerTurnTimeoutSeconds", "45").unwrap();
    handle_dialog_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
        &mut state,
    );
    assert_eq!(state.last_event, "settings saved");
    assert!(state.dialog.is_none());
    let config = AppConfig::load_effective(dir.path(), None).unwrap();
    assert_eq!(config.agent.provider_turn_timeout_seconds, 45);
}

#[test]
fn monitor_quick_actions_can_select_and_prefill_editable_commands() {
    let dir = tempdir().unwrap();
    let session = SessionStore::new(dir.path())
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Library,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    assert!(handle_monitor_quick_action_key(
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        &mut state,
        &progress_tx,
        &done_tx
    ));
    assert_eq!(state.selected_command, 1);
    let rendered = format_task_monitor_text(&state, None, 18);
    assert!(rendered.contains("quick actions (Up/Down select, Enter run/edit):"));
    assert!(!rendered.contains("quick actions (Up/Down select, Enter run):"));
    assert!(rendered.contains("> /prompt render <name> --file path (edit)"));

    assert!(handle_monitor_quick_action_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
        &progress_tx,
        &done_tx
    ));
    assert_eq!(state.input.buffer(), "/prompt render <name> --file path");
    assert_eq!(state.selected_command, 0);
    assert!(state
        .last_event
        .contains("quick action ready for edit: /prompt render"));
}

#[test]
fn monitor_quick_action_hit_test_can_activate_action() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Environment,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let tools_area = Rect {
        x: 0,
        y: 3,
        width: 100,
        height: 12,
    };
    let rendered = format_task_monitor_text(&state, None, tools_area.height);
    let action_row = rendered
        .lines()
        .position(|line| line.contains("/doctor docker --json"))
        .expect("environment check quick action should be visible");
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    assert!(activate_monitor_quick_action_at_row(
        &mut state,
        tools_area,
        tools_area.y + 1 + action_row as u16,
        &progress_tx,
        &done_tx,
    ));

    assert_eq!(state.selected_command, 0);
    assert!(state
        .last_event
        .contains("quick action submitted: /doctor docker --json"));
}

#[test]
fn monitor_truncation_keeps_selected_quick_action_visible() {
    let state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 6,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Environment,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    let rendered = format_task_monitor_text(&state, None, 7);
    assert!(rendered.contains("[Environment]"));
    assert!(rendered.contains("> /handoff --env-check docker --format pr"));
    assert!(rendered.contains("[more: use /session"));
    assert!(!rendered.contains("environment evidence unavailable"));
}

#[test]
fn approvals_tab_can_approve_selected_request() {
    let dir = tempdir().unwrap();
    let runtime = AgentRuntime::new(
        AppConfig::default(),
        RuntimeOptions {
            workspace: dir.path().to_path_buf(),
            provider: None,
            model: None,
            assume_yes: true,
            resume_session: None,
            stream_output: false,
        },
    )
    .unwrap();
    let session_id = runtime.session_id();
    let store = SessionStore::new(dir.path());
    let session = store.load(&session_id).unwrap();
    let request = session
        .enqueue_approval_request(
            "write_file",
            PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk: RiskLevel::Medium,
                reason: "write requires approval".to_string(),
            },
        )
        .unwrap();

    let mut state = TuiState {
        runtime: Some(runtime),
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Approvals,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert_eq!(blocker_count(&state), Some(1));
    assert!(handle_approval_tab_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.last_event, "approval approved");
    assert!(state
        .chat
        .last()
        .is_some_and(|line| line.content.contains("approved approval request")));
    let loaded = store.load(&session_id).unwrap();
    let updated = loaded.load_approval_requests().unwrap();
    assert_eq!(updated[0].id, request.id);
    assert_eq!(updated[0].status, ApprovalStatus::Approved);
    assert_eq!(blocker_count(&state), Some(0));
}

#[test]
fn approval_prompt_covers_input_and_returns_after_choice() {
    let dir = tempdir().unwrap();
    let runtime = AgentRuntime::new(
        AppConfig::default(),
        RuntimeOptions {
            workspace: dir.path().to_path_buf(),
            provider: None,
            model: None,
            assume_yes: true,
            resume_session: None,
            stream_output: false,
        },
    )
    .unwrap();
    let session_id = runtime.session_id();
    let store = SessionStore::new(dir.path());
    let session = store.load(&session_id).unwrap();
    let request = session
        .enqueue_approval_request(
            "write_file",
            PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk: RiskLevel::Medium,
                reason: "write requires approval".to_string(),
            },
        )
        .unwrap();

    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = TuiState {
        runtime: Some(runtime),
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
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
    };
    state.input.set_buffer("draft message".to_string());

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let blocked = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(blocked.contains("Permission"));
    assert!(blocked.contains("approve/deny"));
    assert!(blocked.contains("write_file"));
    assert!(!blocked.contains("Message Box"));
    assert!(!blocked.contains("draft message"));

    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();
    let mut clipboard = Vec::new();
    handle_tui_key_with_clipboard_writer(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
        &progress_tx,
        &done_tx,
        &mut clipboard,
    )
    .unwrap();

    assert_eq!(state.last_event, "approval approved");
    assert_eq!(state.input.buffer(), "draft message");
    let loaded = store.load(&session_id).unwrap();
    let updated = loaded.load_approval_requests().unwrap();
    assert_eq!(updated[0].id, request.id);
    assert_eq!(updated[0].status, ApprovalStatus::Approved);

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let restored = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(restored.contains("Message Box"));
    assert!(restored.contains("draft message"));
    assert!(!restored.contains("Permission"));
}

#[test]
fn permission_dialog_auto_opens_for_pending_approval() {
    let dir = tempdir().unwrap();
    let runtime = AgentRuntime::new(
        AppConfig::default(),
        RuntimeOptions {
            workspace: dir.path().to_path_buf(),
            provider: None,
            model: None,
            assume_yes: true,
            resume_session: None,
            stream_output: false,
        },
    )
    .unwrap();
    let session_id = runtime.session_id();
    let store = SessionStore::new(dir.path());
    let session = store.load(&session_id).unwrap();
    session
        .enqueue_approval_request(
            "write_file",
            PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk: RiskLevel::Medium,
                reason: "write requires approval".to_string(),
            },
        )
        .unwrap();
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = test_tui_state();
    state.runtime = Some(runtime);
    state.input.set_buffer("draft message".to_string());

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered.contains("Permission"));
    assert!(rendered.contains("approve/deny"));
    assert!(rendered.contains("write_file"));
    assert!(!rendered.contains("Message Box"));

    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();
    let mut clipboard = Vec::new();
    handle_tui_key_with_clipboard_writer(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
        &progress_tx,
        &done_tx,
        &mut clipboard,
    )
    .unwrap();
    assert_eq!(state.last_event, "approval approved");
}

#[test]
fn approvals_tab_opens_and_saves_btw_answer_prompt() {
    let dir = tempdir().unwrap();
    let runtime = AgentRuntime::new(
        AppConfig::default(),
        RuntimeOptions {
            workspace: dir.path().to_path_buf(),
            provider: None,
            model: None,
            assume_yes: true,
            resume_session: None,
            stream_output: false,
        },
    )
    .unwrap();
    let session_id = runtime.session_id();
    let store = SessionStore::new(dir.path());
    let session = store.load(&session_id).unwrap();
    let question = session
        .enqueue_side_question("which model should I use?")
        .unwrap();

    let mut state = TuiState {
        runtime: Some(runtime),
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Approvals,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert_eq!(blocker_count(&state), Some(1));
    assert!(handle_approval_tab_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.last_event, "btw answer prompt opened");
    assert_eq!(state.input.buffer(), "");
    assert!(matches!(state.dialog, Some(TuiDialog::Interview(_))));
    assert!(state.side_question_prompt.is_none());
    for ch in "use v-pro".chars() {
        handle_side_question_prompt_key(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            &mut state,
        );
    }
    for _ in 0..4 {
        handle_side_question_prompt_key(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            &mut state,
        );
    }
    handle_side_question_prompt_key(
        KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE),
        &mut state,
    );
    handle_side_question_prompt_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
    );
    assert_eq!(state.last_event, "btw answer saved");
    assert!(state.dialog.is_none());
    assert!(state.side_question_prompt.is_none());
    assert!(state
        .chat
        .last()
        .is_some_and(|line| line.content.contains("answered btw question")));
    let loaded = store.load(&session_id).unwrap();
    let updated = loaded.load_side_questions().unwrap();
    assert_eq!(updated[0].id, question.id);
    assert_eq!(updated[0].status, SideQuestionStatus::Answered);
    assert_eq!(updated[0].answer.as_deref(), Some("use v4-pro"));
    assert_eq!(blocker_count(&state), Some(0));
}

#[test]
fn interview_dialog_saves_side_question_answer() {
    let dir = tempdir().unwrap();
    let runtime = AgentRuntime::new(
        AppConfig::default(),
        RuntimeOptions {
            workspace: dir.path().to_path_buf(),
            provider: None,
            model: None,
            assume_yes: true,
            resume_session: None,
            stream_output: false,
        },
    )
    .unwrap();
    let session_id = runtime.session_id();
    let store = SessionStore::new(dir.path());
    let session = store.load(&session_id).unwrap();
    let question = session
        .enqueue_side_question("which model should I use?")
        .unwrap();
    let mut state = test_tui_state();
    state.runtime = Some(runtime);
    state.monitor_tab = MonitorTab::Approvals;

    assert!(handle_approval_tab_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state
    ));
    assert!(matches!(state.dialog, Some(TuiDialog::Interview(_))));
    assert!(state.side_question_prompt.is_none());

    for ch in "use v-pro".chars() {
        handle_dialog_key(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            &mut state,
        );
    }
    for _ in 0..4 {
        handle_dialog_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &mut state);
    }
    handle_dialog_key(
        KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE),
        &mut state,
    );
    handle_dialog_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
    );

    assert_eq!(state.last_event, "btw answer saved");
    assert!(state.dialog.is_none());
    let loaded = store.load(&session_id).unwrap();
    let updated = loaded.load_side_questions().unwrap();
    assert_eq!(updated[0].id, question.id);
    assert_eq!(updated[0].status, SideQuestionStatus::Answered);
    assert_eq!(updated[0].answer.as_deref(), Some("use v4-pro"));
}

#[test]
fn approval_prompt_mouse_selects_blockers_without_acting() {
    let dir = tempdir().unwrap();
    let runtime = AgentRuntime::new(
        AppConfig::default(),
        RuntimeOptions {
            workspace: dir.path().to_path_buf(),
            provider: None,
            model: None,
            assume_yes: true,
            resume_session: None,
            stream_output: false,
        },
    )
    .unwrap();
    let session_id = runtime.session_id();
    let store = SessionStore::new(dir.path());
    let session = store.load(&session_id).unwrap();
    session
        .enqueue_approval_request(
            "write_file",
            PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk: RiskLevel::Medium,
                reason: "write requires approval".to_string(),
            },
        )
        .unwrap();
    let question = session.enqueue_side_question("switch model?").unwrap();

    let mut state = TuiState {
        runtime: Some(runtime),
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Approvals,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let area = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 24,
    };
    let layout = chat_ui_layout(area);
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: layout.input.x + 2,
            row: layout.input.y + 2,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    assert_eq!(state.selected_approval, 1);
    assert_eq!(
        state.last_event,
        format!("btw selected: {}", short_id(&question.id.to_string()))
    );
    assert!(state.side_question_prompt.is_none());

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: layout.input.x + 2,
            row: layout.input.y + 1 + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    assert_eq!(state.selected_approval, 0);
    assert!(state.last_event.starts_with("approval selected: "));
    let loaded = store.load(&session_id).unwrap();
    assert_eq!(
        loaded.load_approval_requests().unwrap()[0].status,
        ApprovalStatus::Pending
    );

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: layout.input.x + 2,
            row: layout.input.y + 1 + 2,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    assert_eq!(state.selected_approval, 1);
    assert_eq!(
        state.last_event,
        format!("btw selected: {}", short_id(&question.id.to_string()))
    );
    assert!(state.side_question_prompt.is_none());
}

#[test]
fn running_tui_handles_btw_commands_without_runtime() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let session_id = session.id().to_string();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session_id.clone(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_running_tui_local_command(
        &mut state,
        "/btw ask explain the diff after tests"
    ));
    assert!(state
        .last_event
        .contains("running command ok: queued by-the-way question"));
    let loaded = store.load(&session_id).unwrap();
    let questions = loaded.load_side_questions().unwrap();
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0].status, SideQuestionStatus::Open);

    assert!(handle_running_tui_local_command(&mut state, "/btw list"));
    assert!(state
        .chat
        .last()
        .is_some_and(|line| line.content.contains("explain the diff after tests")));

    let answer = format!(
        "/btw answer {} after test pass",
        short_id(&questions[0].id.to_string())
    );
    assert!(handle_running_tui_local_command(&mut state, &answer));
    let loaded = store.load(&session_id).unwrap();
    let questions = loaded.load_side_questions().unwrap();
    assert_eq!(questions[0].status, SideQuestionStatus::Answered);
    assert_eq!(questions[0].answer.as_deref(), Some("after test pass"));
}

#[test]
fn running_tui_status_reads_active_session_without_runtime() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.append_message("user", "run tests").unwrap();
    session.enqueue_side_question("summarize later").unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_running_tui_local_command(&mut state, "/status"));
    let output = &state.chat.last().unwrap().content;
    assert!(output.contains("running session"));
    assert!(output.contains("open_btw=1"));
    assert!(output.contains("messages=1"));
}

#[test]
fn running_tui_stop_marks_session_paused_and_rebuilds_runtime() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".deepcli/credentials")).unwrap();
    std::fs::write(
        dir.path().join(".deepcli/config.json"),
        serde_json::to_vec_pretty(&AppConfig::default()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path()
            .join(".deepcli/credentials/deepseek-credentials.json"),
        r#"{"apiKey":"test","model":"deepseek-chat"}"#,
    )
    .unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-chat".to_string()),
        )
        .unwrap();
    let session_id = session.id().to_string();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session_id.clone(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_running_tui_local_command(&mut state, "/stop"));
    assert!(!state.running);
    assert!(state.runtime.is_some());
    assert_eq!(state.last_event, "task stopped");
    assert!(state
        .chat
        .last()
        .is_some_and(|line| line.content.contains("已停止当前任务")));
    let loaded = store.load(&session_id).unwrap();
    assert_eq!(loaded.metadata.state, SessionState::Paused);
    assert!(loaded
        .load_audit_events()
        .unwrap()
        .iter()
        .any(|event| event.event_type == "task_stopped"));
}

#[test]
fn running_tui_handles_trace_approval_and_session_commands_without_runtime() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.append_message("user", "inspect me").unwrap();
    session
        .append_audit_event(
            "provider_turn_started",
            serde_json::json!({
                "iteration": 1,
                "timeout_seconds": 600,
                "request": {
                    "message_count": 2,
                    "tool_count": 1,
                    "total_bytes": 2048,
                    "compacted": false
                }
            }),
        )
        .unwrap();
    session
        .append_audit_event(
            "provider_turn_completed",
            serde_json::json!({
                "iteration": 1,
                "elapsed_ms": 1200,
                "tool_calls": 0,
                "usage": {
                    "total_tokens": 42
                }
            }),
        )
        .unwrap();
    fs::create_dir_all(dir.path().join(".deepcli/logs")).unwrap();
    fs::write(
        dir.path().join(".deepcli/logs/deepcli.log"),
        "provider ok\n",
    )
    .unwrap();
    let request = session
        .enqueue_approval_request(
            "write_file",
            PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk: RiskLevel::Medium,
                reason: "write requires approval".to_string(),
            },
        )
        .unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_running_tui_local_command(
        &mut state,
        "/trace --limit 5"
    ));
    assert!(state
        .chat
        .last()
        .is_some_and(|line| line.content.contains("provider_turn_started")));

    assert!(handle_running_tui_local_command(&mut state, "/usage"));
    assert!(state.chat.last().is_some_and(|line| {
        line.content.contains("provider turns:") && line.content.contains("total=42")
    }));

    assert!(handle_running_tui_local_command(
        &mut state,
        "/logs --file deepcli.log --limit 5"
    ));
    assert!(state
        .chat
        .last()
        .is_some_and(|line| line.content.contains("provider ok")));

    assert!(handle_running_tui_local_command(&mut state, "/help usage"));
    assert!(state
        .chat
        .last()
        .is_some_and(|line| line.content.contains("running-safe: yes")));

    assert!(handle_running_tui_local_command(
        &mut state,
        "/terminal --dry-run --json"
    ));
    assert!(state.chat.last().is_some_and(|line| {
        line.content.contains("deepcli.terminal.v1") && line.content.contains("\"opened\": false")
    }));

    assert!(handle_running_tui_local_command(
        &mut state,
        "/session history"
    ));
    assert!(state
        .chat
        .last()
        .is_some_and(|line| line.content.contains("inspect me")));

    assert!(handle_running_tui_local_command(
        &mut state,
        "/approval list"
    ));
    assert!(state
        .chat
        .last()
        .is_some_and(|line| line.content.contains("write_file")));

    let approve = format!("/approval approve {}", short_id(&request.id.to_string()));
    assert!(handle_running_tui_local_command(&mut state, &approve));
    let loaded = store.load(&session.id().to_string()).unwrap();
    assert_eq!(
        loaded.load_approval_requests().unwrap()[0].status,
        ApprovalStatus::Approved
    );
}

#[test]
fn running_tui_allows_session_restore_backup_dry_run_only() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.save_backup("src/lib.rs", "old content\n").unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "new content\n").unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_running_tui_local_command(
        &mut state,
        "/session restore-backup latest --dry-run --json"
    ));
    let preview = state
        .chat
        .last()
        .expect("dry-run should append a result")
        .content
        .clone();
    let value: serde_json::Value = serde_json::from_str(&preview).unwrap();
    assert_eq!(value["schema"], "deepcli.session.restore_backup.v1");
    assert_eq!(value["status"], "preview");
    assert_eq!(value["dryRun"], true);
    assert_eq!(
        fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "new content\n"
    );

    assert!(handle_running_tui_local_command(
            &mut state,
            "/session restore-backup latest --dry-run --json --output .deepcli/exports/restore-preview.json"
        ));
    assert!(state.chat.last().is_some_and(|line| {
        line.role == "error" && line.content.contains("restore-backup --dry-run --output")
    }));
    assert!(!dir
        .path()
        .join(".deepcli/exports/restore-preview.json")
        .exists());

    assert!(handle_running_tui_local_command(
        &mut state,
        "/session restore-backup latest"
    ));
    assert!(state.chat.last().is_some_and(|line| {
        line.role == "error"
            && line
                .content
                .contains("stop or wait for the running task before restoring")
    }));
    assert_eq!(
        fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "new content\n"
    );
}

#[test]
fn running_tui_blocks_session_write_actions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.rename("active task").unwrap();
    session.append_message("user", "inspect me").unwrap();
    let empty = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    let empty_id = empty.id().to_string();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_running_tui_local_command(
        &mut state,
        "/session history --json"
    ));
    assert!(state.chat.last().is_some_and(|line| {
        line.role == "deepcli" && line.content.contains("deepcli.session.inspect.v1")
    }));

    for command in [
        "/session history --json --output .deepcli/exports/session-history.json",
        "/session rename --current renamed while running",
        "/session export --current .deepcli/exports/session-current.json",
        "/session prune-empty --force",
    ] {
        assert!(handle_running_tui_local_command(&mut state, command));
        assert!(
            state.chat.last().is_some_and(|line| {
                line.role == "error"
                    && line
                        .content
                        .contains("wait for the running task or use `/stop`")
            }),
            "{command} should be rejected while running"
        );
    }
    let loaded = store.load(&session.id().to_string()).unwrap();
    assert_eq!(loaded.metadata.title.as_deref(), Some("active task"));
    assert!(store.load(&empty_id).is_ok());
    assert!(!dir
        .path()
        .join(".deepcli/exports/session-history.json")
        .exists());
    assert!(!dir
        .path()
        .join(".deepcli/exports/session-current.json")
        .exists());
}

#[test]
fn running_tui_handles_product_loop_reports_without_runtime() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    for (command, expected_schema) in [
        ("/recipes sota --json", "deepcli.recipes.v1"),
        ("/scorecard --json", "deepcli.scorecard.v1"),
        ("/round --json", "deepcli.round.v1"),
        ("/benchmark status --json", "deepcli.benchmark.status.v1"),
        (
            "/benchmark baselines --json",
            "deepcli.benchmark.baselines.v1",
        ),
        ("/preflight --dry-run --json", "deepcli.preflight.v1"),
        ("/privacy --json --no-history", "deepcli.privacy.scan.v1"),
    ] {
        assert!(handle_running_tui_local_command(&mut state, command));
        assert!(
            state
                .chat
                .last()
                .is_some_and(|line| line.content.contains(expected_schema)),
            "{command} should render {expected_schema}"
        );
        assert!(state.last_event.starts_with("running command ok"));
    }

    assert!(handle_running_tui_local_command(
        &mut state,
        "/round --json --run-benchmark"
    ));
    assert!(state.chat.last().is_some_and(|line| {
        line.role == "error"
            && line
                .content
                .contains("stop or wait for the running task before executing")
    }));

    assert!(handle_running_tui_local_command(
        &mut state,
        "/benchmark run-suite --json"
    ));
    assert!(state.chat.last().is_some_and(|line| {
        line.role == "error"
            && line
                .content
                .contains("stop or wait for the running task before executing")
    }));
}

#[test]
fn running_tui_blocks_artifact_output_for_local_side_commands() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".deepcli/logs")).unwrap();
    fs::write(
        dir.path().join(".deepcli/logs/deepcli.log"),
        "provider ok\n",
    )
    .unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session
        .append_audit_event(
            "provider_turn_completed",
            serde_json::json!({
                "iteration": 1,
                "elapsed_ms": 1200,
                "tool_calls": 0,
                "usage": {
                    "total_tokens": 42
                }
            }),
        )
        .unwrap();
    session.enqueue_side_question("answer after tests").unwrap();
    session
        .enqueue_approval_request(
            "write_file",
            PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk: RiskLevel::Medium,
                reason: "write requires approval".to_string(),
            },
        )
        .unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_running_tui_local_command(
        &mut state,
        "/usage --json"
    ));
    assert!(state
        .chat
        .last()
        .is_some_and(|line| line.content.contains("deepcli.usage.v1")));

    for (command, output_path) in [
        (
            "/usage --json --output .deepcli/exports/usage.json",
            ".deepcli/exports/usage.json",
        ),
        (
            "/trace --json --output=.deepcli/exports/trace.json",
            ".deepcli/exports/trace.json",
        ),
        (
            "/logs --json --output .deepcli/exports/logs.json",
            ".deepcli/exports/logs.json",
        ),
        (
            "/privacy --json --no-history --output .deepcli/exports/privacy.json",
            ".deepcli/exports/privacy.json",
        ),
        (
            "/recipes sota --json --output .deepcli/exports/recipes.json",
            ".deepcli/exports/recipes.json",
        ),
        (
            "/scorecard --json --output .deepcli/exports/scorecard.json",
            ".deepcli/exports/scorecard.json",
        ),
        (
            "/round --json --output .deepcli/exports/round.json",
            ".deepcli/exports/round.json",
        ),
        (
            "/benchmark status --json --output .deepcli/exports/benchmark-status.json",
            ".deepcli/exports/benchmark-status.json",
        ),
        (
            "/selftest --json --output .deepcli/exports/selftest.json",
            ".deepcli/exports/selftest.json",
        ),
        (
            "/preflight --dry-run --json --output .deepcli/exports/preflight.json",
            ".deepcli/exports/preflight.json",
        ),
        (
            "/completion json --output .deepcli/exports/commands.json",
            ".deepcli/exports/commands.json",
        ),
        (
            "/approval list --json --output .deepcli/exports/approvals.json",
            ".deepcli/exports/approvals.json",
        ),
        (
            "/btw list --json --output .deepcli/exports/btw.json",
            ".deepcli/exports/btw.json",
        ),
        (
            "/terminal --dry-run --json --output .deepcli/exports/terminal.json",
            ".deepcli/exports/terminal.json",
        ),
        (
            "/fork --current --dry-run --json --output .deepcli/exports/fork.json",
            ".deepcli/exports/fork.json",
        ),
    ] {
        assert!(handle_running_tui_local_command(&mut state, command));
        assert!(
            state.chat.last().is_some_and(|line| {
                line.role == "error"
                    && line.content.contains("writes a file")
                    && line.content.contains("stop or wait for the running task")
            }),
            "{command} should be rejected while the agent is running"
        );
        assert!(
            !dir.path().join(output_path).exists(),
            "{output_path} should not be written while the agent is running"
        );
    }
}

#[test]
fn running_tui_blocks_completion_force_install() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(ensure_running_completion_is_observation_only(&[
        "install".to_string(),
        "zsh".to_string(),
        "--force".to_string(),
        "--json".to_string(),
    ])
    .is_err());
    assert!(ensure_running_completion_is_observation_only(&[
        "install".to_string(),
        "zsh".to_string(),
        "--json".to_string(),
    ])
    .is_ok());
    assert!(ensure_running_completion_is_observation_only(&[
        "status".to_string(),
        "zsh".to_string(),
        "--json".to_string(),
    ])
    .is_ok());

    assert!(handle_running_tui_local_command(
        &mut state,
        "/completion install zsh --force --dry-run --json"
    ));
    assert!(state.chat.last().is_some_and(|line| {
        line.role == "error"
            && line.content.contains("completion install --force")
            && line.content.contains("stop or wait for the running task")
    }));
}

#[test]
fn running_tui_can_fork_persisted_context_without_runtime() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.rename("parallel investigation").unwrap();
    session
        .append_message("user", "inspect fork behavior")
        .unwrap();
    session
        .append_message("assistant", "persisted answer")
        .unwrap();
    session.set_state(SessionState::Executing).unwrap();
    let source_id = session.id().to_string();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: source_id.clone(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_running_tui_local_command(
        &mut state,
        "/fork --current --no-open --json"
    ));
    let output = state
        .chat
        .last()
        .expect("fork should append a chat line")
        .content
        .clone();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.session.fork.v1");
    assert_eq!(value["source"]["id"], source_id);
    assert_eq!(value["contextCopy"]["sourceState"], "executing");
    assert_eq!(value["contextCopy"]["runningAgentState"], true);
    assert_eq!(value["contextCopy"]["hotForkSupported"], false);
    let fork_id = value["fork"]["id"].as_str().unwrap();
    let fork = store.load(fork_id).unwrap();
    assert_eq!(fork.load_messages().unwrap().len(), 2);
    assert_eq!(fork.metadata.state, SessionState::WaitingUser);
    assert!(state.last_event.starts_with("running command ok"));
}

#[test]
fn running_tui_allows_read_only_git_inspection_only() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("tracked.txt"), "base\n").unwrap();
    run_git_for_ui_test(dir.path(), &["init"]);
    run_git_for_ui_test(dir.path(), &["config", "user.name", "zero-kotori"]);
    run_git_for_ui_test(
        dir.path(),
        &["config", "user.email", "kotorizero8@gmail.com"],
    );
    run_git_for_ui_test(dir.path(), &["add", "tracked.txt"]);
    run_git_for_ui_test(dir.path(), &["commit", "-m", "baseline"]);
    std::fs::write(dir.path().join("tracked.txt"), "base\nchanged\n").unwrap();

    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_running_tui_local_command(
        &mut state,
        "/git status --json"
    ));
    let value: serde_json::Value =
        serde_json::from_str(&state.chat.last().unwrap().content).unwrap();
    assert_eq!(value["schema"], "deepcli.git.inspect.v1");
    assert_eq!(value["kind"], "status");
    assert!(state.last_event.starts_with("running command ok"));

    assert!(handle_running_tui_local_command(
        &mut state,
        "/git status --json --output .deepcli/exports/git-status.json"
    ));
    assert!(state.chat.last().is_some_and(|line| {
        line.role == "error"
            && line
                .content
                .contains("stop or wait for the running task before executing")
    }));
    assert!(!dir.path().join(".deepcli/exports/git-status.json").exists());

    assert!(handle_running_tui_local_command(
        &mut state,
        "/git commit running"
    ));
    assert!(state.chat.last().is_some_and(|line| {
        line.role == "error"
            && line
                .content
                .contains("stop or wait for the running task before executing")
    }));

    let git_suggestions = slash_command_suggestions_for_state("/gi", true).unwrap();
    assert!(git_suggestions
        .iter()
        .any(|summary| summary.name == "/git" && summary.running_safe));
}

fn run_git_for_ui_test(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn header_status_uses_active_session_metadata_while_running() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.rename("compiler repair").unwrap();
    let state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    let header = header_status_for_state(&state);
    assert_eq!(header.session, session.id().to_string());
    assert_eq!(header.title, "compiler repair");
    assert_eq!(header.provider, "deepseek");
    assert_eq!(header.model, "deepseek-v4-pro");
    assert_ne!(header.session, "<running>");
}

#[test]
fn task_monitor_reads_active_session_while_runtime_is_running() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.enqueue_side_question("switch model?").unwrap();
    session
        .enqueue_approval_request(
            "write_file",
            PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk: RiskLevel::Medium,
                reason: "write requires approval".to_string(),
            },
        )
        .unwrap();
    session
        .append_tool_call(&ToolCallRecord {
            tool: "check_environment".to_string(),
            input: json!({"target": "compiler"}),
            output: json!({
                "target": "compiler",
                "ready": false,
                "recommended_action": "/install compiler --smoke"
            }),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: Utc::now(),
        })
        .unwrap();
    session
        .append_audit_event(
            "provider_turn_started",
            json!({
                "request": {
                    "total_bytes": 700_000,
                    "compacted": true
                }
            }),
        )
        .unwrap();
    session
        .append_audit_event(
            "provider_turn_completed",
            json!({
                "elapsed_ms": 45_000,
                "tool_calls": 2,
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 20,
                    "total_tokens": 120,
                    "prompt_cache_hit_tokens": 5,
                    "prompt_cache_miss_tokens": 5
                }
            }),
        )
        .unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Overview,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    let monitor = session_monitor_for_state(&state).unwrap();
    let overview = format_task_monitor_text(&state, Some(&monitor), 9);
    assert!(overview.contains("state=New ui=running"));
    assert!(overview.contains("approvals=1 btw=1"));

    state.monitor_tab = MonitorTab::Approvals;
    let approvals = format_task_monitor_text(&state, Some(&monitor), 9);
    assert!(approvals.contains("pending approvals: 1"));
    assert!(approvals.contains("open btw questions: 1"));
    assert!(!approvals.contains("running handoff"));

    state.monitor_tab = MonitorTab::Environment;
    let environment = format_task_monitor_text(&state, Some(&monitor), 10);
    assert!(environment.contains("check_environment target=compiler status=needs_setup"));
    assert!(environment.contains("recommended: /install compiler --smoke"));
    let actions = environment_quick_actions(Some(&monitor));
    assert!(actions
        .iter()
        .any(|action| action.command == "/install compiler --smoke" && action.edit_before_run));
    assert!(actions
        .iter()
        .any(|action| action.command == "/compiler test --json"));

    state.monitor_tab = MonitorTab::Usage;
    let usage = format_task_monitor_text(&state, Some(&monitor), 10);
    assert!(usage.contains("provider turns: started=1 completed=1 avg=45000ms"));
    assert!(usage.contains("tokens: prompt=100 completion=20 total=120"));
    assert!(usage.contains("hit_rate=50.0%"));
}

#[test]
fn environment_setup_quick_action_prefills_instead_of_running() {
    let monitor = SessionMonitor {
        observation: SessionObservation {
            state: "Running".to_string(),
            plan_total: 0,
            plan_completed: 0,
            plan_in_progress: 0,
            plan_failed: 0,
            current_step: None,
            latest_test: None,
            pending_approvals: 0,
            open_questions: 0,
            tool_calls: 0,
            failed_tools: 0,
        },
        usage: SessionObservationUsage::default(),
        recent_tests: Vec::new(),
        recent_environment: vec![SessionObservationEnvironment {
            tool: "check_environment".to_string(),
            target: "compiler".to_string(),
            status: "needs_setup".to_string(),
            ready: Some(false),
            detail: "recommended: /install compiler --smoke".to_string(),
        }],
        pending_approvals: Vec::new(),
        open_questions: Vec::new(),
        recent_events: Vec::new(),
    };
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Environment,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let actions = environment_quick_actions(Some(&monitor));
    let setup_index = actions
        .iter()
        .position(|action| action.command == "/install compiler --smoke")
        .unwrap();
    assert!(actions[setup_index].edit_before_run);
    state.selected_command = setup_index;
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    activate_selected_monitor_quick_action(&mut state, &actions, &progress_tx, &done_tx);

    assert_eq!(state.input.buffer(), "/install compiler --smoke");
    assert_eq!(
        state.last_event,
        "quick action ready for edit: /install compiler --smoke"
    );
    assert!(state.running);
}

#[test]
fn approvals_tab_can_approve_active_session_while_running() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let request = session
        .enqueue_approval_request(
            "write_file",
            PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk: RiskLevel::Medium,
                reason: "write requires approval".to_string(),
            },
        )
        .unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Approvals,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert_eq!(blocker_count(&state), Some(1));
    assert!(handle_approval_tab_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.last_event, "approval approved");
    let loaded = store.load(&session.id().to_string()).unwrap();
    let updated = loaded.load_approval_requests().unwrap();
    assert_eq!(updated[0].id, request.id);
    assert_eq!(updated[0].status, ApprovalStatus::Approved);
    assert_eq!(blocker_count(&state), Some(0));
}

#[test]
fn approvals_tab_answers_active_btw_question_while_running() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let question = session.enqueue_side_question("which tests ran?").unwrap();
    let mut state = TuiState {
        runtime: None,
        active_session: Some(ActiveSessionRef {
            workspace: dir.path().to_path_buf(),
            session_id: session.id().to_string(),
        }),
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Approvals,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "running".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_approval_tab_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.last_event, "btw answer prompt opened");
    for ch in "cargo test".chars() {
        handle_side_question_prompt_key(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            &mut state,
        );
    }
    handle_side_question_prompt_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state,
    );
    assert_eq!(state.last_event, "btw answer saved");
    let loaded = store.load(&session.id().to_string()).unwrap();
    let updated = loaded.load_side_questions().unwrap();
    assert_eq!(updated[0].id, question.id);
    assert_eq!(updated[0].status, SideQuestionStatus::Answered);
    assert_eq!(updated[0].answer.as_deref(), Some("cargo test"));
}

#[test]
fn chat_ui_render_keeps_tools_out_of_primary_view() {
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: vec![ToolLogItem {
            title: "tool: read_file".to_string(),
            detail: "done".to_string(),
            expanded: false,
        }],
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: Some(0),
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Tools,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    terminal
        .draw(|frame| render_chat_ui(frame, &state))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Messages"));
    assert!(rendered.contains("Message Box"));
    assert!(!rendered.contains("Task Monitor"));
    assert!(!rendered.contains("Tools"));
    assert!(!rendered.contains("tool: read_file"));
}

#[test]
fn tools_tab_keys_and_mouse_keep_selected_tool_visible() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 9,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: (0..9)
            .map(|index| ToolLogItem {
                title: format!("tool: item-{index}"),
                detail: format!("detail-{index}"),
                expanded: false,
            })
            .collect(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: Some(0),
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Tools,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_tools_tab_key(
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.selected_tool, Some(TOOL_KEY_SCROLL_STEP));
    let rendered = format_task_monitor_text(&state, None, 6);
    assert!(rendered.contains("* > tool: item-5"));

    assert!(handle_tools_tab_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut state
    ));
    assert!(state.tool_log[TOOL_KEY_SCROLL_STEP].expanded);
    let rendered = format_task_monitor_text(&state, None, 8);
    assert!(rendered.contains("selected detail: tool: item-5"));
    assert!(rendered.contains("detail-5"));

    handle_tools_scroll_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        },
        Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 6,
        },
        true,
    );
    assert_eq!(
        state.selected_tool,
        Some(TOOL_KEY_SCROLL_STEP.saturating_sub(TOOL_MOUSE_SCROLL_STEP))
    );
    assert_eq!(state.result_scroll, 9);
    assert!(state.last_event.contains("tool selected: tool: item-2"));
}

#[test]
fn tools_tab_mouse_click_maps_focused_window_to_actual_tool() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: (0..9)
            .map(|index| ToolLogItem {
                title: format!("tool: item-{index}"),
                detail: format!("detail-{index}"),
                expanded: false,
            })
            .collect(),
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: Some(7),
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Tools,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let tools_area = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 6,
    };
    let visible_line = visible_panel_line_indices(
        tool_tab_lines(&state).len() + 1,
        tools_area.height,
        selected_tool_panel_line(&state),
    )
    .iter()
    .position(|line| *line == selected_tool_panel_line(&state))
    .unwrap();

    toggle_tool_at_row(
        &mut state,
        tools_area,
        tools_area.y + 1 + visible_line as u16,
    );

    assert_eq!(state.selected_tool, Some(7));
    assert!(state.tool_log[7].expanded);
    assert!(state.tool_log.iter().take(7).all(|item| !item.expanded));
    assert!(state.last_event.contains("expanded: tool: item-7"));
}

#[test]
fn tools_tab_expanded_selected_tool_shows_detail_preview_and_full_output_hint() {
    let long_detail = (0..12)
        .map(|index| format!("stderr line {index}: {}", "x".repeat(220)))
        .collect::<Vec<_>>()
        .join("\n");
    let state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: vec![ToolLogItem {
            title: "tool: run_tests [failed]".to_string(),
            detail: long_detail,
            expanded: true,
        }],
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: Some(0),
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Tools,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    let rendered = format_task_monitor_text(&state, None, 14);

    assert!(rendered.contains("selected detail: tool: run_tests [failed]"));
    assert!(rendered.contains("stderr line 0:"));
    assert!(
        rendered.contains("[detail truncated; Ctrl-O prefill full output, Ctrl-F failed tools]")
    );
    assert!(rendered.contains("* v tool: run_tests [failed]"));
}

#[test]
fn tools_tab_ctrl_shortcuts_prefill_session_tool_commands() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: vec![ToolLogItem {
            title: "tool: run_tests [failed]".to_string(),
            detail: "stderr".to_string(),
            expanded: true,
        }],
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: Some(0),
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Tools,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    assert!(handle_tools_tab_key(
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
        &mut state
    ));
    assert_eq!(state.input.buffer(), "/session tools --limit 20 --current");
    assert_eq!(state.last_event, "prefilled tool output command");

    state.input.clear();
    assert!(handle_tools_tab_key(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        &mut state
    ));
    assert_eq!(
        state.input.buffer(),
        "/session tools --failed --limit 20 --current"
    );
    assert_eq!(state.last_event, "prefilled failed tool output command");

    state.input.clear();
    assert!(!handle_tools_tab_key(
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
        &mut state
    ));
    assert_eq!(state.input.buffer(), "");
}

#[test]
fn tools_tab_shows_visible_session_tool_actions() {
    let state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: vec![ToolLogItem {
            title: "tool: run_tests [failed]".to_string(),
            detail: "stderr".to_string(),
            expanded: false,
        }],
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: Some(0),
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Tools,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };

    let rendered = format_task_monitor_text(&state, None, 12);

    assert!(rendered.contains("tool actions"));
    assert!(rendered.contains("> /session tools --limit 20 --current (edit)"));
    assert!(rendered.contains("  /session tools --failed --limit 20 --current (edit)"));
}

#[test]
fn tools_tab_mouse_click_prefills_visible_tool_action_without_toggling_tool() {
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: Vec::new(),
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: vec![ToolLogItem {
            title: "tool: run_tests [failed]".to_string(),
            detail: "stderr".to_string(),
            expanded: false,
        }],
        resume_picker: None,
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: Some(0),
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Tools,
        selected_approval: 0,
        running: true,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let tools_area = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 12,
    };
    let rendered = format_task_monitor_text(&state, None, tools_area.height);
    let failed_action_line = rendered
        .lines()
        .position(|line| line.contains("/session tools --failed --limit 20 --current"))
        .expect("failed tool action should be visible");
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    assert!(activate_monitor_quick_action_at_row(
        &mut state,
        tools_area,
        tools_area.y + 1 + failed_action_line as u16,
        &progress_tx,
        &done_tx,
    ));

    assert_eq!(
        state.input.buffer(),
        "/session tools --failed --limit 20 --current"
    );
    assert!(!state.tool_log[0].expanded);
    assert!(state.last_event.contains("quick action ready for edit"));
}

#[test]
fn resume_picker_preview_shows_selected_session_context() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.rename("compiler repair").unwrap();
    session.append_message("user", "继续上次任务").unwrap();
    session
        .append_message("assistant", "已完成 lv4，下一步处理数组参数")
        .unwrap();
    session
        .write_summary("lv4 已通过，继续 lv5 数组和函数调用")
        .unwrap();
    let picker = ResumePicker::new(store.list().unwrap());

    let preview = format_resume_preview_text(&picker, 20);
    assert!(preview.contains("title: compiler repair"));
    assert!(preview.contains("activity: messages=2"));
    assert!(preview.contains("summary: lv4 已通过"));
    assert!(preview.contains("recent messages:"));
    assert!(preview.contains("assistant: 已完成 lv4"));

    let backend = TestBackend::new(120, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            render_resume_picker(
                frame,
                Rect {
                    x: 0,
                    y: 0,
                    width: 120,
                    height: 12,
                },
                &picker,
            )
        })
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Resume Preview"));
    assert!(rendered.contains("compiler repair"));
}

#[test]
fn resume_picker_filters_sessions_by_metadata() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut compiler = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    compiler.rename("compiler repair").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let mut kimi = store
        .create(
            dir.path(),
            "kimi".to_string(),
            Some("kimi-for-coding".to_string()),
        )
        .unwrap();
    kimi.rename("frontend polish").unwrap();

    let mut picker = ResumePicker::new(store.list().unwrap());
    assert_eq!(picker.filtered_len(), 2);

    picker.push_query_str("compiler");
    assert_eq!(picker.filtered_len(), 1);
    assert_eq!(picker.selected_session().unwrap().id, compiler.id());
    assert!(format_resume_preview_text(&picker, 12).contains("compiler repair"));

    picker.query = "kimi-for".to_string();
    picker.clamp_selected();
    assert_eq!(picker.filtered_len(), 1);
    assert_eq!(picker.selected_session().unwrap().id, kimi.id());

    picker.query = short_id(&compiler.id().to_string()).to_string();
    picker.clamp_selected();
    assert_eq!(picker.selected_session().unwrap().id, compiler.id());

    picker.query = "missing".to_string();
    picker.clamp_selected();
    assert_eq!(picker.filtered_len(), 0);
    assert!(format_resume_preview_text(&picker, 12).contains("no sessions match"));
}

#[test]
fn resume_picker_mouse_selects_and_scrolls_without_falling_through() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    for index in 0..5 {
        let mut session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        session.rename(format!("session {index}")).unwrap();
    }

    let picker = ResumePicker::new(store.list().unwrap());
    let mut state = TuiState {
        runtime: None,
        active_session: None,
        input: MessageBox::new(),
        chat: vec![ChatLine {
            role: "deepcli".to_string(),
            content: (0..20)
                .map(|index| format!("line-{index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }],
        transcript_scroll: 0,
        result_scroll: 0,
        workspace_changes: None,
        workspace_changes_checked_at: None,
        tool_log: Vec::new(),
        resume_picker: Some(picker),
        credential_prompt: None,
        side_question_prompt: None,
        dialog: None,
        selected_tool: None,
        selected_command: 0,
        selected_change: 0,
        change_patch_scroll: 0,
        monitor_tab: MonitorTab::Result,
        selected_approval: 0,
        running: false,
        exit_requested: false,
        last_event: "ready".to_string(),
        streaming_assistant: None,
        worker: None,
    };
    let area = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 24,
    };
    let layout = chat_ui_layout(area);
    let (list_area, _) = resume_picker_layout(layout.input);
    let (progress_tx, _progress_rx) = mpsc::channel();
    let (done_tx, _done_rx) = mpsc::channel();

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: list_area.x + 2,
            row: list_area.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    let picker = state.resume_picker.as_ref().unwrap();
    assert_eq!(picker.selected, RESUME_PICKER_MOUSE_SCROLL_STEP);
    assert_eq!(state.result_scroll, 0);
    assert!(state.last_event.starts_with("resume selected:"));

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: list_area.x + 2,
            row: list_area.y + 2,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    let picker = state.resume_picker.as_ref().unwrap();
    assert_eq!(picker.selected, 2);

    handle_tui_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: list_area.x + 2,
            row: list_area.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &progress_tx,
        &done_tx,
        area,
    );
    let picker = state.resume_picker.as_ref().unwrap();
    assert_eq!(picker.selected, 0);
}

#[test]
fn session_messages_are_rendered_as_chat_history() {
    let lines = session_messages_to_chat_lines(vec![
        SessionMessage {
            role: "user".to_string(),
            content: "继续上次任务".to_string(),
            created_at: chrono::Utc::now(),
        },
        SessionMessage {
            role: "assistant".to_string(),
            content: "已恢复上下文".to_string(),
            created_at: chrono::Utc::now(),
        },
    ]);

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].role, "你");
    assert_eq!(lines[0].content, "继续上次任务");
    assert_eq!(lines[1].role, "deepcli");
    assert_eq!(lines[1].content, "已恢复上下文");
}

#[test]
fn resumed_tui_history_loads_all_persisted_messages() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let session_id = session.id().to_string();
    for index in 0..40 {
        session
            .append_message("user", format!("message-{index}"))
            .unwrap();
    }
    let runtime = AgentRuntime::new(
        AppConfig::default(),
        RuntimeOptions {
            workspace: dir.path().to_path_buf(),
            provider: None,
            model: None,
            assume_yes: true,
            resume_session: Some(session_id),
            stream_output: false,
        },
    )
    .unwrap();

    let lines = chat_lines_from_runtime(&runtime).unwrap();

    assert_eq!(lines.len(), 40);
    assert_eq!(lines[0].content, "message-0");
    assert_eq!(lines[39].content, "message-39");
}

#[test]
fn ratatui_dashboard_renders_without_panic() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let snapshot = TuiSnapshot {
        session_id: "session".to_string(),
        provider: "deepseek".to_string(),
        model: "deepseek-chat".to_string(),
        state: "planning".to_string(),
        plan_steps: vec!["read context".to_string(), "run tests".to_string()],
        token_usage: "0/160000".to_string(),
        last_event: "initialized".to_string(),
    };
    terminal
        .draw(|frame| render_dashboard(frame, frame.area(), &snapshot))
        .unwrap();
}
