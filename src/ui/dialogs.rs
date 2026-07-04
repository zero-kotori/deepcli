use super::monitor_changes::CHANGE_PATCH_SCROLL_STEP;
use super::{
    compact_ui_text, handle_prompt_input_key, scroll_change_patch_down, scroll_change_patch_up,
    session_monitor_for_state, short_id, workspace_for_state, ChatLine, MessageBox, TuiState,
};
use crate::agents::{AgentStore, SubagentStatus, SubagentTaskUpdate};
use crate::commands::update_project_config_value;
use crate::config::AppConfig;
use crate::session::SessionStore;
use crate::tools::ToolRegistry;
use anyhow::{anyhow, bail, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use serde_json::{Number, Value};
use std::path::PathBuf;
use uuid::Uuid;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(super) enum DialogKind {
    Permission,
    Diff,
    AgentEditor,
    Settings,
    Interview,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TuiDialog {
    #[cfg(test)]
    Notice(NoticeDialog),
    Diff(DiffDialog),
    AgentEditor(AgentEditorDialog),
    Settings(SettingsDialog),
    Interview(InterviewDialog),
}

impl TuiDialog {
    #[cfg(test)]
    pub(super) fn notice(
        kind: DialogKind,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self::Notice(NoticeDialog {
            kind,
            title: title.into(),
            body: body.into(),
        })
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NoticeDialog {
    pub(super) kind: DialogKind,
    title: String,
    body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DiffDialog {
    pub(super) scroll: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentEditorField {
    Task,
    ReadScope,
    WriteScope,
    AllowedTools,
    Context,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AgentEditorDialog {
    task_id: Uuid,
    selected: AgentEditorField,
    task: MessageBox,
    read_scope: MessageBox,
    write_scope: MessageBox,
    allowed_tools: MessageBox,
    context: MessageBox,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SettingsEntry {
    path: &'static str,
    value: MessageBox,
    value_kind: SettingsValueKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsValueKind {
    String,
    Number,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SettingsDialog {
    selected: usize,
    fields: Vec<SettingsEntry>,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InterviewDialog {
    pub(super) id: String,
    pub(super) question: String,
    pub(super) input: MessageBox,
}

pub(super) struct DialogView {
    pub(super) title: String,
    pub(super) body: String,
}

pub(super) fn open_interview_dialog(state: &mut TuiState, id: String, question: String) {
    state.dialog = Some(TuiDialog::Interview(InterviewDialog {
        id,
        question: question.clone(),
        input: MessageBox::new(),
    }));
    state.side_question_prompt = None;
    state.chat.push(ChatLine {
        role: "deepcli".to_string(),
        content: format!("请回答旁路问题：{question}"),
    });
    state.last_event = "btw answer prompt opened".to_string();
}

pub(super) fn open_diff_dialog(state: &mut TuiState) -> bool {
    if selected_diff_section(state).is_none() {
        state.last_event = "change patch unavailable".to_string();
        return false;
    }
    state.dialog = Some(TuiDialog::Diff(DiffDialog {
        scroll: state.change_patch_scroll,
    }));
    state.last_event = "diff dialog opened".to_string();
    true
}

pub(super) fn open_agent_editor_dialog(state: &mut TuiState, task_id: Uuid) -> Result<()> {
    let workspace = workspace_for_state(state)
        .ok_or_else(|| anyhow!("workspace unavailable for agent editor"))?
        .to_path_buf();
    let task = AgentStore::new(&workspace).load(task_id)?;
    if task.status != SubagentStatus::Queued {
        bail!("only queued sub-agent tasks can be edited");
    }
    let mut task_input = MessageBox::new();
    task_input.set_buffer(task.task);
    let mut read_scope = MessageBox::new();
    read_scope.set_buffer(join_paths(&task.read_scope));
    let mut write_scope = MessageBox::new();
    write_scope.set_buffer(join_paths(&task.write_scope));
    let mut allowed_tools = MessageBox::new();
    allowed_tools.set_buffer(task.allowed_tools.join("\n"));
    let mut context = MessageBox::new();
    context.set_buffer(task.context.unwrap_or_default());
    state.dialog = Some(TuiDialog::AgentEditor(AgentEditorDialog {
        task_id,
        selected: AgentEditorField::Task,
        task: task_input,
        read_scope,
        write_scope,
        allowed_tools,
        context,
        error: None,
    }));
    state.last_event = "agent editor opened".to_string();
    Ok(())
}

pub(super) fn open_latest_agent_editor_dialog(state: &mut TuiState) -> bool {
    let Some(workspace) = workspace_for_state(state).map(|path| path.to_path_buf()) else {
        state.last_event = "agent editor unavailable: no workspace".to_string();
        return false;
    };
    let task = match AgentStore::new(&workspace).list().and_then(|tasks| {
        tasks
            .into_iter()
            .rev()
            .find(|task| task.status == SubagentStatus::Queued)
            .ok_or_else(|| anyhow!("no queued sub-agent tasks"))
    }) {
        Ok(task) => task,
        Err(error) => {
            state.last_event = format!("agent editor unavailable: {error}");
            return false;
        }
    };
    match open_agent_editor_dialog(state, task.id) {
        Ok(()) => true,
        Err(error) => {
            state.last_event = format!("agent editor failed: {error}");
            false
        }
    }
}

pub(super) fn open_settings_dialog(state: &mut TuiState) -> Result<()> {
    let workspace = workspace_for_state(state)
        .ok_or_else(|| anyhow!("workspace unavailable for settings"))?
        .to_path_buf();
    let config = AppConfig::load_effective(&workspace, None)?;
    state.dialog = Some(TuiDialog::Settings(SettingsDialog {
        selected: 0,
        fields: settings_entries(&config),
        error: None,
    }));
    state.last_event = "settings dialog opened".to_string();
    Ok(())
}

pub(super) fn render_dialog(frame: &mut Frame<'_>, area: Rect, state: &TuiState) -> bool {
    let Some(view) = dialog_view_for_state(state, area.height) else {
        return false;
    };
    frame.render_widget(
        Paragraph::new(view.body)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(view.title)),
        area,
    );
    true
}

#[cfg(test)]
pub(super) fn dialog_body_for_state(state: &TuiState, height: u16) -> Option<String> {
    dialog_view_for_state(state, height).map(|view| view.body)
}

pub(super) fn dialog_view_for_state(state: &TuiState, height: u16) -> Option<DialogView> {
    if let Some(dialog) = &state.dialog {
        return Some(match dialog {
            #[cfg(test)]
            TuiDialog::Notice(dialog) => DialogView {
                title: dialog.title.clone(),
                body: dialog.body.clone(),
            },
            TuiDialog::Diff(_) => diff_dialog_view(state, height),
            TuiDialog::AgentEditor(dialog) => agent_editor_view(dialog),
            TuiDialog::Settings(dialog) => settings_view(dialog),
            TuiDialog::Interview(dialog) => interview_view(dialog),
        });
    }
    permission_dialog_view(state, height)
}

pub(super) fn handle_dialog_key(key: KeyEvent, state: &mut TuiState) -> bool {
    let Some(dialog) = state.dialog.as_ref() else {
        return false;
    };
    if key.code == KeyCode::Esc {
        state.dialog = None;
        state.last_event = "dialog closed".to_string();
        return true;
    }

    match dialog {
        #[cfg(test)]
        TuiDialog::Notice(_) => false,
        TuiDialog::Diff(_) => handle_diff_dialog_key(key, state),
        TuiDialog::AgentEditor(_) => handle_agent_editor_key(key, state),
        TuiDialog::Settings(_) => handle_settings_key(key, state),
        TuiDialog::Interview(_) => handle_interview_key(key, state),
    }
}

#[cfg(test)]
pub(super) fn replace_dialog_field(state: &mut TuiState, field: &str, value: &str) -> Result<()> {
    match state.dialog.as_mut() {
        Some(TuiDialog::AgentEditor(dialog)) => {
            agent_editor_input_mut(dialog, field)?.set_buffer(value.to_string());
            Ok(())
        }
        Some(TuiDialog::Settings(dialog)) => {
            let entry = dialog
                .fields
                .iter_mut()
                .find(|entry| entry.path == field)
                .ok_or_else(|| anyhow!("settings field `{field}` not found"))?;
            entry.value.set_buffer(value.to_string());
            Ok(())
        }
        _ => bail!("dialog field `{field}` is not editable"),
    }
}

fn permission_dialog_view(state: &TuiState, height: u16) -> Option<DialogView> {
    if state.credential_prompt.is_some()
        || state.side_question_prompt.is_some()
        || state.resume_picker.is_some()
    {
        return None;
    }
    let monitor = session_monitor_for_state(state)?;
    let approval_count = monitor.pending_approvals.len();
    let question_count = monitor.open_questions.len();
    let total = approval_count + question_count;
    if total == 0 {
        return None;
    }
    let selected = state.selected_approval.min(total - 1);
    let content_rows = height.saturating_sub(2) as usize;
    let list_rows = content_rows.saturating_sub(1);
    let mut lines = vec![format!(
        "blockers {}/{} approvals={} interviews={}",
        selected + 1,
        total,
        approval_count,
        question_count
    )];
    lines.extend(
        visible_indices(total, selected, list_rows)
            .into_iter()
            .map(|index| format_blocker_line(&monitor, index, index == selected)),
    );
    Some(DialogView {
        title: "Permission".to_string(),
        body: lines.join("\n"),
    })
}

fn diff_dialog_view(state: &TuiState, height: u16) -> DialogView {
    let visible = height.saturating_sub(4).max(1) as usize;
    let Some(section) = selected_diff_section(state) else {
        return DialogView {
            title: "Diff".to_string(),
            body: "change patch unavailable".to_string(),
        };
    };
    let scroll = diff_dialog_scroll(state).min(section.lines.len().saturating_sub(1));
    let end = (scroll + visible).min(section.lines.len());
    let mut lines = vec![format!(
        "{} {}{}",
        section.label,
        section.path,
        if section.truncated {
            " (truncated)"
        } else {
            ""
        }
    )];
    if scroll > 0 {
        lines.push(format!("[above: {scroll} line(s)]"));
    }
    lines.extend(section.lines[scroll..end].iter().cloned());
    if end < section.lines.len() {
        lines.push(format!("[below: {} line(s)]", section.lines.len() - end));
    }
    DialogView {
        title: "Diff".to_string(),
        body: lines.join("\n"),
    }
}

fn agent_editor_view(dialog: &AgentEditorDialog) -> DialogView {
    let mut lines = vec![format!("agent {}", short_id(&dialog.task_id.to_string()))];
    if let Some(error) = &dialog.error {
        lines.push(format!("error: {}", compact_ui_text(error, 90)));
    }
    for (field, label, value) in [
        (AgentEditorField::Task, "task", dialog.task.buffer()),
        (
            AgentEditorField::ReadScope,
            "read_scope",
            dialog.read_scope.buffer(),
        ),
        (
            AgentEditorField::WriteScope,
            "write_scope",
            dialog.write_scope.buffer(),
        ),
        (
            AgentEditorField::AllowedTools,
            "allowed_tools",
            dialog.allowed_tools.buffer(),
        ),
        (
            AgentEditorField::Context,
            "context",
            dialog.context.buffer(),
        ),
    ] {
        let marker = if dialog.selected == field { ">" } else { " " };
        lines.push(format!(
            "{marker} {label}: {}",
            compact_ui_text(&value.replace('\n', "\\n"), 96)
        ));
    }
    DialogView {
        title: "Agent Editor".to_string(),
        body: lines.join("\n"),
    }
}

fn settings_view(dialog: &SettingsDialog) -> DialogView {
    let mut lines = Vec::new();
    if let Some(error) = &dialog.error {
        lines.push(format!("error: {}", compact_ui_text(error, 100)));
    }
    for (index, entry) in dialog.fields.iter().enumerate() {
        let marker = if index == dialog.selected { ">" } else { " " };
        lines.push(format!(
            "{marker} {} = {}",
            entry.path,
            compact_ui_text(entry.value.buffer(), 72)
        ));
    }
    DialogView {
        title: "Settings".to_string(),
        body: lines.join("\n"),
    }
}

fn interview_view(dialog: &InterviewDialog) -> DialogView {
    DialogView {
        title: "Interview".to_string(),
        body: format!(
            "{}\nanswer: {}",
            compact_ui_text(&dialog.question, 100),
            dialog.input.buffer()
        ),
    }
}

fn handle_diff_dialog_key(key: KeyEvent, state: &mut TuiState) -> bool {
    match key.code {
        KeyCode::Char(']') if key.modifiers.is_empty() => {
            if let Some(count) = diff_section_count(state) {
                state.selected_change = (state.selected_change + 1).min(count.saturating_sub(1));
                state.change_patch_scroll = 0;
                if let Some(TuiDialog::Diff(dialog)) = state.dialog.as_mut() {
                    dialog.scroll = 0;
                }
                state.last_event = "diff selection changed".to_string();
            }
            true
        }
        KeyCode::Char('[') if key.modifiers.is_empty() => {
            state.selected_change = state.selected_change.saturating_sub(1);
            state.change_patch_scroll = 0;
            if let Some(TuiDialog::Diff(dialog)) = state.dialog.as_mut() {
                dialog.scroll = 0;
            }
            state.last_event = "diff selection changed".to_string();
            true
        }
        KeyCode::PageDown => {
            scroll_change_patch_down(state, CHANGE_PATCH_SCROLL_STEP);
            if let Some(TuiDialog::Diff(dialog)) = state.dialog.as_mut() {
                dialog.scroll = state.change_patch_scroll;
            }
            true
        }
        KeyCode::PageUp => {
            scroll_change_patch_up(state, CHANGE_PATCH_SCROLL_STEP);
            if let Some(TuiDialog::Diff(dialog)) = state.dialog.as_mut() {
                dialog.scroll = state.change_patch_scroll;
            }
            true
        }
        _ => false,
    }
}

fn handle_agent_editor_key(key: KeyEvent, state: &mut TuiState) -> bool {
    if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
        save_agent_editor(state);
        return true;
    }
    let Some(TuiDialog::AgentEditor(dialog)) = state.dialog.as_mut() else {
        return false;
    };
    if key.code == KeyCode::Tab {
        dialog.selected = next_agent_editor_field(dialog.selected);
        return true;
    }
    let input = agent_editor_input_for_selected_mut(dialog);
    let action = if key.code == KeyCode::Enter {
        input.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT))
    } else {
        input.handle_key(key)
    };
    !matches!(action, super::MessageBoxAction::Noop)
}

fn handle_settings_key(key: KeyEvent, state: &mut TuiState) -> bool {
    if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
        save_settings(state);
        return true;
    }
    let Some(TuiDialog::Settings(dialog)) = state.dialog.as_mut() else {
        return false;
    };
    if key.code == KeyCode::Tab {
        if !dialog.fields.is_empty() {
            dialog.selected = (dialog.selected + 1) % dialog.fields.len();
        }
        return true;
    }
    let Some(entry) = dialog.fields.get_mut(dialog.selected) else {
        return false;
    };
    let action = entry.value.handle_key(key);
    !matches!(action, super::MessageBoxAction::Noop)
}

fn handle_interview_key(key: KeyEvent, state: &mut TuiState) -> bool {
    match key.code {
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
            handle_prompt_input_key(interview_input_mut(state), key);
            true
        }
        KeyCode::Enter => {
            confirm_interview_dialog(state);
            true
        }
        _ => {
            handle_prompt_input_key(interview_input_mut(state), key);
            true
        }
    }
}

fn confirm_interview_dialog(state: &mut TuiState) {
    let Some(TuiDialog::Interview(dialog)) = state.dialog.as_ref() else {
        return;
    };
    let answer = dialog.input.buffer().trim().to_string();
    if answer.is_empty() {
        state.chat.push(ChatLine {
            role: "error".to_string(),
            content: "btw answer 不能为空。".to_string(),
        });
        state.last_event = "btw answer rejected".to_string();
        return;
    }
    let id = dialog.id.clone();
    match answer_side_question_for_state(state, &id, &answer) {
        Ok(message) => {
            state.dialog = None;
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: message,
            });
            state.last_event = "btw answer saved".to_string();
            clamp_selected_blocker_to_monitor(state);
        }
        Err(error) => {
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            });
            state.last_event = "btw answer failed".to_string();
        }
    }
}

fn save_agent_editor(state: &mut TuiState) {
    let result = save_agent_editor_result(state);
    match result {
        Ok(()) => {
            state.dialog = None;
            state.last_event = "agent task saved".to_string();
        }
        Err(error) => {
            if let Some(TuiDialog::AgentEditor(dialog)) = state.dialog.as_mut() {
                dialog.error = Some(error.to_string());
            }
            state.last_event = format!("agent task save failed: {error}");
        }
    }
}

fn save_agent_editor_result(state: &TuiState) -> Result<()> {
    let workspace = workspace_for_state(state)
        .ok_or_else(|| anyhow!("workspace unavailable for agent editor"))?
        .to_path_buf();
    let Some(TuiDialog::AgentEditor(dialog)) = state.dialog.as_ref() else {
        bail!("agent editor is not open");
    };
    let allowed_tools = parse_lines(dialog.allowed_tools.buffer());
    validate_allowed_tools(&allowed_tools)?;
    AgentStore::new(&workspace).update_queued_subagent_task(
        dialog.task_id,
        SubagentTaskUpdate {
            task: dialog.task.buffer().trim().to_string(),
            read_scope: parse_paths(dialog.read_scope.buffer()),
            write_scope: parse_paths(dialog.write_scope.buffer()),
            allowed_tools,
            context: non_empty_string(dialog.context.buffer()),
        },
    )?;
    Ok(())
}

fn save_settings(state: &mut TuiState) {
    let result = save_settings_result(state);
    match result {
        Ok(()) => {
            state.dialog = None;
            state.last_event = "settings saved".to_string();
        }
        Err(error) => {
            if let Some(TuiDialog::Settings(dialog)) = state.dialog.as_mut() {
                dialog.error = Some(error.to_string());
            }
            state.last_event = format!("settings save failed: {error}");
        }
    }
}

fn save_settings_result(state: &TuiState) -> Result<()> {
    let workspace = workspace_for_state(state)
        .ok_or_else(|| anyhow!("workspace unavailable for settings"))?
        .to_path_buf();
    let Some(TuiDialog::Settings(dialog)) = state.dialog.as_ref() else {
        bail!("settings dialog is not open");
    };
    let mut config = AppConfig::load_effective(&workspace, None)?;
    for entry in &dialog.fields {
        let value = parse_settings_value(entry)?;
        update_project_config_value(&workspace, &config, entry.path, value)?;
        config = AppConfig::load_effective(&workspace, None)?;
    }
    Ok(())
}

fn parse_settings_value(entry: &SettingsEntry) -> Result<Value> {
    match entry.value_kind {
        SettingsValueKind::String => Ok(Value::String(entry.value.buffer().trim().to_string())),
        SettingsValueKind::Number => {
            let value = entry
                .value
                .buffer()
                .trim()
                .parse::<u64>()
                .with_context(|| format!("{} must be a number", entry.path))?;
            Ok(Value::Number(Number::from(value)))
        }
    }
}

trait WithContext<T> {
    fn with_context<F: FnOnce() -> String>(self, f: F) -> Result<T>;
}

impl<T, E> WithContext<T> for std::result::Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn with_context<F: FnOnce() -> String>(self, f: F) -> Result<T> {
        self.map_err(|error| anyhow!("{}: {}", f(), error))
    }
}

