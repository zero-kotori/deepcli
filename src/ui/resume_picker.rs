use super::monitor_shell::truncate_panel_lines;
use super::{
    apply_resume_result, chat_lines_from_runtime, compact_ui_text, rect_contains,
    rect_content_row_contains, short_id, TuiState, RESUME_PICKER_MOUSE_SCROLL_STEP,
};
use crate::commands::list_resumable_sessions;
use crate::session::{SessionMetadata, SessionStore};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::io::{self, Write};
use std::path::Path;

pub(super) struct ResumePicker {
    pub(super) sessions: Vec<SessionMetadata>,
    pub(super) selected: usize,
    pub(super) query: String,
}

impl ResumePicker {
    pub(super) fn new(sessions: Vec<SessionMetadata>) -> Self {
        Self {
            sessions,
            selected: 0,
            query: String::new(),
        }
    }

    pub(super) fn filtered_indices(&self) -> Vec<usize> {
        self.sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| {
                session_matches_resume_query(session, &self.query).then_some(index)
            })
            .collect()
    }

    pub(super) fn filtered_len(&self) -> usize {
        self.sessions
            .iter()
            .filter(|session| session_matches_resume_query(session, &self.query))
            .count()
    }

    pub(super) fn selected_session(&self) -> Option<&SessionMetadata> {
        let indices = self.filtered_indices();
        indices
            .get(self.selected)
            .and_then(|index| self.sessions.get(*index))
    }

    pub(super) fn clamp_selected(&mut self) {
        let len = self.filtered_len();
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len - 1);
        }
    }

    pub(super) fn move_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub(super) fn move_next(&mut self) {
        let len = self.filtered_len();
        if len > 0 {
            self.selected = (self.selected + 1).min(len - 1);
        }
    }

    fn move_previous_by(&mut self, amount: usize) {
        self.selected = self.selected.saturating_sub(amount);
    }

    fn move_next_by(&mut self, amount: usize) {
        let len = self.filtered_len();
        if len > 0 {
            self.selected = self.selected.saturating_add(amount).min(len - 1);
        }
    }

    pub(super) fn move_home(&mut self) {
        self.selected = 0;
    }

    pub(super) fn move_end(&mut self) {
        self.selected = self.filtered_len().saturating_sub(1);
    }

    pub(super) fn push_query_char(&mut self, ch: char) {
        self.query.push(ch);
        self.selected = 0;
    }

    pub(super) fn push_query_str(&mut self, text: &str) {
        self.query.push_str(text);
        self.selected = 0;
    }

    pub(super) fn pop_query_char(&mut self) {
        self.query.pop();
        self.clamp_selected();
    }

    pub(super) fn visible_start(&self, visible: usize) -> usize {
        if visible == 0 {
            0
        } else {
            self.selected.saturating_sub(visible.saturating_sub(1))
        }
    }
}

