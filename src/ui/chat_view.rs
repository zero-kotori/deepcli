use super::monitor_shell::render_task_monitor;
use super::{
    compact_ui_text, credential_prompt_hidden_body, credential_prompt_hidden_cursor,
    header_status_for_state, render_command_palette, render_resume_picker,
    slash_command_suggestions_for_state, ChatLine, TuiState,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthChar;

pub(super) struct ChatUiLayout {
    pub(super) header: Rect,
    pub(super) transcript: Rect,
    pub(super) tools: Rect,
    pub(super) input: Rect,
}

pub(super) fn chat_ui_layout(area: Rect) -> ChatUiLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(9),
            Constraint::Length(5),
        ])
        .split(area);
    ChatUiLayout {
        header: chunks[0],
        transcript: chunks[1],
        tools: chunks[2],
        input: chunks[3],
    }
}

fn transcript_visible_message_count(area: Rect) -> usize {
    area.height.saturating_sub(2).max(1) as usize
}

fn transcript_window(total: usize, scroll: usize, visible: usize) -> (usize, usize, usize) {
    if total == 0 {
        return (0, 0, 0);
    }
    let visible = visible.max(1);
    let max_start = total.saturating_sub(visible);
    let scroll = scroll.min(max_start);
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(visible);
    (start, end, scroll)
}

pub(super) fn format_transcript_text(chat: &[ChatLine], scroll: usize, visible: usize) -> String {
    let (start, end, _) = transcript_window(chat.len(), scroll, visible);
    chat[start..end]
        .iter()
        .map(|line| format!("{}: {}", line.role, line.content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_messages_title(state: &TuiState, visible: usize) -> String {
    let (_, _, scroll) = transcript_window(state.chat.len(), state.transcript_scroll, visible);
    if scroll == 0 {
        "Messages (PageUp history)".to_string()
    } else {
        format!("Messages (scroll={scroll}; PageDown latest, Ctrl-End bottom)")
    }
}

pub(super) fn render_chat_ui(frame: &mut Frame<'_>, state: &TuiState) {
    let areas = chat_ui_layout(frame.area());
    let header_status = header_status_for_state(state);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "deepcli",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  title={} session={} provider={} model={} state={}",
            header_status.title,
            header_status.session,
            header_status.provider,
            header_status.model,
            header_status.state
        )),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(header, areas.header);

    let visible_messages = transcript_visible_message_count(areas.transcript);
    let transcript = format_transcript_text(&state.chat, state.transcript_scroll, visible_messages);
    let messages_title = format_messages_title(state, visible_messages);
    frame.render_widget(
        Paragraph::new(transcript)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(messages_title)),
        areas.transcript,
    );

    if let Some(picker) = &state.resume_picker {
        render_resume_picker(frame, areas.tools, picker);
    } else if let Some(suggestions) =
        slash_command_suggestions_for_state(state.input.buffer(), state.running)
    {
        render_command_palette(frame, areas.tools, state, &suggestions);
    } else {
        render_task_monitor(frame, areas.tools, state);
    }

    let input_title = if let Some(prompt) = &state.credential_prompt {
        if prompt.force {
            "Credential Input (hidden, Enter overwrite, Esc cancel)".to_string()
        } else {
            "Credential Input (hidden, Enter save, Esc cancel)".to_string()
        }
    } else if let Some(prompt) = &state.side_question_prompt {
        format!(
            "BTW Answer (Enter save, Shift-Enter newline, Esc cancel): {}",
            compact_ui_text(&prompt.question, 54)
        )
    } else if state.running {
        "Message Box (running; Ctrl-C after completion to exit)".to_string()
    } else {
        "Message Box (Enter send, Shift-Enter newline, Esc exit)".to_string()
    };
    let input_body = if let Some(prompt) = &state.credential_prompt {
        credential_prompt_hidden_body(prompt)
    } else if let Some(prompt) = &state.side_question_prompt {
        prompt.input.buffer().to_string()
    } else {
        state.input.buffer().to_string()
    };
    frame.render_widget(
        Paragraph::new(input_body)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(input_title)),
        areas.input,
    );
    if let Some(prompt) = &state.credential_prompt {
        let hidden_body = credential_prompt_hidden_body(prompt);
        frame.set_cursor_position(message_box_cursor_position(
            &hidden_body,
            credential_prompt_hidden_cursor(prompt),
            areas.input,
        ));
    } else if let Some(prompt) = &state.side_question_prompt {
        frame.set_cursor_position(message_box_cursor_position(
            prompt.input.buffer(),
            prompt.input.cursor(),
            areas.input,
        ));
    } else {
        frame.set_cursor_position(message_box_cursor_position(
            state.input.buffer(),
            state.input.cursor(),
            areas.input,
        ));
    }
}

pub(super) fn message_box_cursor_position(buffer: &str, cursor: usize, area: Rect) -> Position {
    let inner_x = area.x.saturating_add(1);
    let inner_y = area.y.saturating_add(1);
    let inner_width = area.width.saturating_sub(2).max(1);
    let inner_height = area.height.saturating_sub(2).max(1);
    let cursor = cursor.min(buffer.len());
    let mut row = 0u16;
    let mut column = 0u16;

    for ch in buffer[..cursor].chars() {
        if ch == '\n' {
            row = row.saturating_add(1);
            column = 0;
            continue;
        }
        let width = ch.width().unwrap_or(0).max(1) as u16;
        let width = width.min(inner_width);
        if column.saturating_add(width) > inner_width {
            row = row.saturating_add(1);
            column = 0;
        }
        column = column.saturating_add(width);
        if column >= inner_width {
            row = row.saturating_add(column / inner_width);
            column %= inner_width;
        }
    }

    Position::new(
        inner_x.saturating_add(column.min(inner_width.saturating_sub(1))),
        inner_y.saturating_add(row.min(inner_height.saturating_sub(1))),
    )
}
