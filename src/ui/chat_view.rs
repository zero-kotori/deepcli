use super::{
    approval_prompt_view_for_state, command_palette_auto_popup_enabled, compact_ui_text,
    credential_prompt_hidden_body, credential_prompt_hidden_cursor, render_command_palette,
    render_dialog, render_resume_picker, slash_command_suggestions_for_state, ChatLine, TuiState,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use std::ops::Range;
use unicode_width::UnicodeWidthChar;

pub(super) struct ChatUiLayout {
    #[allow(dead_code)]
    pub(super) header: Rect,
    pub(super) transcript: Rect,
    pub(super) tools: Rect,
    pub(super) input: Rect,
}

pub(super) fn chat_ui_layout(area: Rect) -> ChatUiLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(5)])
        .split(area);
    let hidden = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 0,
    };
    ChatUiLayout {
        header: hidden,
        transcript: chunks[0],
        tools: hidden,
        input: chunks[1],
    }
}

fn transcript_visible_message_count(area: Rect) -> usize {
    area.height.saturating_sub(2).max(1) as usize
}

fn transcript_inner_width(area: Rect) -> usize {
    area.width.saturating_sub(2).max(1) as usize
}

pub(super) fn clamp_transcript_scroll_to_area(state: &mut TuiState, area: Rect) {
    let visible = transcript_visible_message_count(area);
    let width = transcript_inner_width(area);
    let transcript = format_full_transcript_text(&state.chat);
    let visual_lines = wrap_transcript_lines(&transcript, width);
    let max_scroll = visual_lines.len().saturating_sub(visible.max(1));
    state.transcript_scroll = state.transcript_scroll.min(max_scroll);
}

#[cfg(test)]
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