fn session_matches_resume_query(session: &SessionMetadata, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let id = session.id.to_string();
    let short = short_id(&id);
    [
        session.title.as_deref().unwrap_or("<untitled>"),
        id.as_str(),
        short,
        session.provider.as_str(),
        session.model.as_deref().unwrap_or("<unset>"),
    ]
    .iter()
    .any(|value| value.to_lowercase().contains(&query))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeSelection {
    Selected(String),
    NoSessions,
    Cancelled,
}

pub fn pick_resume_session(workspace: &Path) -> Result<ResumeSelection> {
    let sessions = list_resumable_sessions(workspace)?;
    if sessions.is_empty() {
        return Ok(ResumeSelection::NoSessions);
    }

    run_resume_picker_loop(&sessions)
}

fn run_resume_picker_loop(sessions: &[SessionMetadata]) -> Result<ResumeSelection> {
    print_native_resume_sessions(sessions)?;
    let stdin = io::stdin();
    loop {
        print!("resume> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if stdin.read_line(&mut input)? == 0 {
            return Ok(ResumeSelection::Cancelled);
        }

        let input = input.trim();
        if input.is_empty() {
            return Ok(ResumeSelection::Selected(sessions[0].id.to_string()));
        }
        if matches!(input, "q" | "quit" | "cancel") {
            return Ok(ResumeSelection::Cancelled);
        }
        if let Some(session_id) = native_resume_selection(&sessions, input) {
            return Ok(ResumeSelection::Selected(session_id));
        }

        println!("no matching session; enter a number, unique id prefix, or q");
    }
}

fn print_native_resume_sessions(sessions: &[SessionMetadata]) -> io::Result<()> {
    println!("Resumable sessions:");
    for (index, session) in sessions.iter().enumerate() {
        let title = session.title.as_deref().unwrap_or("<untitled>");
        let model = session.model.as_deref().unwrap_or("<unset>");
        println!(
            "{:>2}. {}  {}  {}  {}",
            index + 1,
            short_id(&session.id.to_string()),
            compact_ui_text(title, 50),
            session.provider,
            model
        );
    }
    println!("Enter a number or unique id prefix; blank selects the first session; q cancels.");
    Ok(())
}

fn native_resume_selection(sessions: &[SessionMetadata], input: &str) -> Option<String> {
    if let Ok(number) = input.parse::<usize>() {
        if (1..=sessions.len()).contains(&number) {
            return Some(sessions[number - 1].id.to_string());
        }
    }

    let mut matches = sessions
        .iter()
        .filter(|session| session.id.to_string().starts_with(input))
        .map(|session| session.id.to_string());
    let selected = matches.next()?;
    matches.next().is_none().then_some(selected)
}

fn resume_filter_accepts_char(key: KeyEvent, ch: char) -> bool {
    !ch.is_control()
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
}

pub(super) fn handle_resume_picker_key(key: KeyEvent, state: &mut TuiState) {
    match key.code {
        KeyCode::Up | KeyCode::Left => {
            if let Some(picker) = &mut state.resume_picker {
                picker.move_previous();
            }
        }
        KeyCode::Down | KeyCode::Right => {
            if let Some(picker) = &mut state.resume_picker {
                picker.move_next();
            }
        }
        KeyCode::Home => {
            if let Some(picker) = &mut state.resume_picker {
                picker.move_home();
            }
        }
        KeyCode::End => {
            if let Some(picker) = &mut state.resume_picker {
                picker.move_end();
            }
        }
        KeyCode::Backspace => {
            if let Some(picker) = &mut state.resume_picker {
                picker.pop_query_char();
            }
        }
        KeyCode::Enter => confirm_resume_selection(state),
        KeyCode::Char('q') => {
            if state
                .resume_picker
                .as_ref()
                .is_some_and(|picker| picker.query.is_empty())
            {
                state.resume_picker = None;
            } else if let Some(picker) = &mut state.resume_picker {
                picker.push_query_char('q');
            }
        }
        KeyCode::Char(ch) if resume_filter_accepts_char(key, ch) => {
            if let Some(picker) = &mut state.resume_picker {
                picker.push_query_char(ch);
            }
        }
        _ => {}
    }
}

pub(super) fn handle_resume_picker_mouse_for_state(
    state: &mut TuiState,
    mouse: MouseEvent,
    tools_area: Rect,
) -> bool {
    let Some(picker) = &mut state.resume_picker else {
        return false;
    };
    let handled = handle_resume_picker_mouse(picker, mouse, tools_area);
    if handled {
        state.last_event = resume_picker_selection_event(picker);
    }
    handled
}

fn handle_resume_picker_mouse(picker: &mut ResumePicker, mouse: MouseEvent, area: Rect) -> bool {
    match mouse.kind {
        MouseEventKind::ScrollUp
            if resume_picker_pointer_in_list(area, mouse.column, mouse.row) =>
        {
            picker.move_previous_by(RESUME_PICKER_MOUSE_SCROLL_STEP);
            true
        }
        MouseEventKind::ScrollDown
            if resume_picker_pointer_in_list(area, mouse.column, mouse.row) =>
        {
            picker.move_next_by(RESUME_PICKER_MOUSE_SCROLL_STEP);
            true
        }
        MouseEventKind::Down(MouseButton::Left) => {
            select_resume_picker_row(picker, area, mouse.column, mouse.row)
        }
        _ => false,
    }
}

fn resume_picker_pointer_in_list(area: Rect, column: u16, row: u16) -> bool {
    let (list_area, _) = resume_picker_layout(area);
    rect_contains(list_area, column, row)
}

fn select_resume_picker_row(picker: &mut ResumePicker, area: Rect, column: u16, row: u16) -> bool {
    let (list_area, _) = resume_picker_layout(area);
    if !rect_contains(list_area, column, row) || !rect_content_row_contains(list_area, row) {
        return false;
    }
    let visible = list_area.height.saturating_sub(2) as usize;
    if visible == 0 || picker.filtered_len() == 0 {
        return false;
    }
    let row_index = row.saturating_sub(list_area.y + 1) as usize;
    let selected = picker.visible_start(visible).saturating_add(row_index);
    if selected >= picker.filtered_len() {
        return false;
    }
    picker.selected = selected;
    true
}

fn resume_picker_selection_event(picker: &ResumePicker) -> String {
    picker
        .selected_session()
        .map(|session| {
            format!(
                "resume selected: {} {}",
                short_id(&session.id.to_string()),
                compact_ui_text(session.title.as_deref().unwrap_or("<untitled>"), 50)
            )
        })
        .unwrap_or_else(|| "resume selection unavailable".to_string())
}

fn confirm_resume_selection(state: &mut TuiState) {
    let Some(picker) = state.resume_picker.take() else {
        return;
    };
    let Some(session) = picker.selected_session() else {
        return;
    };
    let result = {
        let Some(runtime) = state.runtime.as_mut() else {
            return;
        };
        runtime
            .resume_session(&session.id.to_string())
            .map(|message| (message, chat_lines_from_runtime(runtime)))
    };
    apply_resume_result(state, result);
}

pub(super) fn render_resume_picker(frame: &mut Frame<'_>, area: Rect, picker: &ResumePicker) {
    let (list_area, preview_area) = resume_picker_layout(area);
    let visible = list_area.height.saturating_sub(2) as usize;
    let filtered = picker.filtered_indices();
    let start = picker.visible_start(visible);
    let mut items = filtered
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .filter_map(|(filtered_index, session_index)| {
            let session = picker.sessions.get(*session_index)?;
            let selected = if filtered_index == picker.selected {
                "*"
            } else {
                " "
            };
            let title = session.title.as_deref().unwrap_or("<untitled>");
            let model = session.model.as_deref().unwrap_or("<unset>");
            Some(ListItem::new(format!(
                "{selected} {}  {}  {}  {}",
                short_id(&session.id.to_string()),
                title,
                session.provider,
                model
            )))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        items.push(ListItem::new("no matching sessions"));
    }
    let title = if picker.query.trim().is_empty() {
        format!(
            "Resume Sessions ({}/{}, type to filter)",
            filtered.len(),
            picker.sessions.len()
        )
    } else {
        format!(
            "Resume Sessions filter=\"{}\" ({}/{})",
            compact_ui_text(&picker.query, 32),
            filtered.len(),
            picker.sessions.len()
        )
    };
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
        list_area,
    );
    frame.render_widget(
        Paragraph::new(format_resume_preview_text(picker, preview_area.height))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Resume Preview (Enter confirm, Esc cancel)"),
            ),
        preview_area,
    );
}