fn settings_entries(config: &AppConfig) -> Vec<SettingsEntry> {
    [
        (
            "defaultProvider",
            config.default_provider.clone(),
            SettingsValueKind::String,
        ),
        (
            "agent.providerTurnTimeoutSeconds",
            config.agent.provider_turn_timeout_seconds.to_string(),
            SettingsValueKind::Number,
        ),
        (
            "agent.maxToolIterations",
            config.agent.max_tool_iterations.to_string(),
            SettingsValueKind::Number,
        ),
        (
            "agent.maxContextTokens",
            config.agent.max_context_tokens.to_string(),
            SettingsValueKind::Number,
        ),
        (
            "agent.reservedOutputTokens",
            config.agent.reserved_output_tokens.to_string(),
            SettingsValueKind::Number,
        ),
        (
            "agent.maxSubagentDepth",
            config.agent.max_subagent_depth.to_string(),
            SettingsValueKind::Number,
        ),
        (
            "permissions.defaultMode",
            config.permissions.default_mode.clone(),
            SettingsValueKind::String,
        ),
    ]
    .into_iter()
    .map(|(path, value, value_kind)| {
        let mut input = MessageBox::new();
        input.set_buffer(value);
        SettingsEntry {
            path,
            value: input,
            value_kind,
        }
    })
    .collect()
}

