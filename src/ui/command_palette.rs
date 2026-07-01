use super::{rect_contains, rect_content_row_contains, TuiState};
use crate::commands::{CommandHelpSummary, CommandRouter};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

pub(super) const COMMAND_PALETTE_MATCH_LIMIT: usize = 16;
pub(super) const RUNNING_SAFE_PALETTE_PRIORITY: &[&str] = &[
    "/help",
    "/status",
    "/usage",
    "/logs",
    "/trace",
    "/stop",
    "/quit",
    "/selftest",
    "/preflight",
    "/completion",
    "/round",
    "/scorecard",
    "/opportunities",
    "/benchmark",
    "/recipes",
    "/privacy",
    "/fork",
    "/approval",
    "/git",
    "/session",
    "/cleanup",
    "/terminal",
    "/btw",
];

pub(super) fn handle_command_palette_key(key: KeyEvent, state: &mut TuiState) -> bool {
    let Some(suggestions) =
        slash_command_suggestions_for_state(state.input.buffer(), state.running)
    else {
        return false;
    };
    if suggestions.is_empty() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            state.selected_command = state.selected_command.saturating_sub(1);
            true
        }
        KeyCode::Down => {
            state.selected_command = (state.selected_command + 1).min(suggestions.len() - 1);
            true
        }
        KeyCode::Tab => {
            complete_selected_command(state, &suggestions);
            true
        }
        _ => false,
    }
}

pub(super) fn handle_command_palette_mouse_for_state(
    state: &mut TuiState,
    mouse: MouseEvent,
    tools_area: Rect,
) -> bool {
    let Some(suggestions) =
        slash_command_suggestions_for_state(state.input.buffer(), state.running)
    else {
        return false;
    };
    if suggestions.is_empty() || !rect_contains(tools_area, mouse.column, mouse.row) {
        return false;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            state.selected_command = state.selected_command.saturating_sub(1);
            state.last_event =
                command_palette_selection_event(state.selected_command, &suggestions);
            true
        }
        MouseEventKind::ScrollDown => {
            state.selected_command = (state.selected_command + 1).min(suggestions.len() - 1);
            state.last_event =
                command_palette_selection_event(state.selected_command, &suggestions);
            true
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(index) = clicked_command_palette_index(
                &suggestions,
                state.selected_command,
                state.running,
                tools_area,
                mouse.column,
                mouse.row,
            ) else {
                return false;
            };
            state.selected_command = index;
            complete_selected_command(state, &suggestions);
            true
        }
        _ => false,
    }
}

fn clicked_command_palette_index(
    suggestions: &[CommandHelpSummary],
    selected: usize,
    running: bool,
    area: Rect,
    column: u16,
    row: u16,
) -> Option<usize> {
    if suggestions.is_empty()
        || !rect_contains(area, column, row)
        || !rect_content_row_contains(area, row)
    {
        return None;
    }
    let content_row = row.saturating_sub(area.y + 1) as usize;
    if content_row != command_palette_matches_line_index(running) {
        return None;
    }
    let column = column.saturating_sub(area.x + 1) as usize;
    let mut offset = "matches: ".len();
    for (index, summary) in suggestions.iter().enumerate() {
        let token = command_palette_match_token(index, selected, summary);
        let end = offset + token.len();
        if (offset..end).contains(&column) {
            return Some(index);
        }
        offset = end + 2;
    }
    None
}

fn command_palette_selection_event(selected: usize, suggestions: &[CommandHelpSummary]) -> String {
    let selected = selected.min(suggestions.len().saturating_sub(1));
    suggestions
        .get(selected)
        .map(|summary| format!("command selected: {}", summary.name))
        .unwrap_or_else(|| "command selection unavailable".to_string())
}

pub(super) fn complete_selected_command(state: &mut TuiState, suggestions: &[CommandHelpSummary]) {
    if let Some(selected) = suggestions.get(state.selected_command.min(suggestions.len() - 1)) {
        state.input.set_buffer(format!("{} ", selected.name));
        state.selected_command = 0;
        state.last_event = format!("completed {}", selected.name);
    }
}

pub(super) fn clamp_selected_command(state: &mut TuiState) {
    if let Some(suggestions) =
        slash_command_suggestions_for_state(state.input.buffer(), state.running)
    {
        if suggestions.is_empty() {
            state.selected_command = 0;
        } else {
            state.selected_command = state.selected_command.min(suggestions.len() - 1);
        }
    } else {
        state.selected_command = 0;
    }
}