pub(super) fn resume_picker_layout(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);
    (chunks[0], chunks[1])
}

pub(super) fn format_resume_preview_text(picker: &ResumePicker, height: u16) -> String {
    let Some(metadata) = picker.selected_session() else {
        return if picker.query.trim().is_empty() {
            "no session selected".to_string()
        } else {
            format!(
                "no sessions match `{}`\nBackspace edits the filter; Esc cancels.",
                compact_ui_text(&picker.query, 80)
            )
        };
    };
    let store = SessionStore::new(&metadata.workspace);
    let session = match store.load(&metadata.id.to_string()) {
        Ok(session) => session,
        Err(error) => {
            return format!(
                "selected: {}\npreview unavailable: {}",
                short_id(&metadata.id.to_string()),
                compact_ui_text(&error.to_string(), 160)
            )
        }
    };

    let title = session.metadata.title.as_deref().unwrap_or("<untitled>");
    let model = session.metadata.model.as_deref().unwrap_or("<unset>");
    let mut lines = vec![
        format!("title: {}", compact_ui_text(title, 90)),
        format!("id: {}", session.id()),
        format!(
            "workspace: {}",
            compact_ui_text(&metadata.workspace.display().to_string(), 96)
        ),
        format!(
            "provider={} model={} state={:?}",
            session.metadata.provider, model, session.metadata.state
        ),
        format!(
            "created={} updated={}",
            session.metadata.created_at, session.metadata.updated_at
        ),
    ];

    match session.activity_summary() {
        Ok(activity) => lines.push(format!(
            "activity: messages={} tools={} tests={} approvals={} btw={} summary={}",
            activity.message_count,
            activity.tool_call_count,
            activity.test_run_count,
            activity.approval_request_count,
            activity.side_question_count,
            activity.has_summary
        )),
        Err(error) => lines.push(format!(
            "activity unavailable: {}",
            compact_ui_text(&error.to_string(), 120)
        )),
    }

    match session.load_summary() {
        Ok(Some(summary)) if !summary.trim().is_empty() => lines.push(format!(
            "summary: {}",
            compact_ui_text(
                &summary.split_whitespace().collect::<Vec<_>>().join(" "),
                140
            )
        )),
        Ok(_) => lines.push("summary: <none>".to_string()),
        Err(error) => lines.push(format!(
            "summary unavailable: {}",
            compact_ui_text(&error.to_string(), 120)
        )),
    }

    match session.load_recent_messages(3) {
        Ok(messages) if messages.is_empty() => lines.push("recent messages: <none>".to_string()),
        Ok(messages) => {
            lines.push("recent messages:".to_string());
            lines.extend(messages.into_iter().map(|message| {
                format!(
                    "  {}: {}",
                    message.role,
                    compact_ui_text(&message.content.replace('\n', " "), 120)
                )
            }));
        }
        Err(error) => lines.push(format!(
            "messages unavailable: {}",
            compact_ui_text(&error.to_string(), 120)
        )),
    }

    truncate_panel_lines(lines, height)
}