fn answer_side_question_for_state(state: &mut TuiState, id: &str, answer: &str) -> Result<String> {
    if let Some(runtime) = state.runtime.as_mut() {
        return runtime.answer_current_side_question(id, answer);
    }
    let active = state
        .active_session
        .as_ref()
        .ok_or_else(|| anyhow!("当前运行会话不可用"))?;
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let item = session.answer_side_question(id, answer)?;
    Ok(format!("answered btw question {}", item.id))
}

pub(super) fn clamp_selected_blocker_to_monitor(state: &mut TuiState) {
    let remaining = session_monitor_for_state(state)
        .map(|monitor| monitor.pending_approvals.len() + monitor.open_questions.len())
        .unwrap_or_default();
    if remaining == 0 {
        state.selected_approval = 0;
    } else {
        state.selected_approval = state.selected_approval.min(remaining - 1);
    }
}

fn format_blocker_line(
    monitor: &crate::runtime::SessionMonitor,
    index: usize,
    selected: bool,
) -> String {
    let marker = if selected { ">" } else { " " };
    if let Some(approval) = monitor.pending_approvals.get(index) {
        return format!(
            "{marker} approve/deny {} {} risk={} {}",
            short_id(&approval.id),
            compact_ui_text(&approval.tool, 24),
            approval.risk,
            compact_ui_text(&approval.reason, 64)
        );
    }
    let question_index = index.saturating_sub(monitor.pending_approvals.len());
    let Some(question) = monitor.open_questions.get(question_index) else {
        return format!("{marker} blocker unavailable");
    };
    format!(
        "{marker} interview {} {}",
        short_id(&question.id),
        compact_ui_text(&question.question, 82)
    )
}