#[cfg(test)]
pub(super) fn format_transcript_text(chat: &[ChatLine], scroll: usize, visible: usize) -> String {
    let (start, end, _) = transcript_window(chat.len(), scroll, visible);
    chat[start..end]
        .iter()
        .map(|line| format!("{}: {}", line.role, line.content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_visible_transcript_text(
    chat: &[ChatLine],
    scroll: usize,
    visible: usize,
    width: usize,
) -> VisibleTranscript {
    let visible = visible.max(1);
    let needed = visible.saturating_add(scroll);
    let (visual_lines, complete) = collect_visible_transcript_tail(chat, needed, width);
    if !complete {
        let end = visual_lines.len().saturating_sub(scroll);
        let start = end.saturating_sub(visible);
        return VisibleTranscript {
            text: visual_lines[start..end].join("\n"),
            scroll,
        };
    }

    let (start, end, scroll) = transcript_visual_window(visual_lines.len(), scroll, visible);
    VisibleTranscript {
        text: visual_lines[start..end].join("\n"),
        scroll,
    }
}

struct VisibleTranscript {
    text: String,
    scroll: usize,
}

fn collect_visible_transcript_tail(
    chat: &[ChatLine],
    needed: usize,
    width: usize,
) -> (Vec<String>, bool) {
    if chat.is_empty() || needed == 0 {
        return (Vec::new(), true);
    }

    let mut reversed = Vec::new();
    for (index, line) in chat.iter().enumerate().rev() {
        let message = format!("{}: {}", line.role, line.content);
        let wrapped = wrap_transcript_lines(&message, width);
        for visual_line in wrapped.into_iter().rev() {
            reversed.push(visual_line);
            if reversed.len() >= needed {
                reversed.reverse();
                return (reversed, false);
            }
        }
        if index > 0 {
            reversed.push(String::new());
            if reversed.len() >= needed {
                reversed.reverse();
                return (reversed, false);
            }
        }
    }
    reversed.reverse();
    (reversed, true)
}

fn format_messages_title(scroll: usize) -> String {
    if scroll == 0 {
        "Messages (PageUp history)".to_string()
    } else {
        format!("Messages (scroll={scroll}; PageDown latest, Ctrl-End bottom)")
    }
}

fn format_full_transcript_text(chat: &[ChatLine]) -> String {
    chat.iter()
        .map(|line| format!("{}: {}", line.role, line.content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn transcript_visual_window(total: usize, scroll: usize, visible: usize) -> (usize, usize, usize) {
    if total == 0 {
        return (0, 0, 0);
    }
    let visible = visible.max(1);
    let max_scroll = total.saturating_sub(visible);
    let scroll = scroll.min(max_scroll);
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(visible);
    (start, end, scroll)
}

fn wrap_transcript_lines(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let width = width.max(1);
    let mut lines = Vec::new();
    for line in text.split('\n') {
        append_wrapped_line(&mut lines, line, width);
    }
    lines
}

fn append_wrapped_line(lines: &mut Vec<String>, line: &str, width: usize) {
    if line.is_empty() {
        lines.push(String::new());
        return;
    }

    let mut current = String::new();
    let mut column = 0usize;
    for ch in line.chars() {
        let char_width = ch.width().unwrap_or(0).max(1);
        if column > 0 && column.saturating_add(char_width) > width {
            lines.push(current);
            current = String::new();
            column = 0;
        }
        current.push(ch);
        column = column.saturating_add(char_width.min(width));
        if column >= width {
            lines.push(current);
            current = String::new();
            column = 0;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
}

pub(super) fn render_chat_ui(frame: &mut Frame<'_>, state: &TuiState) {
    let areas = chat_ui_layout(frame.area());

    let visible_messages = transcript_visible_message_count(areas.transcript);
    let transcript_width = transcript_inner_width(areas.transcript);
    let transcript = format_visible_transcript_text(
        &state.chat,
        state.transcript_scroll,
        visible_messages,
        transcript_width,
    );
    let messages_title = format_messages_title(transcript.scroll);
    frame.render_widget(
        Paragraph::new(transcript.text)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(messages_title)),
        areas.transcript,
    );

    if state.credential_prompt.is_none() && state.side_question_prompt.is_none() {
        if let Some(picker) = &state.resume_picker {
            render_resume_picker(frame, areas.input, picker);
            return;
        } else if render_dialog(frame, areas.input, state) {
            return;
        } else if let Some(prompt) = approval_prompt_view_for_state(state, areas.input.height) {
            frame.render_widget(
                Paragraph::new(prompt.body)
                    .wrap(Wrap { trim: false })
                    .block(Block::default().borders(Borders::ALL).title(prompt.title)),
                areas.input,
            );
            return;
        } else if command_palette_auto_popup_enabled() {
            if let Some(suggestions) =
                slash_command_suggestions_for_state(state.input.buffer(), state.running)
            {
                render_command_palette(frame, areas.input, state, &suggestions);
                return;
            }
        }
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
    let (input_body, input_cursor, input_selection) = if let Some(prompt) = &state.credential_prompt
    {
        (
            credential_prompt_hidden_body(prompt),
            credential_prompt_hidden_cursor(prompt),
            None,
        )
    } else if let Some(prompt) = &state.side_question_prompt {
        (
            prompt.input.buffer().to_string(),
            prompt.input.cursor(),
            prompt.input.selection_range(),
        )
    } else {
        (
            state.input.buffer().to_string(),
            state.input.cursor(),
            state.input.selection_range(),
        )
    };
    let input_scroll = message_box_vertical_scroll(&input_body, input_cursor, areas.input);
    let input_paragraph = if let Some(selection) = input_selection {
        Paragraph::new(message_box_selection_lines(&input_body, selection))
    } else {
        Paragraph::new(input_body.as_str())
    };
    frame.render_widget(
        input_paragraph
            .wrap(Wrap { trim: false })
            .scroll((input_scroll, 0))
            .block(Block::default().borders(Borders::ALL).title(input_title)),
        areas.input,
    );
    frame.set_cursor_position(message_box_cursor_position_with_scroll(
        &input_body,
        input_cursor,
        areas.input,
        input_scroll,
    ));
}

#[cfg(test)]
pub(super) fn message_box_cursor_position(buffer: &str, cursor: usize, area: Rect) -> Position {
    let scroll = message_box_vertical_scroll(buffer, cursor, area);
    message_box_cursor_position_with_scroll(buffer, cursor, area, scroll)
}

fn message_box_cursor_position_with_scroll(
    buffer: &str,
    cursor: usize,
    area: Rect,
    scroll: u16,
) -> Position {
    let inner_x = area.x.saturating_add(1);
    let inner_y = area.y.saturating_add(1);
    let inner_height = area.height.saturating_sub(2).max(1);
    let inner_width = area.width.saturating_sub(2).max(1);
    let (row, column) = message_box_cursor_offset(buffer, cursor, area);

    Position::new(
        inner_x.saturating_add(column.min(inner_width.saturating_sub(1))),
        inner_y.saturating_add(
            row.saturating_sub(scroll)
                .min(inner_height.saturating_sub(1)),
        ),
    )
}

fn message_box_vertical_scroll(buffer: &str, cursor: usize, area: Rect) -> u16 {
    let inner_height = area.height.saturating_sub(2).max(1);
    let (row, _) = message_box_cursor_offset(buffer, cursor, area);
    row.saturating_sub(inner_height.saturating_sub(1))
}

fn message_box_cursor_offset(buffer: &str, cursor: usize, area: Rect) -> (u16, u16) {
    let inner_width = area.width.saturating_sub(2).max(1);
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

    (row, column.min(inner_width.saturating_sub(1)))
}

fn message_box_selection_lines(buffer: &str, selection: Range<usize>) -> Vec<Line<'static>> {
    if buffer.is_empty() {
        return vec![Line::from(String::new())];
    }

    let mut lines = Vec::new();
    let mut line_start = 0usize;
    for segment in buffer.split_inclusive('\n') {
        let line = segment.strip_suffix('\n').unwrap_or(segment);
        lines.push(Line::from(message_box_selection_spans(
            line, line_start, &selection,
        )));
        line_start += segment.len();
    }
    if buffer.ends_with('\n') {
        lines.push(Line::from(String::new()));
    }
    lines
}

fn message_box_selection_spans(
    line: &str,
    line_start: usize,
    selection: &Range<usize>,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_selected: Option<bool> = None;

    for (offset, ch) in line.char_indices() {
        let selected = selection.contains(&(line_start + offset));
        if current_selected.is_some_and(|value| value != selected) {
            spans.push(selection_span(std::mem::take(&mut current), !selected));
        }
        current_selected = Some(selected);
        current.push(ch);
    }

    if !current.is_empty() {
        spans.push(selection_span(current, current_selected.unwrap_or(false)));
    }
    spans
}

fn selection_span(text: String, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(text, Style::default().fg(Color::Black).bg(Color::White))
    } else {
        Span::raw(text)
    }
}