pub(super) fn slash_command_suggestions_for_state(
    input: &str,
    running: bool,
) -> Option<Vec<CommandHelpSummary>> {
    let query = slash_command_query(input)?;
    let summaries = CommandRouter::help_summaries();
    let exact = summaries
        .iter()
        .copied()
        .filter(|summary| summary.name == query)
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return Some(exact);
    }

    let mut matches = summaries
        .into_iter()
        .filter(|summary| summary.name.starts_with(&query))
        .collect();
    if running {
        matches = prioritize_running_safe_suggestions(matches);
    }
    matches.truncate(COMMAND_PALETTE_MATCH_LIMIT);
    Some(matches)
}

fn prioritize_running_safe_suggestions(
    suggestions: Vec<CommandHelpSummary>,
) -> Vec<CommandHelpSummary> {
    let mut running_safe = Vec::new();
    let mut other = Vec::new();
    for suggestion in suggestions {
        if suggestion.running_safe {
            running_safe.push(suggestion);
        } else {
            other.push(suggestion);
        }
    }
    running_safe.sort_by_key(|summary| running_safe_palette_priority(summary.name));
    running_safe.extend(other);
    running_safe
}

pub(super) fn running_safe_palette_priority(name: &str) -> usize {
    RUNNING_SAFE_PALETTE_PRIORITY
        .iter()
        .position(|candidate| *candidate == name)
        .unwrap_or(usize::MAX)
}

fn slash_command_query(input: &str) -> Option<String> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') || trimmed.contains('\n') {
        return None;
    }
    let first_token = trimmed.split_whitespace().next().unwrap_or("/");
    if first_token == "/" {
        Some("/".to_string())
    } else {
        Some(first_token.to_string())
    }
}

pub(super) fn render_command_palette(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    suggestions: &[CommandHelpSummary],
) {
    let text = format_command_palette_text(
        suggestions,
        state.selected_command,
        area.height,
        state.running,
    );
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Command Help (Up/Down/wheel select, Tab/click complete, Esc dismiss)"),
        ),
        area,
    );
}

pub(super) fn format_command_palette_text(
    suggestions: &[CommandHelpSummary],
    selected: usize,
    height: u16,
    running: bool,
) -> String {
    if suggestions.is_empty() {
        return "no matching commands".to_string();
    }
    let selected = selected.min(suggestions.len() - 1);
    let active = suggestions[selected];
    let mut lines = Vec::new();
    let matches = suggestions
        .iter()
        .enumerate()
        .map(|(index, summary)| command_palette_match_token(index, selected, summary))
        .collect::<Vec<_>>()
        .join("  ");
    if running {
        lines.push("running mode: (run) commands execute now; others wait".to_string());
    }
    lines.push(format!("matches: {matches}"));
    lines.push(format!("selected: {}", active.listing));
    if active.running_safe {
        lines.push("running-safe: yes".to_string());
    }
    lines.push(active.summary.to_string());
    lines.push("usage:".to_string());
    lines.extend(active.usage.iter().map(|usage| format!("  {usage}")));
    if !active.examples.is_empty() {
        lines.push("examples:".to_string());
        lines.extend(
            active
                .examples
                .iter()
                .take(2)
                .map(|example| format!("  {example}")),
        );
    }
    if let Some(note) = active.notes.first() {
        lines.push(format!("note: {note}"));
    }

    let visible = height.saturating_sub(2) as usize;
    if visible == 0 || lines.len() <= visible {
        lines.join("\n")
    } else {
        lines.truncate(visible.saturating_sub(1));
        lines.push("[more: run /help all or /help <command>]".to_string());
        lines.join("\n")
    }
}

pub(super) fn command_palette_match_token(
    index: usize,
    selected: usize,
    summary: &CommandHelpSummary,
) -> String {
    let marker = if summary.running_safe { " (run)" } else { "" };
    if index == selected {
        format!(">{}{}", summary.name, marker)
    } else {
        format!("{}{}", summary.name, marker)
    }
}

pub(super) fn command_palette_matches_line_index(running: bool) -> usize {
    if running {
        1
    } else {
        0
    }
}