fn visible_indices(total: usize, selected: usize, slots: usize) -> Vec<usize> {
    if total == 0 || slots == 0 {
        return Vec::new();
    }
    if total <= slots {
        return (0..total).collect();
    }
    let selected = selected.min(total - 1);
    let mut start = selected.saturating_sub(slots.saturating_sub(1));
    if start + slots > total {
        start = total.saturating_sub(slots);
    }
    (start..start + slots).collect()
}

fn selected_diff_section(
    state: &TuiState,
) -> Option<&super::monitor_changes::WorkspaceDiffSection> {
    let snapshot = state.workspace_changes.as_ref()?;
    if snapshot.diff_sections.is_empty() {
        return None;
    }
    snapshot
        .diff_sections
        .get(state.selected_change.min(snapshot.diff_sections.len() - 1))
}

fn diff_section_count(state: &TuiState) -> Option<usize> {
    let count = state
        .workspace_changes
        .as_ref()
        .filter(|snapshot| snapshot.available)
        .map(|snapshot| snapshot.diff_sections.len())
        .unwrap_or_default();
    (count > 0).then_some(count)
}

fn diff_dialog_scroll(state: &TuiState) -> usize {
    match &state.dialog {
        Some(TuiDialog::Diff(dialog)) => dialog.scroll,
        _ => state.change_patch_scroll,
    }
}

#[cfg(test)]
fn agent_editor_input_mut<'a>(
    dialog: &'a mut AgentEditorDialog,
    field: &str,
) -> Result<&'a mut MessageBox> {
    match field {
        "task" => Ok(&mut dialog.task),
        "read_scope" => Ok(&mut dialog.read_scope),
        "write_scope" => Ok(&mut dialog.write_scope),
        "allowed_tools" => Ok(&mut dialog.allowed_tools),
        "context" => Ok(&mut dialog.context),
        _ => bail!("agent editor field `{field}` not found"),
    }
}

fn agent_editor_input_for_selected_mut(dialog: &mut AgentEditorDialog) -> &mut MessageBox {
    match dialog.selected {
        AgentEditorField::Task => &mut dialog.task,
        AgentEditorField::ReadScope => &mut dialog.read_scope,
        AgentEditorField::WriteScope => &mut dialog.write_scope,
        AgentEditorField::AllowedTools => &mut dialog.allowed_tools,
        AgentEditorField::Context => &mut dialog.context,
    }
}

fn next_agent_editor_field(field: AgentEditorField) -> AgentEditorField {
    match field {
        AgentEditorField::Task => AgentEditorField::ReadScope,
        AgentEditorField::ReadScope => AgentEditorField::WriteScope,
        AgentEditorField::WriteScope => AgentEditorField::AllowedTools,
        AgentEditorField::AllowedTools => AgentEditorField::Context,
        AgentEditorField::Context => AgentEditorField::Task,
    }
}

fn interview_input_mut(state: &mut TuiState) -> Option<&mut MessageBox> {
    match state.dialog.as_mut() {
        Some(TuiDialog::Interview(dialog)) => Some(&mut dialog.input),
        _ => None,
    }
}

fn parse_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_paths(value: &str) -> Vec<PathBuf> {
    parse_lines(value).into_iter().map(PathBuf::from).collect()
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn validate_allowed_tools(tools: &[String]) -> Result<()> {
    let registry = ToolRegistry::mvp();
    for tool in tools {
        if registry.declaration(tool).is_none() {
            bail!("sub-agent allowed_tools contains unknown tool `{tool}`");
        }
    }
    Ok(())
}
