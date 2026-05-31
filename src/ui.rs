use crate::agents::AgentStore;
use crate::commands::{
    handle_approval, handle_completion_local, handle_logs, handle_selftest_local, handle_session,
    handle_trace, handle_usage, list_resumable_sessions, CommandHelpSummary, CommandRouter,
    SlashCommand,
};
use crate::config::{absolutize_workspace_path, AppConfig};
use crate::permissions::PermissionEngine;
use crate::prompts::PromptStore;
use crate::runtime::{
    session_environment_observations_from_tool_calls, session_usage_observation_from_audit_events,
    AgentRuntime, RuntimeOptions, RuntimeProgress, SessionMonitor, SessionObservation,
    SessionObservationApproval, SessionObservationEnvironment, SessionObservationEvent,
    SessionObservationQuestion, SessionObservationTest, SessionObservationUsage,
};
use crate::session::{
    ApprovalStatus, Plan, PlanStepStatus, Session, SessionDiffRecord, SessionMessage,
    SessionMetadata, SessionState, SessionStore, SideQuestion, SideQuestionStatus, ToolCallStatus,
};
use crate::skills::SkillStore;
use crate::tools::ToolExecutor;
use anyhow::{anyhow, Result};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::env;
use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use unicode_width::UnicodeWidthChar;

const TUI_HISTORY_MESSAGE_CHARS: usize = 4_000;
const COMMAND_PALETTE_MATCH_LIMIT: usize = 16;
const TRANSCRIPT_SCROLL_STEP: usize = 6;
const TRANSCRIPT_MOUSE_SCROLL_STEP: usize = 3;
const RESULT_SCROLL_STEP: usize = 4;
const RESULT_MOUSE_SCROLL_STEP: usize = 3;
const WORKTREE_CHANGES_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const WORKTREE_DIFF_PREVIEW_LINES: usize = 18;
const WORKTREE_DIFF_SECTION_LINES: usize = 180;
const CHANGE_PATCH_WINDOW_LINES: usize = 12;
const CHANGE_PATCH_SCROLL_STEP: usize = 8;
const CHANGE_PATCH_MOUSE_SCROLL_STEP: usize = 4;
const RESUME_PICKER_MOUSE_SCROLL_STEP: usize = 3;
const TOOL_KEY_SCROLL_STEP: usize = 5;
const TOOL_MOUSE_SCROLL_STEP: usize = 3;
const TOOL_DETAIL_PREVIEW_CHARS: usize = 1_200;
const TOOL_DETAIL_PREVIEW_LINES: usize = 8;
const TOOL_DETAIL_PREVIEW_LINE_CHARS: usize = 180;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageBoxAction {
    Inserted,
    Submitted(String),
    Noop,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageBox {
    buffer: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiSnapshot {
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub state: String,
    pub plan_steps: Vec<String>,
    pub token_usage: String,
    pub last_event: String,
}

impl MessageBox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_buffer(&mut self, value: String) {
        self.buffer = value;
        self.cursor = self.buffer.len();
        self.history_index = None;
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.history_index = None;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> MessageBoxAction {
        match key.code {
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = 0;
                MessageBoxAction::Inserted
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.buffer.len();
                MessageBoxAction::Inserted
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.buffer.drain(..self.cursor);
                self.cursor = 0;
                self.history_index = None;
                MessageBoxAction::Inserted
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.buffer.drain(self.cursor..);
                self.history_index = None;
                MessageBoxAction::Inserted
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.insert_char('\n');
                MessageBoxAction::Inserted
            }
            KeyCode::Enter => {
                let submitted = self.buffer.trim_end().to_string();
                self.clear();
                if !submitted.is_empty() {
                    self.history.push(submitted.clone());
                }
                MessageBoxAction::Submitted(submitted)
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.insert_char(ch);
                MessageBoxAction::Inserted
            }
            KeyCode::Backspace => {
                self.delete_before_cursor();
                MessageBoxAction::Inserted
            }
            KeyCode::Delete => {
                self.delete_at_cursor();
                MessageBoxAction::Inserted
            }
            KeyCode::Left => {
                self.cursor = self.previous_char_boundary();
                MessageBoxAction::Inserted
            }
            KeyCode::Right => {
                self.cursor = self.next_char_boundary();
                MessageBoxAction::Inserted
            }
            KeyCode::Home => {
                self.cursor = 0;
                MessageBoxAction::Inserted
            }
            KeyCode::End => {
                self.cursor = self.buffer.len();
                MessageBoxAction::Inserted
            }
            KeyCode::Up => {
                if self.history.is_empty() {
                    return MessageBoxAction::Noop;
                }
                let next = self
                    .history_index
                    .map(|index| index.saturating_sub(1))
                    .unwrap_or_else(|| self.history.len() - 1);
                self.history_index = Some(next);
                self.buffer = self.history[next].clone();
                self.cursor = self.buffer.len();
                MessageBoxAction::Inserted
            }
            KeyCode::Down => {
                let Some(index) = self.history_index else {
                    return MessageBoxAction::Noop;
                };
                if index + 1 >= self.history.len() {
                    self.clear();
                } else {
                    self.history_index = Some(index + 1);
                    self.buffer = self.history[index + 1].clone();
                    self.cursor = self.buffer.len();
                }
                MessageBoxAction::Inserted
            }
            _ => MessageBoxAction::Noop,
        }
    }

    fn insert_char(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.history_index = None;
    }

    fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.history_index = None;
    }

    fn delete_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = self.previous_char_boundary();
        self.buffer.drain(previous..self.cursor);
        self.cursor = previous;
        self.history_index = None;
    }

    fn delete_at_cursor(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = self.next_char_boundary();
        self.buffer.drain(self.cursor..next);
        self.history_index = None;
    }

    fn previous_char_boundary(&self) -> usize {
        self.buffer[..self.cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    fn next_char_boundary(&self) -> usize {
        self.buffer[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(offset, _)| self.cursor + offset)
            .unwrap_or_else(|| self.buffer.len())
    }
}

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
        if matches!(input.trim(), "/quit" | "/exit") {
            break;
        }
        let output = runtime.handle_input(input).await?;
        println!("{output}");
    }
    Ok(())
}

#[derive(Debug)]
struct ToolLogItem {
    title: String,
    detail: String,
    expanded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolTabLine {
    text: String,
    tool_index: Option<usize>,
}

#[derive(Debug)]
struct ChatLine {
    role: String,
    content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonitorTab {
    Overview,
    Result,
    Changes,
    Usage,
    Health,
    Library,
    Deliver,
    Tools,
    Tests,
    Environment,
    Approvals,
    Trace,
}

impl MonitorTab {
    fn next(self) -> Self {
        match self {
            Self::Overview => Self::Result,
            Self::Result => Self::Changes,
            Self::Changes => Self::Usage,
            Self::Usage => Self::Health,
            Self::Health => Self::Library,
            Self::Library => Self::Deliver,
            Self::Deliver => Self::Tools,
            Self::Tools => Self::Tests,
            Self::Tests => Self::Environment,
            Self::Environment => Self::Approvals,
            Self::Approvals => Self::Trace,
            Self::Trace => Self::Overview,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Overview => Self::Trace,
            Self::Result => Self::Overview,
            Self::Changes => Self::Result,
            Self::Usage => Self::Changes,
            Self::Health => Self::Usage,
            Self::Library => Self::Health,
            Self::Deliver => Self::Library,
            Self::Tools => Self::Deliver,
            Self::Tests => Self::Tools,
            Self::Environment => Self::Tests,
            Self::Approvals => Self::Environment,
            Self::Trace => Self::Approvals,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Result => "Result",
            Self::Changes => "Changes",
            Self::Usage => "Usage",
            Self::Health => "Health",
            Self::Library => "Library",
            Self::Deliver => "Deliver",
            Self::Tools => "Tools",
            Self::Tests => "Tests",
            Self::Environment => "Environment",
            Self::Approvals => "Approvals",
            Self::Trace => "Trace",
        }
    }

    fn all() -> &'static [Self] {
        &[
            Self::Overview,
            Self::Result,
            Self::Changes,
            Self::Usage,
            Self::Health,
            Self::Library,
            Self::Deliver,
            Self::Tools,
            Self::Tests,
            Self::Environment,
            Self::Approvals,
            Self::Trace,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MonitorQuickAction {
    command: String,
    edit_before_run: bool,
}

impl MonitorQuickAction {
    fn run(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            edit_before_run: false,
        }
    }

    fn edit(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            edit_before_run: true,
        }
    }
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
    worker: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceChangesSnapshot {
    available: bool,
    detail: Option<String>,
    changed: usize,
    staged: usize,
    unstaged: usize,
    untracked: usize,
    paths: Vec<String>,
    diff_preview: Vec<String>,
    diff_preview_truncated: bool,
    diff_sections: Vec<WorkspaceDiffSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceDiffSection {
    label: String,
    path: String,
    lines: Vec<String>,
    truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveSessionRef {
    workspace: PathBuf,
    session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HeaderStatus {
    session: String,
    title: String,
    provider: String,
    model: String,
    state: String,
}

fn active_session_ref(runtime: &AgentRuntime) -> ActiveSessionRef {
    ActiveSessionRef {
        workspace: runtime.workspace().to_path_buf(),
        session_id: runtime.session_id(),
    }
}

fn sync_active_session_ref(state: &mut TuiState) {
    if let Some(runtime) = &state.runtime {
        state.active_session = Some(active_session_ref(runtime));
    }
}

fn session_monitor_for_state(state: &TuiState) -> Option<SessionMonitor> {
    if let Some(monitor) = state
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.session_monitor().ok())
    {
        return Some(monitor);
    }
    state
        .active_session
        .as_ref()
        .and_then(|active| load_active_session_monitor(active).ok())
}

fn header_status_for_state(state: &TuiState) -> HeaderStatus {
    if let Some(runtime) = state.runtime.as_ref() {
        return HeaderStatus {
            session: runtime.session_id(),
            title: runtime
                .session_title()
                .map(str::to_string)
                .unwrap_or_else(|| "<untitled>".to_string()),
            provider: runtime.provider_name().to_string(),
            model: runtime
                .model_name()
                .map(str::to_string)
                .unwrap_or_else(|| "<unset>".to_string()),
            state: runtime.state_label(),
        };
    }
    state
        .active_session
        .as_ref()
        .and_then(|active| load_active_session_header(active).ok())
        .unwrap_or_else(|| HeaderStatus {
            session: "<running>".to_string(),
            title: "<untitled>".to_string(),
            provider: "<running>".to_string(),
            model: "<unset>".to_string(),
            state: "Running".to_string(),
        })
}

fn load_active_session_header(active: &ActiveSessionRef) -> Result<HeaderStatus> {
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    Ok(HeaderStatus {
        session: session.id().to_string(),
        title: session
            .metadata
            .title
            .clone()
            .unwrap_or_else(|| "<untitled>".to_string()),
        provider: session.metadata.provider,
        model: session
            .metadata
            .model
            .clone()
            .unwrap_or_else(|| "<unset>".to_string()),
        state: format!("{:?}", session.metadata.state),
    })
}

fn load_active_session_monitor(active: &ActiveSessionRef) -> Result<SessionMonitor> {
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    session_monitor_from_session(&session)
}

fn session_monitor_from_session(session: &Session) -> Result<SessionMonitor> {
    let plan = session.load_plan()?;
    let (plan_total, plan_completed, plan_in_progress, plan_failed, current_step) = plan
        .as_ref()
        .map(summarize_plan_for_tui)
        .unwrap_or((0, 0, 0, 0, None));
    let latest_test = session
        .load_recent_test_runs(1)?
        .into_iter()
        .last()
        .map(|test| SessionObservationTest {
            command: test.command,
            passed: test.passed,
            exit_code: test.exit_code,
        });
    let pending_approvals = session
        .load_approval_requests()?
        .iter()
        .filter(|request| request.status == ApprovalStatus::Pending)
        .count();
    let open_questions = session
        .load_side_questions()?
        .iter()
        .filter(|question| question.status == SideQuestionStatus::Open)
        .count();
    let tools = session.load_tool_calls()?;
    let failed_tools = tools
        .iter()
        .filter(|tool| matches!(tool.status, ToolCallStatus::Failed | ToolCallStatus::Denied))
        .count();
    let observation = SessionObservation {
        state: format!("{:?}", session.metadata.state),
        plan_total,
        plan_completed,
        plan_in_progress,
        plan_failed,
        current_step,
        latest_test,
        pending_approvals,
        open_questions,
        tool_calls: tools.len(),
        failed_tools,
    };
    let recent_tests = session
        .load_recent_test_runs(6)?
        .into_iter()
        .map(|test| SessionObservationTest {
            command: test.command,
            passed: test.passed,
            exit_code: test.exit_code,
        })
        .collect();
    let recent_environment =
        session_environment_observations_from_tool_calls(&session.load_tool_calls()?, 6);
    let pending_approvals = session
        .load_approval_requests()?
        .into_iter()
        .filter(|request| request.status == ApprovalStatus::Pending)
        .map(|request| SessionObservationApproval {
            id: request.id.to_string(),
            tool: request.tool,
            risk: format!("{:?}", request.decision.risk),
            reason: request.decision.reason,
        })
        .collect();
    let open_questions = session
        .load_side_questions()?
        .into_iter()
        .filter(|question| question.status == SideQuestionStatus::Open)
        .map(|question| SessionObservationQuestion {
            id: question.id.to_string(),
            question: question.question,
        })
        .collect();
    let events = session.load_audit_events()?;
    let usage = session_usage_observation_from_audit_events(&events);
    let skip = events.len().saturating_sub(8);
    let recent_events = events
        .into_iter()
        .skip(skip)
        .map(|event| SessionObservationEvent {
            event_type: event.event_type,
            created_at: event.created_at.format("%H:%M:%S").to_string(),
        })
        .collect();

    Ok(SessionMonitor {
        observation,
        usage,
        recent_tests,
        recent_environment,
        pending_approvals,
        open_questions,
        recent_events,
    })
}

fn summarize_plan_for_tui(plan: &Plan) -> (usize, usize, usize, usize, Option<String>) {
    let mut completed = 0;
    let mut in_progress = 0;
    let mut failed = 0;
    let mut current = None;
    for step in &plan.steps {
        match step.status {
            PlanStepStatus::Completed => completed += 1,
            PlanStepStatus::InProgress => {
                in_progress += 1;
                if current.is_none() {
                    current = Some(step.description.clone());
                }
            }
            PlanStepStatus::Failed => {
                failed += 1;
                if current.is_none() {
                    current = Some(step.description.clone());
                }
            }
            PlanStepStatus::Pending => {}
        }
    }
    (plan.steps.len(), completed, in_progress, failed, current)
}

struct WorkerDone {
    runtime: AgentRuntime,
    result: std::result::Result<String, String>,
}

struct ResumePicker {
    sessions: Vec<SessionMetadata>,
    selected: usize,
    query: String,
}

impl ResumePicker {
    fn new(sessions: Vec<SessionMetadata>) -> Self {
        Self {
            sessions,
            selected: 0,
            query: String::new(),
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        self.sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| {
                session_matches_resume_query(session, &self.query).then_some(index)
            })
            .collect()
    }

    fn filtered_len(&self) -> usize {
        self.sessions
            .iter()
            .filter(|session| session_matches_resume_query(session, &self.query))
            .count()
    }

    fn selected_session(&self) -> Option<&SessionMetadata> {
        let indices = self.filtered_indices();
        indices
            .get(self.selected)
            .and_then(|index| self.sessions.get(*index))
    }

    fn clamp_selected(&mut self) {
        let len = self.filtered_len();
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len - 1);
        }
    }

    fn move_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_next(&mut self) {
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

    fn move_home(&mut self) {
        self.selected = 0;
    }

    fn move_end(&mut self) {
        self.selected = self.filtered_len().saturating_sub(1);
    }

    fn push_query_char(&mut self, ch: char) {
        self.query.push(ch);
        self.selected = 0;
    }

    fn push_query_str(&mut self, text: &str) {
        self.query.push_str(text);
        self.selected = 0;
    }

    fn pop_query_char(&mut self) {
        self.query.pop();
        self.clamp_selected();
    }

    fn visible_start(&self, visible: usize) -> usize {
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

    let mut picker = ResumePicker::new(sessions);
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_resume_picker_loop(&mut terminal, &mut picker);
    let raw_result = disable_raw_mode();
    let screen_result = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let cursor_result = terminal.show_cursor();
    raw_result?;
    screen_result?;
    cursor_result?;
    result
}

fn run_resume_picker_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    picker: &mut ResumePicker,
) -> Result<ResumeSelection> {
    loop {
        terminal.draw(|frame| render_resume_picker(frame, frame.area(), picker))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) => match key.code {
                KeyCode::Up | KeyCode::Left => {
                    picker.move_previous();
                }
                KeyCode::Down | KeyCode::Right => {
                    picker.move_next();
                }
                KeyCode::Home => picker.move_home(),
                KeyCode::End => picker.move_end(),
                KeyCode::Backspace => picker.pop_query_char(),
                KeyCode::Enter => {
                    let Some(session) = picker.selected_session() else {
                        return Ok(ResumeSelection::Cancelled);
                    };
                    return Ok(ResumeSelection::Selected(session.id.to_string()));
                }
                KeyCode::Esc => return Ok(ResumeSelection::Cancelled),
                KeyCode::Char('q') if picker.query.is_empty() => {
                    return Ok(ResumeSelection::Cancelled);
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(ResumeSelection::Cancelled);
                }
                KeyCode::Char(ch) if resume_filter_accepts_char(key, ch) => {
                    picker.push_query_char(ch);
                }
                _ => {}
            },
            Event::Mouse(mouse) => {
                let size = terminal.size()?;
                handle_resume_picker_mouse(
                    picker,
                    mouse,
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
    }
}

fn resume_filter_accepts_char(key: KeyEvent, ch: char) -> bool {
    !ch.is_control()
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
}

#[derive(Debug)]
struct CredentialPrompt {
    provider: String,
    force: bool,
    input: MessageBox,
}

#[derive(Debug)]
struct CredentialPromptSpec {
    provider: String,
    force: bool,
}

#[derive(Debug)]
struct SideQuestionPrompt {
    id: String,
    question: String,
    input: MessageBox,
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
    match mouse.kind {
        MouseEventKind::ScrollUp if rect_contains(areas.transcript, mouse.column, mouse.row) => {
            scroll_transcript(state, TRANSCRIPT_MOUSE_SCROLL_STEP);
        }
        MouseEventKind::ScrollDown if rect_contains(areas.transcript, mouse.column, mouse.row) => {
            state.transcript_scroll = state
                .transcript_scroll
                .saturating_sub(TRANSCRIPT_MOUSE_SCROLL_STEP);
            state.last_event = transcript_scroll_event(state);
        }
        MouseEventKind::ScrollUp if rect_contains(areas.tools, mouse.column, mouse.row) => {
            handle_tools_scroll_mouse(state, mouse, areas.tools, true);
        }
        MouseEventKind::ScrollDown if rect_contains(areas.tools, mouse.column, mouse.row) => {
            handle_tools_scroll_mouse(state, mouse, areas.tools, false);
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
) {
    if handle_resume_picker_mouse_for_state(state, mouse, tools_area) {
        return;
    }
    if handle_command_palette_mouse_for_state(state, mouse, tools_area) {
        return;
    }
    if handle_approvals_mouse_for_state(state, mouse, tools_area) {
        return;
    }
    if state.monitor_tab == MonitorTab::Tools {
        if upward {
            move_selected_tool_by(state, false, TOOL_MOUSE_SCROLL_STEP);
        } else {
            move_selected_tool_by(state, true, TOOL_MOUSE_SCROLL_STEP);
        }
    } else if state.monitor_tab == MonitorTab::Changes {
        if upward {
            scroll_change_patch_up(state, CHANGE_PATCH_MOUSE_SCROLL_STEP);
        } else {
            scroll_change_patch_down(state, CHANGE_PATCH_MOUSE_SCROLL_STEP);
        }
    } else if upward {
        scroll_result_from_mouse(state, RESULT_MOUSE_SCROLL_STEP);
    } else {
        scroll_result_down(state, RESULT_MOUSE_SCROLL_STEP);
    }
}

fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn handle_tui_paste(state: &mut TuiState, text: &str) {
    let text = normalize_pasted_text(text);
    if text.is_empty() {
        return;
    }
    let char_count = text.chars().count();
    if let Some(prompt) = &mut state.credential_prompt {
        prompt.input.insert_str(&text);
        state.last_event = format!("pasted {char_count} hidden char(s)");
        return;
    }
    if let Some(prompt) = &mut state.side_question_prompt {
        prompt.input.insert_str(&text);
        state.last_event = format!("pasted {char_count} char(s) into btw answer");
        return;
    }
    if let Some(picker) = &mut state.resume_picker {
        picker.push_query_str(&text);
        state.last_event = format!("pasted {char_count} char(s) into resume filter");
        return;
    }

    state.input.insert_str(&text);
    clamp_selected_command(state);
    state.last_event = format!("pasted {char_count} char(s)");
}

fn normalize_pasted_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn handle_tui_key(
    key: KeyEvent,
    state: &mut TuiState,
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
) -> Result<()> {
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

fn handle_transcript_scroll_key(key: KeyEvent, state: &mut TuiState) -> bool {
    match key.code {
        KeyCode::PageUp => {
            scroll_transcript(state, TRANSCRIPT_SCROLL_STEP);
            true
        }
        KeyCode::PageDown => {
            state.transcript_scroll = state
                .transcript_scroll
                .saturating_sub(TRANSCRIPT_SCROLL_STEP);
            state.last_event = transcript_scroll_event(state);
            true
        }
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.transcript_scroll = state.chat.len().saturating_sub(1);
            state.last_event = transcript_scroll_event(state);
            true
        }
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.transcript_scroll = 0;
            state.last_event = transcript_scroll_event(state);
            true
        }
        _ => false,
    }
}

fn handle_result_scroll_key(key: KeyEvent, state: &mut TuiState) -> bool {
    if state.monitor_tab != MonitorTab::Result || !state.input.buffer().trim().is_empty() {
        return false;
    }
    match key.code {
        KeyCode::PageUp => {
            scroll_result(state, RESULT_SCROLL_STEP);
            true
        }
        KeyCode::PageDown => {
            scroll_result_down(state, RESULT_SCROLL_STEP);
            true
        }
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.result_scroll = result_output_line_count(state).saturating_sub(1);
            state.last_event = result_scroll_event(state);
            true
        }
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.result_scroll = 0;
            state.last_event = result_scroll_event(state);
            true
        }
        _ => false,
    }
}

fn handle_changes_tab_key(key: KeyEvent, state: &mut TuiState) -> bool {
    if state.monitor_tab != MonitorTab::Changes || !state.input.buffer().trim().is_empty() {
        return false;
    }
    match key.code {
        KeyCode::Char(']') if key.modifiers.is_empty() => {
            select_next_change_patch(state);
            true
        }
        KeyCode::Char('[') if key.modifiers.is_empty() => {
            select_previous_change_patch(state);
            true
        }
        KeyCode::PageDown => {
            scroll_change_patch_down(state, CHANGE_PATCH_SCROLL_STEP);
            true
        }
        KeyCode::PageUp => {
            scroll_change_patch_up(state, CHANGE_PATCH_SCROLL_STEP);
            true
        }
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.change_patch_scroll = 0;
            state.last_event = change_patch_scroll_event(state);
            true
        }
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.change_patch_scroll = selected_change_patch_line_count(state).saturating_sub(1);
            state.last_event = change_patch_scroll_event(state);
            true
        }
        _ => false,
    }
}

fn handle_tools_tab_key(key: KeyEvent, state: &mut TuiState) -> bool {
    if state.monitor_tab != MonitorTab::Tools || !state.input.buffer().trim().is_empty() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            move_selected_tool_by(state, false, 1);
            true
        }
        KeyCode::Down => {
            move_selected_tool_by(state, true, 1);
            true
        }
        KeyCode::PageUp => {
            move_selected_tool_by(state, false, TOOL_KEY_SCROLL_STEP);
            true
        }
        KeyCode::PageDown => {
            move_selected_tool_by(state, true, TOOL_KEY_SCROLL_STEP);
            true
        }
        KeyCode::Home => {
            select_tool_at_index(state, 0);
            true
        }
        KeyCode::End => {
            select_tool_at_index(state, state.tool_log.len().saturating_sub(1));
            true
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            prefill_tools_session_command(state, false);
            true
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            prefill_tools_session_command(state, true);
            true
        }
        KeyCode::Enter
            if key.modifiers.is_empty() || key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            toggle_selected_tool(state);
            true
        }
        _ => false,
    }
}

fn prefill_tools_session_command(state: &mut TuiState, failed_only: bool) {
    let command = if failed_only {
        "/session tools --failed --limit 20 --current"
    } else {
        "/session tools --limit 20 --current"
    };
    state.input.set_buffer(command.to_string());
    state.selected_command = 0;
    state.last_event = if failed_only {
        "prefilled failed tool output command".to_string()
    } else {
        "prefilled tool output command".to_string()
    };
}

fn select_next_change_patch(state: &mut TuiState) {
    let Some(count) = change_patch_count(state) else {
        state.last_event = "change patch unavailable".to_string();
        return;
    };
    state.selected_change = (state.selected_change + 1).min(count.saturating_sub(1));
    state.change_patch_scroll = 0;
    state.last_event = selected_change_patch_event(state);
}

fn select_previous_change_patch(state: &mut TuiState) {
    if change_patch_count(state).is_none() {
        state.last_event = "change patch unavailable".to_string();
        return;
    }
    state.selected_change = state.selected_change.saturating_sub(1);
    state.change_patch_scroll = 0;
    state.last_event = selected_change_patch_event(state);
}

fn scroll_change_patch_down(state: &mut TuiState, amount: usize) {
    let max_scroll = selected_change_patch_line_count(state).saturating_sub(1);
    if max_scroll == 0 {
        state.last_event = change_patch_scroll_event(state);
        return;
    }
    state.change_patch_scroll = state
        .change_patch_scroll
        .saturating_add(amount)
        .min(max_scroll);
    state.last_event = change_patch_scroll_event(state);
}

fn scroll_change_patch_up(state: &mut TuiState, amount: usize) {
    if selected_change_patch_line_count(state) == 0 {
        state.last_event = change_patch_scroll_event(state);
        return;
    }
    state.change_patch_scroll = state.change_patch_scroll.saturating_sub(amount);
    state.last_event = change_patch_scroll_event(state);
}

fn change_patch_count(state: &TuiState) -> Option<usize> {
    let count = state
        .workspace_changes
        .as_ref()
        .filter(|snapshot| snapshot.available)
        .map(|snapshot| snapshot.diff_sections.len())
        .unwrap_or_default();
    (count > 0).then_some(count)
}

fn selected_change_patch_line_count(state: &TuiState) -> usize {
    selected_change_patch_section(state)
        .map(|section| section.lines.len())
        .unwrap_or_default()
}

fn selected_change_patch_section(state: &TuiState) -> Option<&WorkspaceDiffSection> {
    let snapshot = state.workspace_changes.as_ref()?;
    if snapshot.diff_sections.is_empty() {
        return None;
    }
    snapshot
        .diff_sections
        .get(state.selected_change.min(snapshot.diff_sections.len() - 1))
}

fn selected_change_patch_event(state: &TuiState) -> String {
    selected_change_patch_section(state)
        .map(|section| {
            format!(
                "selected change patch {}",
                compact_ui_text(&section.path, 70)
            )
        })
        .unwrap_or_else(|| "change patch unavailable".to_string())
}

fn change_patch_scroll_event(state: &TuiState) -> String {
    if selected_change_patch_section(state).is_none() {
        return "change patch unavailable".to_string();
    }
    if state.change_patch_scroll == 0 {
        "change patch at top".to_string()
    } else {
        format!("change patch scrolled {}", state.change_patch_scroll)
    }
}

fn select_change_patch_at_row(
    state: &mut TuiState,
    tools_area: Rect,
    column: u16,
    row: u16,
) -> bool {
    if state.monitor_tab != MonitorTab::Changes
        || !state.input.buffer().trim().is_empty()
        || state.resume_picker.is_some()
        || state.credential_prompt.is_some()
        || state.side_question_prompt.is_some()
        || slash_command_suggestions_for_state(state.input.buffer(), state.running).is_some()
        || !rect_contains(tools_area, column, row)
        || !rect_content_row_contains(tools_area, row)
    {
        return false;
    }

    let content_index = row.saturating_sub(tools_area.y + 1) as usize;
    let monitor = session_monitor_for_state(state);
    let text = format_task_monitor_text(state, monitor.as_ref(), tools_area.height);
    let lines = text.lines().collect::<Vec<_>>();
    let Some(worktree_files_row) = lines
        .iter()
        .position(|line| line.trim_start() == "worktree files:")
    else {
        return false;
    };

    let mut clicked_path_index = None;
    for (path_index, (line_index, _)) in lines
        .iter()
        .enumerate()
        .skip(worktree_files_row + 1)
        .take_while(|(_, line)| line.trim_start().starts_with("- "))
        .enumerate()
    {
        if line_index == content_index {
            clicked_path_index = Some(path_index);
            break;
        }
    }
    let Some(path_index) = clicked_path_index else {
        return false;
    };

    let Some((path, section_index)) = state
        .workspace_changes
        .as_ref()
        .filter(|snapshot| snapshot.available)
        .and_then(|snapshot| {
            let path = snapshot.paths.get(path_index)?.clone();
            let section_index = snapshot
                .diff_sections
                .iter()
                .position(|section| section.path == path);
            Some((path, section_index))
        })
    else {
        return false;
    };

    if let Some(section_index) = section_index {
        state.selected_change = section_index;
        state.change_patch_scroll = 0;
        state.last_event = selected_change_patch_event(state);
    } else {
        state.last_event = format!("no patch for {}", compact_ui_text(&path, 70));
    }
    true
}

fn clamp_selected_change_patch(state: &mut TuiState) {
    let count = state
        .workspace_changes
        .as_ref()
        .map(|snapshot| snapshot.diff_sections.len())
        .unwrap_or_default();
    if count == 0 {
        state.selected_change = 0;
        state.change_patch_scroll = 0;
        return;
    }
    state.selected_change = state.selected_change.min(count - 1);
    let max_scroll = selected_change_patch_line_count(state).saturating_sub(1);
    state.change_patch_scroll = state.change_patch_scroll.min(max_scroll);
}

fn scroll_result_from_mouse(state: &mut TuiState, amount: usize) {
    if state.monitor_tab == MonitorTab::Result && state.input.buffer().trim().is_empty() {
        scroll_result(state, amount);
    }
}

fn scroll_result(state: &mut TuiState, amount: usize) {
    let max_scroll = result_output_line_count(state).saturating_sub(1);
    state.result_scroll = state.result_scroll.saturating_add(amount).min(max_scroll);
    state.last_event = result_scroll_event(state);
}

fn scroll_result_down(state: &mut TuiState, amount: usize) {
    if state.monitor_tab != MonitorTab::Result || !state.input.buffer().trim().is_empty() {
        return;
    }
    state.result_scroll = state.result_scroll.saturating_sub(amount);
    state.last_event = result_scroll_event(state);
}

fn result_scroll_event(state: &TuiState) -> String {
    if latest_action_result(state).is_none() {
        return "result output unavailable".to_string();
    }
    if state.result_scroll == 0 {
        "result output at latest".to_string()
    } else {
        format!("result output scrolled back {}", state.result_scroll)
    }
}

fn result_output_line_count(state: &TuiState) -> usize {
    latest_action_result(state)
        .map(|result| non_empty_output_lines(result.content).len())
        .unwrap_or_default()
}

fn scroll_transcript(state: &mut TuiState, amount: usize) {
    let max_scroll = state.chat.len().saturating_sub(1);
    state.transcript_scroll = state
        .transcript_scroll
        .saturating_add(amount)
        .min(max_scroll);
    state.last_event = transcript_scroll_event(state);
}

fn transcript_scroll_event(state: &TuiState) -> String {
    if state.transcript_scroll == 0 {
        "messages at latest".to_string()
    } else {
        format!("messages scrolled back {}", state.transcript_scroll)
    }
}

fn handle_approval_tab_key(key: KeyEvent, state: &mut TuiState) -> bool {
    if state.monitor_tab != MonitorTab::Approvals || !state.input.buffer().trim().is_empty() {
        return false;
    }
    let Some(count) = blocker_count(state) else {
        return false;
    };
    if count == 0 {
        return false;
    }
    match key.code {
        KeyCode::Up | KeyCode::Left => {
            state.selected_approval = state.selected_approval.saturating_sub(1);
            true
        }
        KeyCode::Down | KeyCode::Right => {
            state.selected_approval = (state.selected_approval + 1).min(count - 1);
            true
        }
        KeyCode::Enter => {
            activate_selected_blocker(state);
            true
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            deny_selected_blocker(state);
            true
        }
        _ => false,
    }
}

fn handle_approvals_mouse_for_state(
    state: &mut TuiState,
    mouse: MouseEvent,
    tools_area: Rect,
) -> bool {
    if state.monitor_tab != MonitorTab::Approvals
        || !state.input.buffer().trim().is_empty()
        || state.resume_picker.is_some()
        || state.credential_prompt.is_some()
        || state.side_question_prompt.is_some()
        || slash_command_suggestions_for_state(state.input.buffer(), state.running).is_some()
        || !rect_contains(tools_area, mouse.column, mouse.row)
    {
        return false;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            let Some(count) = blocker_count(state) else {
                return false;
            };
            if count == 0 {
                return false;
            }
            state.selected_approval = state.selected_approval.saturating_sub(1);
            state.last_event = selected_blocker_event(state);
            true
        }
        MouseEventKind::ScrollDown => {
            let Some(count) = blocker_count(state) else {
                return false;
            };
            if count == 0 {
                return false;
            }
            state.selected_approval = state.selected_approval.saturating_add(1).min(count - 1);
            state.last_event = selected_blocker_event(state);
            true
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(index) =
                clicked_approvals_tab_index(state, tools_area, mouse.column, mouse.row)
            else {
                return false;
            };
            state.selected_approval = index;
            state.last_event = selected_blocker_event(state);
            true
        }
        _ => false,
    }
}

fn handle_monitor_quick_action_key(
    key: KeyEvent,
    state: &mut TuiState,
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
) -> bool {
    if !key.modifiers.is_empty() || !state.input.buffer().trim().is_empty() {
        return false;
    }
    let monitor = session_monitor_for_state(state);
    let actions = monitor_quick_actions_for_tab(state, monitor.as_ref());
    if actions.is_empty() {
        return false;
    }
    match key.code {
        KeyCode::Up | KeyCode::Left => {
            state.selected_command = state.selected_command.saturating_sub(1);
            state.last_event = selected_quick_action_event(state.selected_command, &actions);
            true
        }
        KeyCode::Down | KeyCode::Right => {
            state.selected_command = (state.selected_command + 1).min(actions.len() - 1);
            state.last_event = selected_quick_action_event(state.selected_command, &actions);
            true
        }
        KeyCode::Enter => {
            activate_selected_monitor_quick_action(state, &actions, progress_tx, done_tx);
            true
        }
        _ => false,
    }
}

fn selected_quick_action_event(selected: usize, actions: &[MonitorQuickAction]) -> String {
    let selected = selected.min(actions.len().saturating_sub(1));
    let command = actions
        .get(selected)
        .map(|action| compact_ui_text(&action.command, 70))
        .unwrap_or_else(|| "<none>".to_string());
    format!("quick action selected: {command}")
}

fn activate_selected_monitor_quick_action(
    state: &mut TuiState,
    actions: &[MonitorQuickAction],
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
) {
    let Some(action) = actions
        .get(state.selected_command.min(actions.len().saturating_sub(1)))
        .cloned()
    else {
        return;
    };
    state.selected_command = 0;
    if action.edit_before_run {
        state.input.set_buffer(action.command.clone());
        state.last_event = format!(
            "quick action ready for edit: {}",
            compact_ui_text(&action.command, 70)
        );
        return;
    }
    state.last_event = format!(
        "quick action submitted: {}",
        compact_ui_text(&action.command, 70)
    );
    submit_tui_input(state, action.command, progress_tx.clone(), done_tx.clone());
}

fn activate_monitor_quick_action_at_row(
    state: &mut TuiState,
    tools_area: Rect,
    row: u16,
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
) -> bool {
    if !state.input.buffer().trim().is_empty()
        || state.resume_picker.is_some()
        || slash_command_suggestions_for_state(state.input.buffer(), state.running).is_some()
        || !rect_content_row_contains(tools_area, row)
    {
        return false;
    }
    let monitor = session_monitor_for_state(state);
    let actions = monitor_quick_actions_for_tab(state, monitor.as_ref());
    let Some(index) =
        clicked_monitor_quick_action_index(state, monitor.as_ref(), tools_area, row, &actions)
    else {
        return false;
    };
    state.selected_command = index;
    activate_selected_monitor_quick_action(state, &actions, progress_tx, done_tx);
    true
}

fn clicked_monitor_quick_action_index(
    state: &TuiState,
    monitor: Option<&SessionMonitor>,
    tools_area: Rect,
    row: u16,
    actions: &[MonitorQuickAction],
) -> Option<usize> {
    if actions.is_empty() || !rect_content_row_contains(tools_area, row) {
        return None;
    }
    let content_index = row.saturating_sub(tools_area.y + 1) as usize;
    let text = format_task_monitor_text(state, monitor, tools_area.height);
    let line = text.lines().nth(content_index)?;
    let trimmed = line.trim_start();
    let command = trimmed.strip_prefix("> ").unwrap_or(trimmed);
    let command = command.strip_suffix(" (edit)").unwrap_or(command);
    if !command.starts_with('/') {
        return None;
    }
    actions.iter().position(|action| action.command == command)
}

fn rect_content_row_contains(area: Rect, row: u16) -> bool {
    row > area.y && row < area.y.saturating_add(area.height).saturating_sub(1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectedBlocker {
    Approval(String),
    SideQuestion(String),
}

fn blocker_count(state: &TuiState) -> Option<usize> {
    session_monitor_for_state(state)
        .map(|monitor| monitor.pending_approvals.len() + monitor.open_questions.len())
}

fn selected_blocker(state: &TuiState) -> Option<SelectedBlocker> {
    let monitor = session_monitor_for_state(state)?;
    let approval_count = monitor.pending_approvals.len();
    let total = approval_count + monitor.open_questions.len();
    if total == 0 {
        return None;
    }
    let index = state.selected_approval.min(total - 1);
    if index < approval_count {
        Some(SelectedBlocker::Approval(
            monitor.pending_approvals[index].id.clone(),
        ))
    } else {
        Some(SelectedBlocker::SideQuestion(
            monitor.open_questions[index - approval_count].id.clone(),
        ))
    }
}

fn clicked_approvals_tab_index(
    state: &TuiState,
    tools_area: Rect,
    column: u16,
    row: u16,
) -> Option<usize> {
    if !rect_contains(tools_area, column, row) || !rect_content_row_contains(tools_area, row) {
        return None;
    }
    let monitor = session_monitor_for_state(state)?;
    let content_row = row.saturating_sub(tools_area.y + 1) as usize;
    let text = format_task_monitor_text(state, Some(&monitor), tools_area.height);
    let line = text.lines().nth(content_row)?.trim_start();
    if !(line.starts_with("* ") || line.starts_with("- ")) {
        return None;
    }
    for (index, approval) in monitor.pending_approvals.iter().enumerate() {
        if line.contains(short_id(&approval.id)) {
            return Some(index);
        }
    }
    let approval_count = monitor.pending_approvals.len();
    for (index, question) in monitor.open_questions.iter().enumerate() {
        if line.contains(short_id(&question.id)) {
            return Some(approval_count + index);
        }
    }
    None
}

fn selected_blocker_event(state: &TuiState) -> String {
    match selected_blocker(state) {
        Some(SelectedBlocker::Approval(id)) => {
            format!("approval selected: {}", short_id(&id))
        }
        Some(SelectedBlocker::SideQuestion(id)) => {
            format!("btw selected: {}", short_id(&id))
        }
        None => "approval selection unavailable".to_string(),
    }
}

fn activate_selected_blocker(state: &mut TuiState) {
    match selected_blocker(state) {
        Some(SelectedBlocker::Approval(_)) => update_selected_approval(state, true),
        Some(SelectedBlocker::SideQuestion(id)) => open_side_question_answer_prompt(state, &id),
        None => {
            state.selected_approval = 0;
            state.last_event = "no pending blockers".to_string();
        }
    }
}

fn deny_selected_blocker(state: &mut TuiState) {
    match selected_blocker(state) {
        Some(SelectedBlocker::Approval(_)) => update_selected_approval(state, false),
        Some(SelectedBlocker::SideQuestion(_)) => {
            state.last_event = "btw questions cannot be denied; press Enter to answer".to_string();
        }
        None => {
            state.selected_approval = 0;
            state.last_event = "no pending blockers".to_string();
        }
    }
}

fn open_side_question_answer_prompt(state: &mut TuiState, question_id: &str) {
    let question = session_monitor_for_state(state).and_then(|monitor| {
        monitor
            .open_questions
            .into_iter()
            .find(|question| question.id == question_id)
            .map(|question| question.question)
    });
    let Some(question) = question else {
        state.last_event = "btw question not found".to_string();
        return;
    };
    state.side_question_prompt = Some(SideQuestionPrompt {
        id: question_id.to_string(),
        question: question.clone(),
        input: MessageBox::new(),
    });
    state.chat.push(ChatLine {
        role: "deepcli".to_string(),
        content: format!("请回答旁路问题：{question}"),
    });
    state.last_event = "btw answer prompt opened".to_string();
}

fn handle_side_question_prompt_key(key: KeyEvent, state: &mut TuiState) {
    match key.code {
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => handle_prompt_input_key(
            state
                .side_question_prompt
                .as_mut()
                .map(|prompt| &mut prompt.input),
            key,
        ),
        KeyCode::Enter => confirm_side_question_prompt(state),
        _ => handle_prompt_input_key(
            state
                .side_question_prompt
                .as_mut()
                .map(|prompt| &mut prompt.input),
            key,
        ),
    }
}

fn handle_prompt_input_key(input: Option<&mut MessageBox>, key: KeyEvent) {
    if let Some(input) = input {
        input.handle_key(key);
    }
}

fn confirm_side_question_prompt(state: &mut TuiState) {
    let Some(prompt) = state.side_question_prompt.as_ref() else {
        return;
    };
    let answer = prompt.input.buffer().trim().to_string();
    if answer.is_empty() {
        state.chat.push(ChatLine {
            role: "error".to_string(),
            content: "btw answer 不能为空。".to_string(),
        });
        state.last_event = "btw answer rejected".to_string();
        return;
    }
    let id = prompt.id.clone();
    match answer_side_question_for_state(state, &id, &answer) {
        Ok(message) => {
            state.side_question_prompt = None;
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

fn update_selected_approval(state: &mut TuiState, approved: bool) {
    let selected = {
        let Some(monitor) = session_monitor_for_state(state) else {
            state.last_event = "failed to read approval requests".to_string();
            return;
        };
        if monitor.pending_approvals.is_empty() {
            state.selected_approval = 0;
            state.last_event = "no pending approval requests".to_string();
            return;
        }
        let index = state
            .selected_approval
            .min(monitor.pending_approvals.len() - 1);
        monitor.pending_approvals[index].id.clone()
    };

    match update_approval_for_state(state, &selected, approved) {
        Ok(message) => {
            let action = if approved {
                "approval approved"
            } else {
                "approval denied"
            };
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: message.clone(),
            });
            state.last_event = action.to_string();
            clamp_selected_blocker_to_monitor(state);
        }
        Err(error) => {
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            });
            state.last_event = "approval action failed".to_string();
        }
    }
}

fn update_approval_for_state(state: &mut TuiState, id: &str, approved: bool) -> Result<String> {
    if let Some(runtime) = state.runtime.as_mut() {
        return runtime.update_current_approval(id, approved);
    }
    let active = state
        .active_session
        .as_ref()
        .ok_or_else(|| anyhow!("当前运行会话不可用"))?;
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let status = if approved {
        ApprovalStatus::Approved
    } else {
        ApprovalStatus::Denied
    };
    let item = session.update_approval_request(id, status)?;
    let action = if approved { "approved" } else { "denied" };
    Ok(format!(
        "{action} approval request {} for tool {}",
        item.id, item.tool
    ))
}

fn clamp_selected_blocker_to_monitor(state: &mut TuiState) {
    let remaining = session_monitor_for_state(state)
        .map(|monitor| monitor.pending_approvals.len() + monitor.open_questions.len())
        .unwrap_or_default();
    if remaining == 0 {
        state.selected_approval = 0;
    } else {
        state.selected_approval = state.selected_approval.min(remaining - 1);
    }
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

fn select_monitor_tab_at_position(
    state: &mut TuiState,
    tools_area: Rect,
    column: u16,
    row: u16,
) -> bool {
    if state.resume_picker.is_some()
        || slash_command_suggestions_for_state(state.input.buffer(), state.running).is_some()
        || !rect_contains(tools_area, column, row)
        || row != tools_area.y.saturating_add(1)
        || column <= tools_area.x
    {
        return false;
    }

    let relative_column = column.saturating_sub(tools_area.x + 1) as usize;
    let mut offset = 0usize;
    for tab in MonitorTab::all() {
        let rendered = if *tab == state.monitor_tab {
            format!("[{}]", tab.label())
        } else {
            tab.label().to_string()
        };
        let end = offset + rendered.len();
        if (offset..end).contains(&relative_column) {
            state.monitor_tab = *tab;
            state.selected_command = 0;
            state.last_event = format!("monitor tab: {}", state.monitor_tab.label());
            return true;
        }
        offset = end + 1;
    }
    false
}

fn handle_command_palette_key(key: KeyEvent, state: &mut TuiState) -> bool {
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

fn handle_command_palette_mouse_for_state(
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

fn complete_selected_command(state: &mut TuiState, suggestions: &[CommandHelpSummary]) {
    if let Some(selected) = suggestions.get(state.selected_command.min(suggestions.len() - 1)) {
        state.input.set_buffer(format!("{} ", selected.name));
        state.selected_command = 0;
        state.last_event = format!("completed {}", selected.name);
    }
}

fn clamp_selected_command(state: &mut TuiState) {
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

fn handle_credential_prompt_key(key: KeyEvent, state: &mut TuiState) {
    match key.code {
        KeyCode::Enter => confirm_credential_prompt(state),
        _ => handle_prompt_input_key(
            state
                .credential_prompt
                .as_mut()
                .map(|prompt| &mut prompt.input),
            key,
        ),
    }
}

fn confirm_credential_prompt(state: &mut TuiState) {
    let Some(prompt) = state.credential_prompt.take() else {
        return;
    };
    let api_key = prompt.input.buffer().trim().to_string();
    if api_key.is_empty() {
        state.chat.push(ChatLine {
            role: "error".to_string(),
            content: "apiKey 不能为空。".to_string(),
        });
        state.last_event = "credential prompt rejected".to_string();
        return;
    }
    let Some(runtime) = state.runtime.as_mut() else {
        state.chat.push(ChatLine {
            role: "error".to_string(),
            content: "当前 runtime 不可用。".to_string(),
        });
        return;
    };
    match runtime.store_provider_api_key(&prompt.provider, api_key, prompt.force) {
        Ok(output) => {
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: output,
            });
            state.last_event = "credentials updated".to_string();
        }
        Err(error) => {
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            });
            state.last_event = "credentials update failed".to_string();
        }
    }
}

fn handle_resume_picker_key(key: KeyEvent, state: &mut TuiState) {
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

fn handle_resume_picker_mouse_for_state(
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

fn handle_running_tui_local_command(state: &mut TuiState, input: &str) -> bool {
    if !state.running {
        return false;
    }
    let command = match CommandRouter::parse(input) {
        Ok(Some(command)) => command,
        Ok(None) => return false,
        Err(error) => {
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            });
            state.last_event = "running command parse failed".to_string();
            return true;
        }
    };
    match command {
        SlashCommand::Help { args } => {
            match CommandRouter::help_for(&args) {
                Ok(output) => {
                    state.chat.push(ChatLine {
                        role: "deepcli".to_string(),
                        content: output.clone(),
                    });
                    state.last_event = format_action_event("running command ok", &output);
                }
                Err(error) => {
                    let message = error.to_string();
                    state.chat.push(ChatLine {
                        role: "error".to_string(),
                        content: message.clone(),
                    });
                    state.last_event = format_action_event("running command failed", &message);
                }
            }
            true
        }
        SlashCommand::Status { args } => {
            if args.is_empty() {
                push_running_command_result(state, format_tui_running_status);
            } else {
                let message =
                    "Agent 运行中的 `/status` 只支持无参数；请在任务空闲后使用 `/status --json` 或 `/status --output ...`。"
                        .to_string();
                state.chat.push(ChatLine {
                    role: "error".to_string(),
                    content: message.clone(),
                });
                state.last_event = format_action_event("running command failed", &message);
            }
            true
        }
        SlashCommand::Usage { args } => {
            push_running_command_result(state, |active| {
                handle_usage(&active.workspace, Some(active.session_id.clone()), args)
            });
            true
        }
        SlashCommand::Btw { args } => {
            push_running_command_result(state, |active| handle_tui_running_btw(active, args));
            true
        }
        SlashCommand::Trace { args } => {
            push_running_command_result(state, |active| {
                handle_trace(&active.workspace, Some(active.session_id.clone()), args)
            });
            true
        }
        SlashCommand::Logs { args } => {
            push_running_command_result(state, |active| handle_logs(&active.workspace, args));
            true
        }
        SlashCommand::Selftest { args } => {
            push_running_command_result(state, |active| {
                handle_selftest_local(&active.workspace, args)
            });
            true
        }
        SlashCommand::Completion { args } => {
            push_running_command_result(state, |active| {
                handle_completion_local(&active.workspace, args)
            });
            true
        }
        SlashCommand::Approval { args } => {
            push_running_command_result(state, |active| {
                handle_approval(&active.workspace, Some(active.session_id.clone()), args)
            });
            true
        }
        SlashCommand::Session { args } => {
            push_running_command_result(state, |active| {
                if matches!(
                    args.first().map(String::as_str),
                    Some("restore-backup" | "restore")
                ) {
                    anyhow::bail!(
                        "/session restore-backup writes files; stop or wait for the running task before restoring"
                    );
                }
                handle_session(&active.workspace, Some(active.session_id.clone()), args)
            });
            true
        }
        SlashCommand::Terminal => {
            push_running_command_result(state, handle_tui_running_terminal);
            true
        }
        SlashCommand::Stop => {
            stop_running_task(state, false, "/stop");
            true
        }
        SlashCommand::Quit => {
            stop_running_task(state, true, "/quit");
            true
        }
        _ => {
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content:
                    "Agent 正在运行；当前支持本地 `/help`、`/status`、`/usage`、`/trace`、`/logs`、`/selftest`、`/completion`、`/approval`、`/session`、`/terminal`、`/stop`、`/quit` 和 `/btw ask/list/answer/clear`。"
                        .to_string(),
            });
            state.last_event = "running command unsupported".to_string();
            true
        }
    }
}

fn push_running_command_result<F>(state: &mut TuiState, action: F)
where
    F: FnOnce(&ActiveSessionRef) -> Result<String>,
{
    let Some(active) = state.active_session.as_ref() else {
        let message = "当前运行会话不可用。".to_string();
        state.result_scroll = 0;
        state.chat.push(ChatLine {
            role: "error".to_string(),
            content: message.clone(),
        });
        state.last_event = format_action_event("running command failed", &message);
        return;
    };
    match action(active) {
        Ok(output) => {
            state.result_scroll = 0;
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: output.clone(),
            });
            state.last_event = format_action_event("running command ok", &output);
        }
        Err(error) => {
            let message = error.to_string();
            state.result_scroll = 0;
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: message.clone(),
            });
            state.last_event = format_action_event("running command failed", &message);
        }
    }
}

fn format_tui_running_status(active: &ActiveSessionRef) -> Result<String> {
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let activity = session.activity_summary()?;
    let open_questions = session
        .load_side_questions()?
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let pending_approvals = session
        .load_approval_requests()?
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let plan = session.load_plan()?.map(|plan| {
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == crate::session::PlanStepStatus::Completed)
            .count();
        format!("{completed}/{} completed", plan.steps.len())
    });
    let latest_test = session
        .load_recent_test_runs(1)?
        .into_iter()
        .last()
        .map(|test| {
            format!(
                "latest_test={} {}",
                if test.passed { "pass" } else { "fail" },
                compact_ui_text(&test.command, 52)
            )
        })
        .unwrap_or_else(|| "latest_test=none".to_string());
    Ok(format!(
        "running session {}\nstate: {:?}\nprovider: {} model: {}\nactivity: messages={} tools={} tests={} side_questions={} approvals={} summary={}\nopen_btw={} pending_approvals={}\nplan: {}\n{}",
        session.id(),
        session.metadata.state,
        session.metadata.provider,
        session
            .metadata
            .model
            .clone()
            .unwrap_or_else(|| "<unset>".to_string()),
        activity.message_count,
        activity.tool_call_count,
        activity.test_run_count,
        activity.side_question_count,
        activity.approval_request_count,
        activity.has_summary,
        open_questions,
        pending_approvals,
        plan.unwrap_or_else(|| "none".to_string()),
        latest_test
    ))
}

fn handle_tui_running_btw(active: &ActiveSessionRef, args: Vec<String>) -> Result<String> {
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let include_all = args.iter().any(|arg| arg == "--all");
            Ok(format_tui_side_questions(
                &session.load_side_questions()?,
                include_all,
            ))
        }
        Some("ask") => {
            let question = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if question.trim().is_empty() {
                anyhow::bail!("/btw ask requires a question");
            }
            let item = session.enqueue_side_question(question.trim())?;
            Ok(format!(
                "queued by-the-way question {} while the main task keeps running: {}",
                item.id, item.question
            ))
        }
        Some("answer") => {
            let id = args
                .get(1)
                .ok_or_else(|| anyhow!("missing side question id"))?;
            let answer = args
                .iter()
                .skip(2)
                .filter(|arg| arg.as_str() != "--current")
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            if answer.trim().is_empty() {
                anyhow::bail!("/btw answer requires an answer");
            }
            let item = session.answer_side_question(id, answer.trim())?;
            Ok(format!("answered by-the-way question {}", item.id))
        }
        Some("clear") => {
            let cleared = session.clear_side_questions()?;
            Ok(format!("cleared {cleared} open by-the-way question(s)"))
        }
        Some(other) => anyhow::bail!("unsupported /btw action `{other}` while running"),
    }
}

fn handle_tui_running_terminal(active: &ActiveSessionRef) -> Result<String> {
    let config = AppConfig::load_effective(&active.workspace, None)?;
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let permissions = PermissionEngine::new(
        &active.workspace,
        config.permissions.clone(),
        config.sandbox.clone(),
    );
    let executor = ToolExecutor::new(
        &active.workspace,
        permissions,
        Some(session),
        config.agent.max_subagent_depth,
    );
    let output = executor.execute_open_terminal_now()?;
    let detail = output.content.trim();
    if detail.is_empty() {
        Ok(format!("opened terminal in {}", active.workspace.display()))
    } else {
        Ok(format!(
            "opened terminal in {}\n{}",
            active.workspace.display(),
            detail
        ))
    }
}

fn stop_running_task(state: &mut TuiState, exit_after: bool, source: &str) {
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

fn mark_active_session_paused(active: &ActiveSessionRef, source: &str) -> Result<()> {
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

fn rebuild_runtime_for_active_session(active: &ActiveSessionRef) -> Result<AgentRuntime> {
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

fn format_tui_side_questions(items: &[SideQuestion], include_all: bool) -> String {
    let lines = items
        .iter()
        .filter(|item| include_all || item.status == SideQuestionStatus::Open)
        .map(|item| {
            let answer = item
                .answer
                .as_ref()
                .map(|answer| format!(" answer={}", compact_ui_text(answer, 60)))
                .unwrap_or_default();
            format!(
                "{} [{}] {}{}",
                short_id(&item.id.to_string()),
                tui_side_question_status_label(&item.status),
                compact_ui_text(&item.question, 86),
                answer
            )
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        "no by-the-way questions".to_string()
    } else {
        lines.join("\n")
    }
}

fn tui_side_question_status_label(status: &SideQuestionStatus) -> &'static str {
    match status {
        SideQuestionStatus::Open => "open",
        SideQuestionStatus::Answered => "answered",
        SideQuestionStatus::Cleared => "cleared",
    }
}

fn submit_tui_input(
    state: &mut TuiState,
    input: String,
    progress_tx: Sender<RuntimeProgress>,
    done_tx: Sender<WorkerDone>,
) {
    if input.trim().is_empty() {
        return;
    }
    state.transcript_scroll = 0;
    state.result_scroll = 0;
    let trimmed = input.trim();
    if matches!(trimmed, "/quit" | "/exit") {
        if state.running {
            stop_running_task(state, true, trimmed);
            return;
        }
        state.exit_requested = true;
        return;
    }
    if handle_running_tui_local_command(state, trimmed) {
        return;
    }
    if handle_tui_local_command(state, trimmed) {
        return;
    }
    if state.running {
        state.chat.push(ChatLine {
            role: "deepcli".to_string(),
            content:
                "Agent 正在运行；当前可用 `/help`、`/status`、`/usage`、`/trace`、`/logs`、`/selftest`、`/completion`、`/approval`、`/session`、`/terminal`、`/stop`、`/quit` 或 `/btw ask/list/answer/clear` 处理旁路事项。"
                    .to_string(),
        });
        state.last_event = "input deferred while running".to_string();
        return;
    }
    sync_active_session_ref(state);
    let Some(mut runtime) = state.runtime.take() else {
        return;
    };
    runtime.set_progress_sender(Some(progress_tx));
    state.chat.push(ChatLine {
        role: "你".to_string(),
        content: input.clone(),
    });
    state.running = true;
    state.last_event = "running".to_string();
    state.worker = Some(tokio::spawn(async move {
        let result = runtime
            .handle_input(&input)
            .await
            .map_err(|error| error.to_string());
        let _ = done_tx.send(WorkerDone { runtime, result });
    }));
}

fn handle_tui_local_command(state: &mut TuiState, input: &str) -> bool {
    if let Some(parsed) = parse_tui_credential_set(input) {
        match parsed {
            Ok(spec) => {
                state.credential_prompt = Some(CredentialPrompt {
                    provider: spec.provider.clone(),
                    force: spec.force,
                    input: MessageBox::new(),
                });
                state.chat.push(ChatLine {
                    role: "deepcli".to_string(),
                    content: format!(
                        "请输入 `{}` 的 API key。输入内容会隐藏显示，Enter 保存，Esc 取消。",
                        spec.provider
                    ),
                });
                state.last_event = "credential prompt opened".to_string();
            }
            Err(error) => state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            }),
        }
        return true;
    }

    if input == "/resume" {
        let Some(runtime) = state.runtime.as_mut() else {
            return false;
        };
        match runtime.list_sessions() {
            Ok(sessions) if sessions.is_empty() => state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: "当前目录没有可恢复的历史会话。可用 `/session list --all` 查看空会话。"
                    .to_string(),
            }),
            Ok(sessions) => {
                state.resume_picker = Some(ResumePicker::new(sessions));
            }
            Err(error) => state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            }),
        }
        return true;
    }

    if let Some(id) = input.strip_prefix("/resume ") {
        let result = {
            let Some(runtime) = state.runtime.as_mut() else {
                return false;
            };
            runtime
                .resume_session(id.trim())
                .map(|message| (message, chat_lines_from_runtime(runtime)))
        };
        apply_resume_result(state, result);
        return true;
    }

    if input == "/rename" {
        let Some(runtime) = state.runtime.as_mut() else {
            return false;
        };
        match runtime.rename_current_session("") {
            Ok(message) => state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: message,
            }),
            Err(error) => state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            }),
        }
        return true;
    }

    if let Some(title) = input.strip_prefix("/rename ") {
        let Some(runtime) = state.runtime.as_mut() else {
            return false;
        };
        match runtime.rename_current_session(title.trim()) {
            Ok(message) => state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: message,
            }),
            Err(error) => state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            }),
        }
        return true;
    }

    false
}

fn parse_tui_credential_set(input: &str) -> Option<Result<CredentialPromptSpec>> {
    let trimmed = input.trim();
    if !trimmed.starts_with("/credentials") {
        return None;
    }
    let parts = match shell_words::split(trimmed) {
        Ok(parts) => parts,
        Err(error) => return Some(Err(anyhow!("failed to parse credentials command: {error}"))),
    };
    if parts.first().map(String::as_str) != Some("/credentials")
        || parts.get(1).map(String::as_str) != Some("set")
    {
        return None;
    }
    let Some(provider) = parts.get(2).filter(|value| !value.trim().is_empty()) else {
        return Some(Err(anyhow!("missing provider name")));
    };
    let mut force = false;
    for arg in parts.iter().skip(3) {
        match arg.as_str() {
            "--force" => force = true,
            "--stdin" => {
                return Some(Err(anyhow!(
                    "TUI 已提供隐藏输入框，请去掉 --stdin 后重新执行"
                )));
            }
            other => {
                return Some(Err(anyhow!(
                    "unsupported /credentials set option `{other}`"
                )))
            }
        }
    }
    Some(Ok(CredentialPromptSpec {
        provider: provider.to_string(),
        force,
    }))
}

fn apply_resume_result(state: &mut TuiState, result: Result<(String, Result<Vec<ChatLine>>)>) {
    match result {
        Ok((message, Ok(mut chat))) => {
            chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: message,
            });
            state.chat = chat;
            state.tool_log.clear();
            state.selected_tool = None;
            state.monitor_tab = MonitorTab::Overview;
            sync_active_session_ref(state);
            state.last_event = "session resumed".to_string();
        }
        Ok((message, Err(error))) => {
            state.chat = vec![
                ChatLine {
                    role: "deepcli".to_string(),
                    content: message,
                },
                ChatLine {
                    role: "error".to_string(),
                    content: format!("读取历史会话失败：{error}"),
                },
            ];
            state.tool_log.clear();
            state.selected_tool = None;
            state.monitor_tab = MonitorTab::Overview;
            sync_active_session_ref(state);
            state.last_event = "session resumed with history error".to_string();
        }
        Err(error) => state.chat.push(ChatLine {
            role: "error".to_string(),
            content: error.to_string(),
        }),
    }
}

fn chat_lines_from_runtime(runtime: &AgentRuntime) -> Result<Vec<ChatLine>> {
    Ok(session_messages_to_chat_lines(runtime.session_messages()?))
}

fn session_messages_to_chat_lines(messages: Vec<SessionMessage>) -> Vec<ChatLine> {
    messages
        .into_iter()
        .filter(|message| !message.content.trim().is_empty())
        .map(|message| ChatLine {
            role: match message.role.as_str() {
                "user" => "你".to_string(),
                "assistant" => "deepcli".to_string(),
                other => other.to_string(),
            },
            content: truncate_history_message(&message.content),
        })
        .collect()
}

fn truncate_history_message(content: &str) -> String {
    let char_count = content.chars().count();
    if char_count <= TUI_HISTORY_MESSAGE_CHARS {
        return content.to_string();
    }
    let mut truncated = content
        .chars()
        .take(TUI_HISTORY_MESSAGE_CHARS)
        .collect::<String>();
    truncated.push_str(&format!(
        "\n[deepcli truncated UI history: original_chars={char_count}]"
    ));
    truncated
}

fn drain_progress(state: &mut TuiState, progress_rx: &Receiver<RuntimeProgress>) {
    while let Ok(event) = progress_rx.try_recv() {
        state.last_event = event.plain_text();
        match event {
            RuntimeProgress::ToolStarted { tool } => {
                state.tool_log.push(ToolLogItem {
                    title: format!("tool: {tool}"),
                    detail: format!("正在运行工具 `{tool}`"),
                    expanded: false,
                });
                if state.selected_tool.is_none() {
                    state.selected_tool = Some(0);
                }
            }
            RuntimeProgress::ToolCompleted { tool, ok, summary } => {
                let status = if ok { "done" } else { "failed" };
                if let Some(item) = state
                    .tool_log
                    .iter_mut()
                    .rev()
                    .find(|item| item.title == format!("tool: {tool}"))
                {
                    item.title = format!("tool: {tool} [{status}]");
                    item.detail = summary;
                } else {
                    state.tool_log.push(ToolLogItem {
                        title: format!("tool: {tool} [{status}]"),
                        detail: summary,
                        expanded: false,
                    });
                }
            }
            other => {
                state.tool_log.push(ToolLogItem {
                    title: other.plain_text(),
                    detail: other.plain_text(),
                    expanded: false,
                });
            }
        }
    }
}

fn drain_done(state: &mut TuiState, done_rx: &Receiver<WorkerDone>) {
    while let Ok(done) = done_rx.try_recv() {
        if state.worker.is_none() && !state.running {
            continue;
        }
        state.worker = None;
        state.runtime = Some(done.runtime);
        sync_active_session_ref(state);
        state.running = false;
        state.result_scroll = 0;
        match done.result {
            Ok(output) => {
                state.last_event = format_action_event("action ok", &output);
                state.chat.push(ChatLine {
                    role: "deepcli".to_string(),
                    content: output,
                });
            }
            Err(error) => {
                state.last_event = format_action_event("action failed", &error);
                state.chat.push(ChatLine {
                    role: "error".to_string(),
                    content: error,
                });
            }
        }
    }
}

fn toggle_selected_tool(state: &mut TuiState) {
    if state.monitor_tab != MonitorTab::Tools {
        return;
    }
    if let Some(index) = state.selected_tool {
        if let Some(item) = state.tool_log.get_mut(index) {
            item.expanded = !item.expanded;
            let state_label = if item.expanded {
                "expanded"
            } else {
                "collapsed"
            };
            state.last_event = format!("{state_label}: {}", item.title);
        }
    }
}

fn toggle_tool_at_row(state: &mut TuiState, tools_area: Rect, row: u16) {
    if state.monitor_tab != MonitorTab::Tools {
        return;
    }
    if row <= tools_area.y + 1 || row >= tools_area.y + tools_area.height.saturating_sub(1) {
        return;
    }
    let line = row.saturating_sub(tools_area.y + 1) as usize;
    let Some(index) = visible_tool_index_at_line(state, tools_area.height, line) else {
        return;
    };
    if let Some(item) = state.tool_log.get_mut(index) {
        item.expanded = !item.expanded;
        state.selected_tool = Some(index);
        let state_label = if item.expanded {
            "expanded"
        } else {
            "collapsed"
        };
        state.last_event = format!("{state_label}: {}", item.title);
    }
}

fn move_selected_tool_by(state: &mut TuiState, forward: bool, step: usize) {
    if state.tool_log.is_empty() {
        state.selected_tool = None;
        state.last_event = "no tool calls yet".to_string();
        return;
    }
    let current = state.selected_tool.unwrap_or(if forward {
        0
    } else {
        state.tool_log.len().saturating_sub(1)
    });
    let next = if forward {
        current
            .saturating_add(step.max(1))
            .min(state.tool_log.len().saturating_sub(1))
    } else {
        current.saturating_sub(step.max(1))
    };
    select_tool_at_index(state, next);
}

fn select_tool_at_index(state: &mut TuiState, index: usize) {
    if state.tool_log.is_empty() {
        state.selected_tool = None;
        state.last_event = "no tool calls yet".to_string();
        return;
    }
    let index = index.min(state.tool_log.len().saturating_sub(1));
    state.selected_tool = Some(index);
    if let Some(item) = state.tool_log.get(index) {
        state.last_event = format!("tool selected: {}", item.title);
    }
}

fn visible_tool_index_at_line(state: &TuiState, height: u16, line: usize) -> Option<usize> {
    let tool_lines = tool_tab_lines(state);
    let total_lines = tool_lines.len().saturating_add(1);
    let focus_line = selected_tool_panel_line(state);
    let original_line = visible_panel_line_indices(total_lines, height, focus_line)
        .get(line)
        .and_then(|line| *line)?;
    if original_line == 0 {
        return None;
    }
    tool_lines
        .get(original_line.saturating_sub(1))
        .and_then(|line| line.tool_index)
}

fn selected_tool_panel_line(state: &TuiState) -> Option<usize> {
    let selected = state.selected_tool?;
    if state
        .tool_log
        .get(selected)
        .is_some_and(|item| item.expanded)
    {
        return tool_tab_lines(state)
            .iter()
            .position(|line| line.text.starts_with("selected detail: "))
            .map(|line| line + 1);
    }
    tool_tab_lines(state)
        .iter()
        .position(|line| line.tool_index == Some(selected))
        .map(|line| line + 1)
}

fn visible_panel_line_indices(
    total_lines: usize,
    height: u16,
    focus_line: Option<usize>,
) -> Vec<Option<usize>> {
    let visible = height.saturating_sub(2) as usize;
    if visible == 0 || total_lines <= visible {
        return (0..total_lines).map(Some).collect();
    }
    let Some(focus_line) = focus_line.filter(|line| *line < total_lines) else {
        return (0..visible.saturating_sub(1))
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
    };
    if focus_line < visible.saturating_sub(1) {
        return (0..visible.saturating_sub(1))
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
    }
    if visible <= 2 {
        return (0..visible.saturating_sub(1))
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
    }

    let body_slots = visible.saturating_sub(2);
    let start = focus_line
        .saturating_sub(body_slots.saturating_sub(1))
        .max(1);
    let end = start.saturating_add(body_slots).min(total_lines);
    let mut focused = Vec::with_capacity(visible);
    focused.push(Some(0));
    focused.extend((start..end).map(Some));
    focused.push(None);
    focused.truncate(visible);
    focused
}

struct ChatUiLayout {
    header: Rect,
    transcript: Rect,
    tools: Rect,
    input: Rect,
}

fn chat_ui_layout(area: Rect) -> ChatUiLayout {
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

fn format_transcript_text(chat: &[ChatLine], scroll: usize, visible: usize) -> String {
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

fn render_chat_ui(frame: &mut Frame<'_>, state: &TuiState) {
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

fn credential_prompt_hidden_body(prompt: &CredentialPrompt) -> String {
    "*".repeat(prompt.input.buffer().chars().count())
}

fn credential_prompt_hidden_cursor(prompt: &CredentialPrompt) -> usize {
    prompt.input.buffer()[..prompt.input.cursor()]
        .chars()
        .count()
}

fn message_box_cursor_position(buffer: &str, cursor: usize, area: Rect) -> Position {
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

fn render_task_monitor(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let monitor = session_monitor_for_state(state);
    let text = format_task_monitor_text(state, monitor.as_ref(), area.height);
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Task Monitor (Ctrl-T/click tabs; Up/Down actions; Enter/click run)"),
        ),
        area,
    );
}

fn format_task_monitor_text(
    state: &TuiState,
    monitor: Option<&SessionMonitor>,
    height: u16,
) -> String {
    let quick_actions = monitor_quick_actions_for_tab(state, monitor);
    let mut lines = vec![format_monitor_tabs(state.monitor_tab)];
    lines.extend(match state.monitor_tab {
        MonitorTab::Overview => format_task_overview_lines(
            state,
            monitor.map(|monitor| &monitor.observation),
            &quick_actions,
            state.selected_command,
        ),
        MonitorTab::Result => {
            format_result_tab_lines(state, &quick_actions, state.selected_command, height)
        }
        MonitorTab::Changes => {
            format_changes_tab_lines(state, &quick_actions, state.selected_command)
        }
        MonitorTab::Usage => {
            format_usage_tab_lines(monitor, &quick_actions, state.selected_command)
        }
        MonitorTab::Health => {
            format_health_tab_lines(state, &quick_actions, state.selected_command)
        }
        MonitorTab::Library => {
            format_library_tab_lines(state, &quick_actions, state.selected_command)
        }
        MonitorTab::Deliver => {
            format_deliver_tab_lines(monitor, &quick_actions, state.selected_command)
        }
        MonitorTab::Tools => format_tool_tab_lines(state),
        MonitorTab::Tests => {
            format_tests_tab_lines(monitor, &quick_actions, state.selected_command)
        }
        MonitorTab::Environment => {
            format_environment_tab_lines(monitor, &quick_actions, state.selected_command)
        }
        MonitorTab::Approvals => format_approvals_tab_lines(monitor, state.selected_approval),
        MonitorTab::Trace => {
            format_trace_tab_lines(state, monitor, &quick_actions, state.selected_command)
        }
    });
    let selected_action_line = if state.monitor_tab == MonitorTab::Tools {
        selected_tool_panel_line(state)
    } else if state.selected_command == 0 {
        None
    } else {
        selected_monitor_quick_action_line(&lines)
    };
    truncate_panel_lines_with_focus(lines, height, selected_action_line)
}

fn format_monitor_tabs(active: MonitorTab) -> String {
    MonitorTab::all()
        .iter()
        .map(|tab| {
            if *tab == active {
                format!("[{}]", tab.label())
            } else {
                tab.label().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_task_overview_lines(
    state: &TuiState,
    observation: Option<&SessionObservation>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let ui_state = if state.running { "running" } else { "ready" };
    let last = compact_ui_text(&state.last_event, 80);
    let mut lines = if let Some(observation) = observation {
        let plan = if observation.plan_total == 0 {
            "plan=none".to_string()
        } else {
            let mut plan = format!(
                "plan={}/{}",
                observation.plan_completed, observation.plan_total
            );
            if observation.plan_in_progress > 0 {
                plan.push_str(&format!(" running={}", observation.plan_in_progress));
            }
            if observation.plan_failed > 0 {
                plan.push_str(&format!(" failed={}", observation.plan_failed));
            }
            plan
        };
        let test = observation
            .latest_test
            .as_ref()
            .map(format_latest_test)
            .unwrap_or_else(|| "test=none".to_string());
        let current = observation
            .current_step
            .as_deref()
            .map(|step| format!(" current={}", compact_ui_text(step, 48)))
            .unwrap_or_default();
        vec![
            format!(
                "state={} ui={} {plan} approvals={} btw={}{}",
                observation.state,
                ui_state,
                observation.pending_approvals,
                observation.open_questions,
                current
            ),
            format!(
                "{} tools={} failed_tools={} last={}",
                test,
                observation.tool_calls.max(state.tool_log.len()),
                observation.failed_tools,
                last
            ),
        ]
    } else {
        vec![
            format!("state={ui_state} plan=unknown approvals=unknown btw=unknown"),
            format!("test=unknown tools={} last={last}", state.tool_log.len()),
        ]
    };
    if let Some(result) = latest_action_result_line(state) {
        lines.push(result);
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn format_tool_tab_lines(state: &TuiState) -> Vec<String> {
    tool_tab_lines(state)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

fn tool_tab_lines(state: &TuiState) -> Vec<ToolTabLine> {
    if state.tool_log.is_empty() {
        return vec![ToolTabLine {
            text: "no tool calls yet".to_string(),
            tool_index: None,
        }];
    }
    let mut lines = Vec::new();
    if let Some((index, item)) = selected_tool_item(state).filter(|(_, item)| item.expanded) {
        lines.push(ToolTabLine {
            text: format!("selected detail: {}", item.title),
            tool_index: None,
        });
        for line in tool_detail_preview_lines(&item.detail) {
            lines.push(ToolTabLine {
                text: format!("  {line}"),
                tool_index: None,
            });
        }
        if tool_detail_is_truncated(&item.detail) {
            lines.push(ToolTabLine {
                text: "  [detail truncated; Ctrl-O prefill full output, Ctrl-F failed tools]"
                    .to_string(),
                tool_index: None,
            });
        }
        lines.push(ToolTabLine {
            text: format!(
                "tool calls: selected {}/{}",
                index + 1,
                state.tool_log.len()
            ),
            tool_index: None,
        });
    } else {
        lines.push(ToolTabLine {
            text: "tool calls: Up/Down select, Enter/click expand, Ctrl-O full".to_string(),
            tool_index: None,
        });
    }
    lines.extend(state.tool_log.iter().enumerate().map(|(index, item)| {
        let marker = if item.expanded { "v" } else { ">" };
        let selected = if state.selected_tool == Some(index) {
            "*"
        } else {
            " "
        };
        ToolTabLine {
            text: format!("{selected} {marker} {}", item.title),
            tool_index: Some(index),
        }
    }));
    lines
}

fn selected_tool_item(state: &TuiState) -> Option<(usize, &ToolLogItem)> {
    let index = state.selected_tool?;
    state.tool_log.get(index).map(|item| (index, item))
}

fn tool_detail_preview_lines(detail: &str) -> Vec<String> {
    let normalized = normalize_pasted_text(detail);
    let preview = normalized
        .chars()
        .take(TOOL_DETAIL_PREVIEW_CHARS)
        .collect::<String>();
    let mut lines = preview
        .lines()
        .take(TOOL_DETAIL_PREVIEW_LINES)
        .map(|line| compact_ui_text(line, TOOL_DETAIL_PREVIEW_LINE_CHARS))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("<empty detail>".to_string());
    }
    lines
}

fn tool_detail_is_truncated(detail: &str) -> bool {
    let normalized = normalize_pasted_text(detail);
    normalized.chars().count() > TOOL_DETAIL_PREVIEW_CHARS
        || normalized.lines().count() > TOOL_DETAIL_PREVIEW_LINES
        || normalized
            .lines()
            .any(|line| line.chars().count() > TOOL_DETAIL_PREVIEW_LINE_CHARS)
}

fn format_result_tab_lines(
    state: &TuiState,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
    height: u16,
) -> Vec<String> {
    let mut lines = Vec::new();
    match latest_action_result(state) {
        Some(result) => {
            lines.push(format!("status: {}", result.status));
            lines.push(format!("summary: {}", compact_ui_text(result.summary, 100)));
            let output_lines = non_empty_output_lines(result.content);
            let window_size = result_output_window_size(height, quick_actions);
            let scroll = state
                .result_scroll
                .min(output_lines.len().saturating_sub(1));
            let end = output_lines.len().saturating_sub(scroll).max(1);
            let start = end.saturating_sub(window_size);
            let below = output_lines.len().saturating_sub(end);
            if start == 0 && below == 0 {
                lines.push("output:".to_string());
            } else {
                lines.push(format!(
                    "output (PageUp older; PageDown latest; above={start} below={below}):"
                ));
            }
            let mut emitted = false;
            for line in output_lines.into_iter().skip(start).take(end - start) {
                lines.push(format!("  {}", compact_ui_text(line, 118)));
                emitted = true;
            }
            if !emitted {
                lines.push("  <empty>".to_string());
            }
        }
        None => {
            lines.push("no command output yet".to_string());
            lines.push("run a quick action or slash command to populate this view".to_string());
        }
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn result_output_window_size(height: u16, quick_actions: &[MonitorQuickAction]) -> usize {
    let visible = height.saturating_sub(2) as usize;
    let tab_lines = 1;
    let result_fixed_lines = 3;
    let quick_action_lines = if quick_actions.is_empty() {
        0
    } else {
        1 + quick_actions.len()
    };
    visible
        .saturating_sub(tab_lines + result_fixed_lines + quick_action_lines)
        .max(1)
}

struct ChangesPanelState {
    session_id: String,
    total_diff_records: usize,
    recent: Vec<SessionDiffRecord>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct DiffPanelStats {
    insertions: usize,
    deletions: usize,
    paths: Vec<String>,
}

fn refresh_workspace_changes_snapshot(state: &mut TuiState) {
    if state.monitor_tab != MonitorTab::Changes {
        return;
    }
    let now = Instant::now();
    if state
        .workspace_changes_checked_at
        .is_some_and(|checked_at| {
            now.duration_since(checked_at) < WORKTREE_CHANGES_REFRESH_INTERVAL
        })
    {
        return;
    }
    state.workspace_changes_checked_at = Some(now);
    state.workspace_changes = active_workspace_for_state(state)
        .as_deref()
        .map(load_workspace_changes_snapshot);
    clamp_selected_change_patch(state);
}

fn format_changes_tab_lines(
    state: &TuiState,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    append_workspace_changes_lines(
        &mut lines,
        state.workspace_changes.as_ref(),
        state.selected_change,
        state.change_patch_scroll,
    );
    match load_changes_panel_state(state) {
        Ok(Some(panel)) => {
            lines.push(format!(
                "session: {} diff_records={} showing={}",
                short_id(&panel.session_id),
                panel.total_diff_records,
                panel.recent.len()
            ));
            if panel.total_diff_records == 0 {
                lines.push("no session diff records yet".to_string());
                lines.push("run /diff --stat to inspect current Git worktree".to_string());
            } else {
                let aggregate = summarize_diff_records(&panel.recent);
                lines.push(format!(
                    "recent summary: files={} +{} -{}",
                    aggregate.paths.len(),
                    aggregate.insertions,
                    aggregate.deletions
                ));
                if !aggregate.paths.is_empty() {
                    lines.push("files:".to_string());
                    for path in aggregate.paths.iter().take(4) {
                        lines.push(format!("  - {}", compact_ui_text(path, 96)));
                    }
                    if aggregate.paths.len() > 4 {
                        lines.push(format!("  ... {} more", aggregate.paths.len() - 4));
                    }
                }
                lines.push("recent diffs:".to_string());
                for record in panel.recent.iter().rev().take(4) {
                    let stats = summarize_diff_content(&record.content);
                    lines.push(format!(
                        "  * {} {} +{} -{}",
                        record.modified_at.format("%H:%M:%S"),
                        compact_diff_record_name(record),
                        stats.insertions,
                        stats.deletions
                    ));
                }
            }
        }
        Ok(None) => {
            lines.push("changes unavailable: no active session".to_string());
            lines.push("run /diff --stat to inspect current Git worktree".to_string());
        }
        Err(error) => {
            lines.push(format!(
                "changes unavailable: {}",
                compact_ui_text(&error.to_string(), 100)
            ));
            lines.push("run /session diffs --limit 5 for a detailed error".to_string());
        }
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn append_workspace_changes_lines(
    lines: &mut Vec<String>,
    snapshot: Option<&WorkspaceChangesSnapshot>,
    selected_change: usize,
    patch_scroll: usize,
) {
    match snapshot {
        Some(snapshot) if snapshot.available && snapshot.changed == 0 => {
            lines.push("worktree: clean".to_string());
        }
        Some(snapshot) if snapshot.available => {
            lines.push(format!(
                "worktree: dirty changed={} staged={} unstaged={} untracked={}",
                snapshot.changed, snapshot.staged, snapshot.unstaged, snapshot.untracked
            ));
            if !snapshot.paths.is_empty() {
                lines.push("worktree files:".to_string());
                for path in snapshot.paths.iter().take(4) {
                    lines.push(format!("  - {}", compact_ui_text(path, 96)));
                }
                if snapshot.paths.len() > 4 {
                    lines.push(format!("  ... {} more", snapshot.paths.len() - 4));
                }
            }
            append_worktree_patch_preview_lines(lines, snapshot, selected_change, patch_scroll);
        }
        Some(snapshot) => {
            lines.push(format!(
                "worktree: unavailable{}",
                snapshot
                    .detail
                    .as_ref()
                    .map(|detail| format!(" ({})", compact_ui_text(detail, 90)))
                    .unwrap_or_default()
            ));
        }
        None => {
            lines.push("worktree: not checked yet".to_string());
        }
    }
}

fn append_worktree_patch_preview_lines(
    lines: &mut Vec<String>,
    snapshot: &WorkspaceChangesSnapshot,
    selected_change: usize,
    patch_scroll: usize,
) {
    if !snapshot.diff_sections.is_empty() {
        append_selected_worktree_patch_lines(lines, snapshot, selected_change, patch_scroll);
        return;
    }
    if snapshot.diff_preview.is_empty() {
        if snapshot.untracked > 0 && snapshot.changed == snapshot.untracked {
            lines.push("worktree patch: none (untracked files only)".to_string());
        } else {
            lines.push("worktree patch: none".to_string());
        }
        return;
    }
    lines.push(format!(
        "worktree patch preview{}:",
        if snapshot.diff_preview_truncated {
            " (truncated)"
        } else {
            ""
        }
    ));
    for line in &snapshot.diff_preview {
        lines.push(format!("  {}", compact_ui_text(line, 118)));
    }
}

fn append_selected_worktree_patch_lines(
    lines: &mut Vec<String>,
    snapshot: &WorkspaceChangesSnapshot,
    selected_change: usize,
    patch_scroll: usize,
) {
    let selected = selected_change.min(snapshot.diff_sections.len() - 1);
    let section = &snapshot.diff_sections[selected];
    let max_scroll = section.lines.len().saturating_sub(1);
    let start = patch_scroll.min(max_scroll);
    let end = (start + CHANGE_PATCH_WINDOW_LINES).min(section.lines.len());
    let below = section.lines.len().saturating_sub(end);
    lines.push(format!(
        "selected patch: {}/{} {} {}{}",
        selected + 1,
        snapshot.diff_sections.len(),
        section.label,
        compact_ui_text(&section.path, 72),
        if section.truncated {
            " (truncated)"
        } else {
            ""
        }
    ));
    if start > 0 {
        lines.push(format!("  [above: {start} line(s)]"));
    }
    for line in section.lines.iter().skip(start).take(end - start) {
        lines.push(format!("  {}", compact_ui_text(line, 118)));
    }
    if below > 0 {
        lines.push(format!("  [below: {below} line(s)]"));
    }
}

fn load_changes_panel_state(state: &TuiState) -> Result<Option<ChangesPanelState>> {
    let Some(active) = active_session_ref_for_state(state) else {
        return Ok(None);
    };
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let total_diff_records = session.activity_summary()?.diff_count;
    let recent = session.load_recent_diffs(5)?;
    Ok(Some(ChangesPanelState {
        session_id: active.session_id,
        total_diff_records,
        recent,
    }))
}

fn active_session_ref_for_state(state: &TuiState) -> Option<ActiveSessionRef> {
    state
        .runtime
        .as_ref()
        .map(active_session_ref)
        .or_else(|| state.active_session.clone())
}

fn active_workspace_for_state(state: &TuiState) -> Option<PathBuf> {
    state
        .runtime
        .as_ref()
        .map(|runtime| runtime.workspace().to_path_buf())
        .or_else(|| {
            state
                .active_session
                .as_ref()
                .map(|active| active.workspace.clone())
        })
}

fn load_workspace_changes_snapshot(workspace: &Path) -> WorkspaceChangesSnapshot {
    match Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=normal"])
        .current_dir(workspace)
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut snapshot = parse_git_status_snapshot(&stdout);
            if snapshot.changed > snapshot.untracked {
                let (preview, truncated, sections) = load_git_diff_preview(workspace);
                snapshot.diff_preview = preview;
                snapshot.diff_preview_truncated = truncated;
                snapshot.diff_sections = sections;
            }
            snapshot
        }
        Ok(output) => WorkspaceChangesSnapshot {
            available: false,
            detail: Some(compact_ui_text(
                &String::from_utf8_lossy(&output.stderr),
                120,
            )),
            changed: 0,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            paths: Vec::new(),
            diff_preview: Vec::new(),
            diff_preview_truncated: false,
            diff_sections: Vec::new(),
        },
        Err(error) => WorkspaceChangesSnapshot {
            available: false,
            detail: Some(error.to_string()),
            changed: 0,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            paths: Vec::new(),
            diff_preview: Vec::new(),
            diff_preview_truncated: false,
            diff_sections: Vec::new(),
        },
    }
}

fn load_git_diff_preview(workspace: &Path) -> (Vec<String>, bool, Vec<WorkspaceDiffSection>) {
    let mut lines = Vec::new();
    let mut truncated = false;
    let mut sections = Vec::new();
    append_git_diff_preview_section(
        workspace,
        "unstaged",
        &["diff", "--no-ext-diff", "--unified=3", "--"],
        &mut lines,
        &mut truncated,
        &mut sections,
    );
    append_git_diff_preview_section(
        workspace,
        "staged",
        &["diff", "--cached", "--no-ext-diff", "--unified=3", "--"],
        &mut lines,
        &mut truncated,
        &mut sections,
    );
    (lines, truncated, sections)
}

fn append_git_diff_preview_section(
    workspace: &Path,
    label: &str,
    args: &[&str],
    lines: &mut Vec<String>,
    truncated: &mut bool,
    sections: &mut Vec<WorkspaceDiffSection>,
) {
    if *truncated {
        return;
    }
    let output = match Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            push_limited_preview_line(
                lines,
                truncated,
                format!("{label} diff unavailable: {error}"),
            );
            return;
        }
    };
    if !output.status.success() {
        let detail = compact_ui_text(&String::from_utf8_lossy(&output.stderr), 100);
        push_limited_preview_line(
            lines,
            truncated,
            format!("{label} diff unavailable: {detail}"),
        );
        return;
    }
    let content = String::from_utf8_lossy(&output.stdout);
    if content.trim().is_empty() {
        return;
    }
    sections.extend(parse_diff_sections(label, &content));
    push_limited_preview_line(lines, truncated, format!("{label} diff:"));
    for line in content.lines() {
        push_limited_preview_line(lines, truncated, line.to_string());
        if *truncated {
            return;
        }
    }
}

fn push_limited_preview_line(lines: &mut Vec<String>, truncated: &mut bool, line: String) {
    if lines.len() >= WORKTREE_DIFF_PREVIEW_LINES {
        *truncated = true;
        return;
    }
    lines.push(line);
}

fn parse_diff_sections(label: &str, content: &str) -> Vec<WorkspaceDiffSection> {
    let mut sections = Vec::new();
    let mut current: Option<WorkspaceDiffSection> = None;
    for line in content.lines() {
        if line.starts_with("diff --git ") {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some(WorkspaceDiffSection {
                label: label.to_string(),
                path: diff_header_display_path(line).unwrap_or_else(|| "<unknown>".to_string()),
                lines: vec![line.to_string()],
                truncated: false,
            });
            continue;
        }
        if let Some(section) = current.as_mut() {
            push_limited_diff_section_line(section, line);
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }
    sections
}

fn push_limited_diff_section_line(section: &mut WorkspaceDiffSection, line: &str) {
    if section.lines.len() >= WORKTREE_DIFF_SECTION_LINES {
        section.truncated = true;
        return;
    }
    section.lines.push(line.to_string());
}

fn diff_header_display_path(line: &str) -> Option<String> {
    line.split_whitespace()
        .find_map(|part| part.strip_prefix("b/"))
        .map(|path| path.trim_matches('"').to_string())
        .filter(|path| !path.is_empty() && path != "/dev/null")
}

fn parse_git_status_snapshot(status: &str) -> WorkspaceChangesSnapshot {
    let mut snapshot = WorkspaceChangesSnapshot {
        available: true,
        detail: None,
        changed: 0,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        paths: Vec::new(),
        diff_preview: Vec::new(),
        diff_preview_truncated: false,
        diff_sections: Vec::new(),
    };
    for line in status.lines().filter(|line| !line.trim().is_empty()) {
        snapshot.changed += 1;
        let bytes = line.as_bytes();
        let index_status = bytes.first().copied().unwrap_or(b' ') as char;
        let worktree_status = bytes.get(1).copied().unwrap_or(b' ') as char;
        if line.starts_with("?? ") {
            snapshot.untracked += 1;
        } else {
            if index_status != ' ' {
                snapshot.staged += 1;
            }
            if worktree_status != ' ' {
                snapshot.unstaged += 1;
            }
        }
        if let Some(path) = git_status_display_path(line) {
            push_unique_diff_path(&mut snapshot.paths, path);
        }
    }
    snapshot
}

fn git_status_display_path(line: &str) -> Option<String> {
    let path = line.get(3..)?.trim();
    if path.is_empty() {
        return None;
    }
    let display = path
        .rsplit(" -> ")
        .next()
        .unwrap_or(path)
        .trim_matches('"')
        .to_string();
    if display.is_empty() {
        None
    } else {
        Some(display)
    }
}

fn summarize_diff_records(records: &[SessionDiffRecord]) -> DiffPanelStats {
    let mut aggregate = DiffPanelStats::default();
    for record in records {
        let stats = summarize_diff_content(&record.content);
        aggregate.insertions += stats.insertions;
        aggregate.deletions += stats.deletions;
        for path in stats.paths {
            push_unique_diff_path(&mut aggregate.paths, path);
        }
    }
    aggregate
}

fn summarize_diff_content(content: &str) -> DiffPanelStats {
    let mut stats = DiffPanelStats::default();
    for line in content.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            push_unique_diff_path(&mut stats.paths, path.to_string());
            continue;
        }
        if line.starts_with("+++") {
            continue;
        }
        if let Some(path) = line.strip_prefix("--- a/") {
            push_unique_diff_path(&mut stats.paths, path.to_string());
            continue;
        }
        if line.starts_with("---") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            for part in rest.split_whitespace() {
                if let Some(path) = part.strip_prefix("b/") {
                    push_unique_diff_path(&mut stats.paths, path.to_string());
                }
            }
            continue;
        }
        if line.starts_with('+') {
            stats.insertions += 1;
        } else if line.starts_with('-') {
            stats.deletions += 1;
        }
    }
    stats
        .paths
        .retain(|path| path != "/dev/null" && !path.is_empty());
    stats
}

fn push_unique_diff_path(paths: &mut Vec<String>, path: String) {
    if path == "/dev/null" || path.is_empty() || paths.iter().any(|existing| existing == &path) {
        return;
    }
    paths.push(path);
}

fn compact_diff_record_name(record: &SessionDiffRecord) -> String {
    let display = record
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&record.name);
    compact_ui_text(display, 70)
}

fn monitor_quick_actions_for_tab(
    state: &TuiState,
    monitor: Option<&SessionMonitor>,
) -> Vec<MonitorQuickAction> {
    match state.monitor_tab {
        MonitorTab::Overview => vec![
            MonitorQuickAction::run("/status --json"),
            MonitorQuickAction::run("/next --json"),
            MonitorQuickAction::run("/trace --limit 30"),
        ],
        MonitorTab::Result => vec![
            MonitorQuickAction::run("/trace --limit 30"),
            MonitorQuickAction::run("/status --json"),
            MonitorQuickAction::run("/session history --limit 5"),
        ],
        MonitorTab::Changes => vec![
            MonitorQuickAction::run("/diff --stat"),
            MonitorQuickAction::run("/diff --name-only"),
            MonitorQuickAction::run("/review"),
            MonitorQuickAction::run("/handoff --format pr"),
        ],
        MonitorTab::Usage => vec![
            MonitorQuickAction::run("/usage --json"),
            MonitorQuickAction::run("/trace --limit 30"),
            MonitorQuickAction::run("/logs --limit 80"),
            MonitorQuickAction::run("/status --json"),
        ],
        MonitorTab::Health => health_quick_actions_for_state(state),
        MonitorTab::Library => library_quick_actions_for_state(state),
        MonitorTab::Deliver => deliver_quick_actions(monitor),
        MonitorTab::Tools => Vec::new(),
        MonitorTab::Tests => vec![
            MonitorQuickAction::run("/test discover --json"),
            MonitorQuickAction::run("/test run --json"),
            MonitorQuickAction::run("/accept --json"),
            MonitorQuickAction::run("/gate --json"),
        ],
        MonitorTab::Environment => environment_quick_actions(monitor),
        MonitorTab::Approvals => Vec::new(),
        MonitorTab::Trace => vec![
            MonitorQuickAction::run("/trace --limit 30"),
            MonitorQuickAction::run("/logs --limit 80"),
            MonitorQuickAction::run("/usage --json"),
            MonitorQuickAction::run("/session diagnose --json"),
        ],
    }
}

fn health_quick_actions_for_state(state: &TuiState) -> Vec<MonitorQuickAction> {
    let Some(workspace) = workspace_for_state(state) else {
        return vec![
            MonitorQuickAction::run("/doctor --quick"),
            MonitorQuickAction::run("/config validate --json"),
        ];
    };
    let Ok(config) = AppConfig::load_effective(workspace, None) else {
        return vec![
            MonitorQuickAction::run("/config validate --json"),
            MonitorQuickAction::run("/doctor --quick"),
        ];
    };
    let header = header_status_for_state(state);
    let provider_name = if header.provider.starts_with('<') {
        config.default_provider.clone()
    } else {
        header.provider
    };
    let mut actions = vec![MonitorQuickAction::run("/model show --json")];
    if provider_needs_credentials_for_ui(workspace, &config, &provider_name) {
        actions.push(MonitorQuickAction::run(format!(
            "/credentials set {provider_name}"
        )));
    }
    actions.extend([
        MonitorQuickAction::run(format!("/credentials status {provider_name} --json")),
        MonitorQuickAction::run("/config validate --json"),
        MonitorQuickAction::run("/selftest --json"),
        MonitorQuickAction::run("/doctor --quick"),
    ]);
    actions
}

fn provider_needs_credentials_for_ui(
    workspace: &Path,
    config: &AppConfig,
    provider_name: &str,
) -> bool {
    if config.provider(Some(provider_name)).is_err() {
        return false;
    }
    match config.provider_runtime(workspace, Some(provider_name)) {
        Ok(runtime) => runtime.api_key.is_none(),
        Err(_) => true,
    }
}

fn environment_quick_actions(monitor: Option<&SessionMonitor>) -> Vec<MonitorQuickAction> {
    let target = environment_action_target(monitor);
    let mut actions = vec![
        MonitorQuickAction::run(format!("/env check {target} --json")),
        MonitorQuickAction::run(format!("/env plan {target} --smoke --json")),
    ];
    if environment_needs_setup(monitor) {
        actions.push(MonitorQuickAction::edit(format!(
            "/env setup {target} --smoke"
        )));
    }
    actions.extend([
        MonitorQuickAction::run(format!("/env test {target} --json")),
        MonitorQuickAction::run(format!("/accept --env-check {target} --json")),
        MonitorQuickAction::run(format!("/gate --env-check {target} --json")),
        MonitorQuickAction::run(format!("/handoff --env-check {target} --format pr")),
    ]);
    actions
}

fn deliver_quick_actions(monitor: Option<&SessionMonitor>) -> Vec<MonitorQuickAction> {
    if monitor.is_some() {
        let target = environment_action_target(monitor);
        vec![
            MonitorQuickAction::run("/review"),
            MonitorQuickAction::run("/test run --json"),
            MonitorQuickAction::run(format!("/accept --env-check {target} --json")),
            MonitorQuickAction::run(format!("/gate --env-check {target} --json")),
            MonitorQuickAction::run(format!("/handoff --env-check {target} --format pr")),
        ]
    } else {
        vec![
            MonitorQuickAction::run("/test discover --json"),
            MonitorQuickAction::run("/accept --json"),
            MonitorQuickAction::run("/gate --json"),
            MonitorQuickAction::run("/handoff --format pr"),
        ]
    }
}

fn environment_action_target(monitor: Option<&SessionMonitor>) -> String {
    monitor
        .and_then(|monitor| monitor.recent_environment.last())
        .map(|environment| environment.target.as_str())
        .filter(|target| matches!(*target, "docker" | "compiler"))
        .unwrap_or("docker")
        .to_string()
}

fn environment_needs_setup(monitor: Option<&SessionMonitor>) -> bool {
    monitor
        .and_then(|monitor| monitor.recent_environment.last())
        .map(|environment| {
            environment.ready == Some(false)
                || environment.status.contains("needs")
                || environment.status.contains("missing")
                || environment.detail.contains("/env setup")
        })
        .unwrap_or(true)
}

fn library_quick_actions_for_state(state: &TuiState) -> Vec<MonitorQuickAction> {
    if workspace_for_state(state).is_none() {
        return vec![
            MonitorQuickAction::run("/prompt list --json"),
            MonitorQuickAction::run("/skill list --json"),
            MonitorQuickAction::run("/agent list --json"),
        ];
    }
    vec![
        MonitorQuickAction::run("/prompt list --json"),
        MonitorQuickAction::edit("/prompt render <name> --file path"),
        MonitorQuickAction::run("/skill list --json"),
        MonitorQuickAction::run("/agent list --json"),
    ]
}

fn append_monitor_quick_actions(
    lines: &mut Vec<String>,
    label: &str,
    actions: &[MonitorQuickAction],
    selected: usize,
) {
    if actions.is_empty() {
        return;
    }
    lines.push(format!("{label} (Up/Down select, Enter run):"));
    let selected = selected.min(actions.len() - 1);
    for (index, action) in actions.iter().enumerate() {
        let marker = if index == selected { ">" } else { " " };
        let suffix = if action.edit_before_run {
            " (edit)"
        } else {
            ""
        };
        lines.push(format!(" {marker} {}{suffix}", action.command));
    }
}

fn format_usage_tab_lines(
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(monitor) = monitor else {
        let mut lines = vec!["usage unavailable for running handoff".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let usage = &monitor.usage;
    let mut lines = Vec::new();
    if usage.provider_turns_started == 0
        && usage.provider_turns_completed == 0
        && usage.total_tokens.is_none()
        && usage.max_request_bytes.is_none()
    {
        lines.push("no provider usage recorded yet".to_string());
    } else {
        lines.push(format!(
            "provider turns: started={} completed={} avg={} max={} tool_calls={}",
            usage.provider_turns_started,
            usage.provider_turns_completed,
            format_optional_ms(usage.provider_average_elapsed_ms),
            format_optional_ms(usage.provider_max_elapsed_ms),
            usage.provider_tool_calls
        ));
        lines.push(format!(
            "tokens: prompt={} completion={} total={}",
            format_optional_u64(usage.prompt_tokens),
            format_optional_u64(usage.completion_tokens),
            format_optional_u64(usage.total_tokens)
        ));
        lines.push(format!(
            "request: latest={} max={} compacted_turns={}",
            format_optional_bytes(usage.latest_request_bytes),
            format_optional_bytes(usage.max_request_bytes),
            usage.compacted_turns
        ));
        if usage.prompt_cache_hit_tokens.is_some() || usage.prompt_cache_miss_tokens.is_some() {
            lines.push(format!(
                "context cache: hit={} miss={} hit_rate={}",
                format_optional_u64(usage.prompt_cache_hit_tokens),
                format_optional_u64(usage.prompt_cache_miss_tokens),
                format_cache_hit_rate(usage)
            ));
        }
        if usage.provider_turns_started > usage.provider_turns_completed {
            lines.push(
                "warning: provider turn started but not completed; inspect /trace --limit 30"
                    .to_string(),
            );
        }
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn format_health_tab_lines(
    state: &TuiState,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(workspace) = workspace_for_state(state) else {
        let mut lines = vec!["health unavailable: no workspace".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let header = header_status_for_state(state);
    let config = match AppConfig::load_effective(workspace, None) {
        Ok(config) => config,
        Err(error) => {
            let mut lines = vec![format!(
                "config load failed: {}",
                compact_ui_text(&error.to_string(), 100)
            )];
            append_monitor_quick_actions(
                &mut lines,
                "quick actions",
                quick_actions,
                selected_quick_action,
            );
            return lines;
        }
    };
    let provider_name = if header.provider.starts_with('<') {
        config.default_provider.clone()
    } else {
        header.provider.clone()
    };
    let mut lines = vec![format!(
        "provider: active={} model={} default={}",
        provider_name,
        header.model,
        compact_ui_text(&config.default_provider, 40)
    )];
    match config.provider(Some(&provider_name)) {
        Ok((name, provider)) => {
            let credentials_path = absolutize_workspace_path(workspace, &provider.credentials_file);
            let env_key = provider_env_key_for_ui(name);
            let env_present = env::var_os(&env_key).is_some();
            match config.provider_runtime(workspace, Some(name)) {
                Ok(runtime) => {
                    let api_key = if runtime.api_key.is_some() {
                        "configured"
                    } else {
                        "missing"
                    };
                    let model = runtime
                        .model
                        .as_deref()
                        .or(provider.acceptance_model.as_deref())
                        .unwrap_or("<unset>");
                    lines.push(format!(
                        "credentials: api_key={} file={} env={}",
                        api_key,
                        presence_label(credentials_path.exists()),
                        presence_label(env_present)
                    ));
                    lines.push(format!(
                        "runtime: type={} model={} endpoint={}",
                        runtime.provider_type,
                        compact_ui_text(model, 40),
                        runtime.endpoint.as_deref().unwrap_or("<default>")
                    ));
                }
                Err(error) => {
                    lines.push(format!(
                        "credentials: file={} env={} error={}",
                        presence_label(credentials_path.exists()),
                        presence_label(env_present),
                        compact_ui_text(&error.to_string(), 70)
                    ));
                }
            }
        }
        Err(error) => lines.push(format!(
            "provider config error: {}",
            compact_ui_text(&error.to_string(), 90)
        )),
    }
    lines.push(format!(
        "config: project={} permissions={} timeout={}s max_iters={}",
        presence_label(workspace.join(".deepcli/config.json").exists()),
        config.permissions.default_mode,
        config.agent.provider_turn_timeout_seconds,
        config.agent.max_tool_iterations
    ));
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn format_library_tab_lines(
    state: &TuiState,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(workspace) = workspace_for_state(state) else {
        let mut lines = vec!["library unavailable: no workspace".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let mut lines = Vec::new();
    match PromptStore::new(workspace).list() {
        Ok(prompts) => {
            let custom_count = prompts
                .iter()
                .filter(|prompt| {
                    workspace
                        .join(".deepcli")
                        .join("prompts")
                        .join(format!("{}.md", prompt.name))
                        .exists()
                })
                .count();
            lines.push(format!(
                "prompts: total={} custom={} builtins={}",
                prompts.len(),
                custom_count,
                prompts.len().saturating_sub(custom_count)
            ));
            lines.extend(
                prompts
                    .iter()
                    .take(3)
                    .map(|prompt| format_library_item("prompt", &prompt.name, &prompt.description)),
            );
        }
        Err(error) => lines.push(format!(
            "prompts: error={}",
            compact_ui_text(&error.to_string(), 90)
        )),
    }
    match SkillStore::new(workspace).discover() {
        Ok(skills) if skills.is_empty() => {
            lines.push("skills: none registered".to_string());
        }
        Ok(skills) => {
            lines.push(format!("skills: total={}", skills.len()));
            lines.extend(
                skills
                    .iter()
                    .take(3)
                    .map(|skill| format_library_item("skill", &skill.name, &skill.description)),
            );
        }
        Err(error) => lines.push(format!(
            "skills: error={}",
            compact_ui_text(&error.to_string(), 90)
        )),
    }
    match AgentStore::new(workspace).list() {
        Ok(tasks) if tasks.is_empty() => {
            lines.push("agents: no sub-agent tasks".to_string());
        }
        Ok(tasks) => {
            lines.push(format!("agents: total={}", tasks.len()));
            lines.extend(tasks.iter().rev().take(3).map(|task| {
                format!(
                    "  agent {} status={:?} {}",
                    short_id(&task.id.to_string()),
                    task.status,
                    compact_ui_text(&task.task, 58)
                )
            }));
        }
        Err(error) => lines.push(format!(
            "agents: error={}",
            compact_ui_text(&error.to_string(), 90)
        )),
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn format_deliver_tab_lines(
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(monitor) = monitor else {
        let mut lines = vec!["delivery evidence unavailable for running handoff".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let observation = &monitor.observation;
    let mut lines = vec!["acceptance checklist:".to_string()];
    lines.push(format!("  plan: {}", delivery_plan_status(observation)));
    lines.push(format!("  tests: {}", delivery_test_status(monitor)));
    lines.push(format!(
        "  environment: {}",
        delivery_environment_status(monitor)
    ));
    lines.push(format!(
        "  blockers: approvals={} btw={} failed_tools={}",
        observation.pending_approvals, observation.open_questions, observation.failed_tools
    ));
    append_monitor_quick_actions(
        &mut lines,
        "recommended flow",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn format_tests_tab_lines(
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(monitor) = monitor else {
        let mut lines = vec!["tests unavailable for running handoff".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let mut lines = Vec::new();
    if monitor.recent_tests.is_empty() {
        lines.push("no test runs recorded".to_string());
    } else {
        lines.extend(monitor.recent_tests.iter().rev().map(format_latest_test));
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn format_environment_tab_lines(
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    match monitor {
        Some(monitor) if monitor.recent_environment.is_empty() => {
            lines.push("no environment evidence recorded".to_string());
        }
        Some(monitor) => {
            lines.push("recent environment evidence:".to_string());
            lines.extend(
                monitor
                    .recent_environment
                    .iter()
                    .rev()
                    .map(format_latest_environment),
            );
        }
        None => lines.push("environment evidence unavailable for running handoff".to_string()),
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn format_approvals_tab_lines(
    monitor: Option<&SessionMonitor>,
    selected_approval: usize,
) -> Vec<String> {
    let Some(monitor) = monitor else {
        return vec!["approvals unavailable for running handoff".to_string()];
    };
    let mut lines = Vec::new();
    if monitor.pending_approvals.is_empty() {
        lines.push("pending approvals: none".to_string());
    } else {
        lines.push(format!(
            "pending approvals: {} (Up/Down select, Enter approve, d deny)",
            monitor.pending_approvals.len()
        ));
        lines.extend(
            monitor
                .pending_approvals
                .iter()
                .enumerate()
                .map(|(index, approval)| {
                    format_pending_approval(index == selected_approval, approval)
                }),
        );
    }
    if monitor.open_questions.is_empty() {
        lines.push("open btw questions: none".to_string());
    } else {
        lines.push(format!(
            "open btw questions: {} (Enter opens answer box)",
            monitor.open_questions.len()
        ));
        let approval_count = monitor.pending_approvals.len();
        lines.extend(
            monitor
                .open_questions
                .iter()
                .enumerate()
                .map(|(index, question)| {
                    format_open_question(approval_count + index == selected_approval, question)
                }),
        );
    }
    lines
}

fn format_trace_tab_lines(
    state: &TuiState,
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let mut lines = vec![format!("last: {}", compact_ui_text(&state.last_event, 100))];
    if let Some(result) = latest_action_result_line(state) {
        lines.push(result);
    }
    if let Some(monitor) = monitor {
        if monitor.recent_events.is_empty() {
            lines.push("no audit events recorded".to_string());
        } else {
            lines.extend(
                monitor
                    .recent_events
                    .iter()
                    .rev()
                    .map(|event| format!("{} {}", event.created_at, event.event_type)),
            );
        }
    } else {
        lines.push("audit events unavailable for running handoff".to_string());
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn format_pending_approval(selected: bool, approval: &SessionObservationApproval) -> String {
    let marker = if selected { "*" } else { "-" };
    format!(
        "{marker} {} {} risk={} {}",
        short_id(&approval.id),
        approval.tool,
        approval.risk,
        compact_ui_text(&approval.reason, 70)
    )
}

fn format_open_question(selected: bool, question: &SessionObservationQuestion) -> String {
    let marker = if selected { "*" } else { "-" };
    format!(
        "{marker} {} {}",
        short_id(&question.id),
        compact_ui_text(&question.question, 82)
    )
}

fn truncate_panel_lines(mut lines: Vec<String>, height: u16) -> String {
    let visible = height.saturating_sub(2) as usize;
    if visible > 0 && lines.len() > visible {
        lines.truncate(visible.saturating_sub(1));
        lines.push(more_panel_lines_marker());
    }
    lines.join("\n")
}

fn truncate_panel_lines_with_focus(
    lines: Vec<String>,
    height: u16,
    focus_line: Option<usize>,
) -> String {
    let visible = height.saturating_sub(2) as usize;
    if visible == 0 || lines.len() <= visible {
        return lines.join("\n");
    }
    let Some(focus_line) = focus_line.filter(|line| *line < lines.len()) else {
        return truncate_panel_lines(lines, height);
    };
    if focus_line < visible.saturating_sub(1) {
        return truncate_panel_lines(lines, height);
    }
    if visible <= 2 {
        return lines
            .into_iter()
            .take(visible.saturating_sub(1))
            .chain(std::iter::once(more_panel_lines_marker()))
            .collect::<Vec<_>>()
            .join("\n");
    }

    let body_slots = visible.saturating_sub(2);
    let start = focus_line
        .saturating_sub(body_slots.saturating_sub(1))
        .max(1);
    let end = start.saturating_add(body_slots).min(lines.len());
    let mut focused = Vec::with_capacity(visible);
    focused.push(lines[0].clone());
    focused.extend(lines[start..end].iter().cloned());
    focused.push(more_panel_lines_marker());
    focused.truncate(visible);
    focused.join("\n")
}

fn selected_monitor_quick_action_line(lines: &[String]) -> Option<usize> {
    lines
        .iter()
        .position(|line| line.trim_start().starts_with("> /"))
}

fn more_panel_lines_marker() -> String {
    "[more: use /session, /approval, /btw, /env, or /trace for full detail]".to_string()
}

fn format_latest_test(test: &SessionObservationTest) -> String {
    let status = if test.passed { "pass" } else { "fail" };
    let code = test
        .exit_code
        .map(|code| format!(" code={code}"))
        .unwrap_or_default();
    format!(
        "test={}{} {}",
        status,
        code,
        compact_ui_text(&test.command, 42)
    )
}

fn workspace_for_state(state: &TuiState) -> Option<&Path> {
    state
        .runtime
        .as_ref()
        .map(AgentRuntime::workspace)
        .or_else(|| {
            state
                .active_session
                .as_ref()
                .map(|active| active.workspace.as_path())
        })
}

fn provider_env_key_for_ui(provider: &str) -> String {
    format!(
        "{}_API_KEY",
        provider.to_ascii_uppercase().replace('-', "_")
    )
}

fn presence_label(present: bool) -> &'static str {
    if present {
        "present"
    } else {
        "missing"
    }
}

fn format_library_item(kind: &str, name: &str, description: &str) -> String {
    format!(
        "  {kind} {} - {}",
        compact_ui_text(name, 28),
        compact_ui_text(description, 62)
    )
}

fn delivery_plan_status(observation: &SessionObservation) -> String {
    if observation.plan_total == 0 {
        return "missing plan; run /plan".to_string();
    }
    if observation.plan_failed > 0 {
        return format!(
            "blocked failed={}/{}",
            observation.plan_failed, observation.plan_total
        );
    }
    if observation.plan_completed == observation.plan_total {
        return format!(
            "ok {}/{}",
            observation.plan_completed, observation.plan_total
        );
    }
    format!(
        "pending {}/{} running={}",
        observation.plan_completed, observation.plan_total, observation.plan_in_progress
    )
}

fn delivery_test_status(monitor: &SessionMonitor) -> String {
    let latest = monitor
        .observation
        .latest_test
        .as_ref()
        .or_else(|| monitor.recent_tests.last());
    match latest {
        Some(test) if test.passed => format!("ok {}", compact_ui_text(&test.command, 50)),
        Some(test) => format!(
            "failing code={} {}",
            test.exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "-".to_string()),
            compact_ui_text(&test.command, 50)
        ),
        None => "missing; run /test run --json".to_string(),
    }
}

fn delivery_environment_status(monitor: &SessionMonitor) -> String {
    match monitor.recent_environment.last() {
        Some(environment) if environment.ready == Some(true) => {
            format!("ok target={}", environment.target)
        }
        Some(environment) => format!(
            "{} target={}",
            environment.status,
            compact_ui_text(&environment.target, 32)
        ),
        None => "not requested; add --env-check when Docker/compiler matters".to_string(),
    }
}

fn format_optional_ms(value: Option<u128>) -> String {
    value
        .map(|value| format!("{value}ms"))
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_bytes(value: Option<usize>) -> String {
    value
        .map(|value| format!("{}KiB", value.div_ceil(1024)))
        .unwrap_or_else(|| "-".to_string())
}

fn format_cache_hit_rate(usage: &SessionObservationUsage) -> String {
    let Some(hit) = usage.prompt_cache_hit_tokens else {
        return "-".to_string();
    };
    let miss = usage.prompt_cache_miss_tokens.unwrap_or_default();
    let total = hit + miss;
    if total == 0 {
        "-".to_string()
    } else {
        format!("{:.1}%", hit as f64 * 100.0 / total as f64)
    }
}

fn format_latest_environment(environment: &SessionObservationEnvironment) -> String {
    let ready = environment
        .ready
        .map(|ready| format!(" ready={ready}"))
        .unwrap_or_default();
    let detail = if environment.detail.is_empty() {
        String::new()
    } else {
        format!(" {}", compact_ui_text(&environment.detail, 86))
    };
    format!(
        "{} target={} status={}{}{}",
        environment.tool, environment.target, environment.status, ready, detail
    )
}

fn compact_ui_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let keep = limit.saturating_sub(3);
    let mut output = value.chars().take(keep).collect::<String>();
    output.push_str("...");
    output
}

fn format_action_event(prefix: &str, output: &str) -> String {
    let summary = first_non_empty_line(output).unwrap_or("<empty>");
    format!("{prefix}: {}", compact_ui_text(summary, 80))
}

fn latest_action_result_line(state: &TuiState) -> Option<String> {
    latest_action_result(state).map(|result| {
        format!(
            "last output: {} {}",
            result.status,
            compact_ui_text(result.summary, 92)
        )
    })
}

struct LatestActionResult<'a> {
    status: &'static str,
    summary: &'a str,
    content: &'a str,
}

fn latest_action_result(state: &TuiState) -> Option<LatestActionResult<'_>> {
    state.chat.iter().rev().find_map(|line| {
        let status = match line.role.as_str() {
            "deepcli" => "ok",
            "error" => "error",
            _ => return None,
        };
        let summary = first_non_empty_line(&line.content)?;
        Some(LatestActionResult {
            status,
            summary,
            content: &line.content,
        })
    })
}

fn non_empty_output_lines(value: &str) -> Vec<&str> {
    value
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect()
}

fn first_non_empty_line(value: &str) -> Option<&str> {
    value.lines().map(str::trim).find(|line| !line.is_empty())
}

fn slash_command_suggestions_for_state(
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

fn running_safe_palette_priority(name: &str) -> usize {
    match name {
        "/help" => 0,
        "/version" => 1,
        "/status" => 2,
        "/usage" => 3,
        "/health" => 4,
        "/logs" => 5,
        "/trace" => 6,
        "/stop" => 7,
        "/quit" => 8,
        "/selftest" => 9,
        "/preflight" => 10,
        "/completion" => 11,
        "/round" => 12,
        "/scorecard" => 13,
        "/benchmark" => 14,
        "/recipes" => 15,
        _ => 100,
    }
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

fn render_command_palette(
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

fn format_command_palette_text(
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

fn command_palette_match_token(
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

fn command_palette_matches_line_index(running: bool) -> usize {
    if running {
        1
    } else {
        0
    }
}

fn render_resume_picker(frame: &mut Frame<'_>, area: Rect, picker: &ResumePicker) {
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
                    .title("Resume Preview (click select, Enter confirm, Esc cancel)"),
            ),
        preview_area,
    );
}

fn resume_picker_layout(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);
    (chunks[0], chunks[1])
}

fn format_resume_preview_text(picker: &ResumePicker, height: u16) -> String {
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

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

pub fn render_dashboard(frame: &mut Frame<'_>, area: Rect, snapshot: &TuiSnapshot) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(5),
        ])
        .split(area);

    let header = Paragraph::new(Line::from(vec![
        Span::styled("deepcli", Style::default().fg(Color::Cyan)),
        Span::raw(format!(
            "  session={} provider={} model={} state={}",
            snapshot.session_id, snapshot.provider, snapshot.model, snapshot.state
        )),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(header, chunks[0]);

    let items = snapshot
        .plan_steps
        .iter()
        .map(|step| ListItem::new(step.clone()))
        .collect::<Vec<_>>();
    let plan = List::new(items).block(Block::default().borders(Borders::ALL).title("Plan"));
    frame.render_widget(plan, chunks[1]);

    let footer = Paragraph::new(format!(
        "tokens: {}\nlast: {}",
        snapshot.token_usage, snapshot.last_event
    ))
    .wrap(Wrap { trim: true })
    .block(Block::default().borders(Borders::ALL).title("Trace"));
    frame.render_widget(footer, chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::permissions::{DecisionOutcome, PermissionDecision, RiskLevel};
    use crate::runtime::RuntimeOptions;
    use crate::runtime::SessionObservationEvent;
    use crate::session::{ApprovalStatus, SessionStore, SideQuestionStatus, ToolCallRecord};
    use chrono::Utc;
    use ratatui::{backend::TestBackend, Terminal};
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
    fn transcript_scroll_keys_move_history_window() {
        let mut state = TuiState {
            runtime: None,
            active_session: None,
            input: MessageBox::new(),
            chat: (0..12)
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
    fn transcript_mouse_wheel_scrolls_messages_area_only() {
        let mut state = TuiState {
            runtime: None,
            active_session: None,
            input: MessageBox::new(),
            chat: (0..12)
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
                column: layout.tools.x + 1,
                row: layout.tools.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
        assert_eq!(state.transcript_scroll, 0);
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Result,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Result,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 24,
        };
        let layout = chat_ui_layout(area);
        let (progress_tx, _progress_rx) = mpsc::channel();
        let (done_tx, _done_rx) = mpsc::channel();

        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: layout.tools.x + 1,
                row: layout.tools.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
        assert_eq!(state.result_scroll, RESULT_MOUSE_SCROLL_STEP);
        assert_eq!(state.transcript_scroll, 0);

        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: layout.tools.x + 1,
                row: layout.tools.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Changes,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 24,
        };
        let layout = chat_ui_layout(area);
        let (progress_tx, _progress_rx) = mpsc::channel();
        let (done_tx, _done_rx) = mpsc::channel();

        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: layout.tools.x + 1,
                row: layout.tools.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
        assert_eq!(state.change_patch_scroll, CHANGE_PATCH_MOUSE_SCROLL_STEP);
        assert_eq!(state.result_scroll, 0);

        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: layout.tools.x + 1,
                row: layout.tools.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 3,
            monitor_tab: MonitorTab::Changes,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 24,
        };
        let layout = chat_ui_layout(area);
        let (progress_tx, _progress_rx) = mpsc::channel();
        let (done_tx, _done_rx) = mpsc::channel();

        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: layout.tools.x + 2,
                row: layout.tools.y + 1 + 4,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
        assert_eq!(state.selected_change, 1);
        assert_eq!(state.change_patch_scroll, 0);
        assert!(state.last_event.contains("src/ui.rs"));
        state.workspace_changes.as_mut().unwrap().paths =
            vec!["src/lib.rs".to_string(), "notes.md".to_string()];

        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: layout.tools.x + 2,
                row: layout.tools.y + 1 + 4,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
    fn slash_command_palette_filters_formats_and_completes() {
        let suggestions = slash_command_suggestions_for_state("/en", false).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].name, "/env");
        assert!(!suggestions[0].running_safe);

        let text = format_command_palette_text(&suggestions, 0, 20, false);
        assert!(text.contains("selected: /env check"));
        assert!(text.contains("/env plan [docker|compiler] [--smoke]"));
        assert!(text.contains("/env setup docker --smoke"));
        assert!(!text.contains("running-safe: yes"));

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
            .any(|summary| summary.name == "/version" && summary.running_safe));
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
            .any(|summary| summary.name == "/health"));
        assert!(running_suggestions
            .iter()
            .any(|summary| summary.name == "/logs" && summary.running_safe));
        assert!(running_suggestions
            .iter()
            .any(|summary| summary.name == "/round" && summary.running_safe));
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
        assert!(version_suggestions[0].running_safe);
        let approval_suggestions = slash_command_suggestions_for_state("/app", true).unwrap();
        assert!(approval_suggestions
            .iter()
            .any(|summary| summary.name == "/approval" && summary.running_safe));
        let check_suggestions = slash_command_suggestions_for_state("/che", true).unwrap();
        assert!(check_suggestions
            .iter()
            .any(|summary| summary.name == "/check" && summary.running_safe));
        let handoff_suggestions = slash_command_suggestions_for_state("/han", true).unwrap();
        assert!(handoff_suggestions
            .iter()
            .any(|summary| summary.name == "/handoff" && summary.running_safe));

        let docker_suggestions = slash_command_suggestions_for_state("/do", true).unwrap();
        assert!(docker_suggestions
            .iter()
            .any(|summary| summary.name == "/docker" && summary.running_safe));
        let compiler_suggestions = slash_command_suggestions_for_state("/com", true).unwrap();
        assert!(compiler_suggestions
            .iter()
            .any(|summary| summary.name == "/compiler" && summary.running_safe));
        let model_suggestions = slash_command_suggestions_for_state("/mo", false).unwrap();
        assert!(model_suggestions
            .iter()
            .any(|summary| summary.name == "/models"));
        let provider_suggestions = slash_command_suggestions_for_state("/prov", false).unwrap();
        assert!(provider_suggestions
            .iter()
            .any(|summary| summary.name == "/providers"));
        let history_suggestions = slash_command_suggestions_for_state("/hi", false).unwrap();
        assert_eq!(history_suggestions[0].name, "/history");
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        state.input.set_buffer("/en".to_string());
        complete_selected_command(&mut state, &suggestions);
        assert_eq!(state.input.buffer(), "/env ");
        assert_eq!(state.last_event, "completed /env");
    }

    #[test]
    fn slash_command_palette_mouse_selects_and_completes_match() {
        let suggestions = slash_command_suggestions_for_state("/", false).unwrap();
        assert!(suggestions.len() > 1);
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Result,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        state.input.set_buffer("/".to_string());
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
                column: layout.tools.x + 2,
                row: layout.tools.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
        assert_eq!(state.selected_command, 1);
        assert_eq!(state.result_scroll, 0);
        assert!(state.last_event.starts_with("command selected: "));

        let offset = "matches: ".len()
            + command_palette_match_token(0, state.selected_command, &suggestions[0]).len()
            + 2;
        let selected = suggestions[state.selected_command].name;
        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: layout.tools.x + 1 + offset as u16 + 1,
                row: layout.tools.y + 1 + command_palette_matches_line_index(false) as u16,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
        assert_eq!(state.input.buffer(), format!("{selected} "));
        assert_eq!(state.selected_command, 0);
        assert_eq!(state.last_event, format!("completed {selected}"));
    }

    #[test]
    fn slash_command_palette_can_be_rendered_in_tools_area() {
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
            selected_tool: Some(0),
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        state.input.set_buffer("/env".to_string());

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
        assert!(rendered.contains("Command Help"));
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "deepcli: tool run_tests failed".to_string(),
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
        let overview =
            format_task_overview_lines(&state, Some(&observation), &actions, 0).join("\n");
        assert!(overview.contains("state=Testing ui=running"));
        assert!(overview.contains("plan=2/4 running=1 failed=1"));
        assert!(overview.contains("approvals=1 btw=2"));
        assert!(overview.contains("current=repair failing compiler tests"));
        assert!(overview.contains("test=fail code=101 cargo test --all-targets"));
        assert!(overview.contains("tools=3 failed_tools=1"));
        assert!(overview.contains("last output: ok verify complete"));
        assert!(overview.contains("> /status --json"));
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Tests,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
                detail: "recommended: /env setup docker --smoke".to_string(),
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

        state.monitor_tab = MonitorTab::Environment;
        let environment = format_task_monitor_text(&state, Some(&monitor), 14);
        assert!(environment.contains("[Environment]"));
        assert!(environment.contains("check_environment target=docker status=needs_setup"));
        assert!(environment.contains("/env plan docker --smoke --json"));
        assert!(environment.contains("/env setup docker --smoke (edit)"));
        assert!(environment.contains("/env test docker --json"));
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Changes,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Changes,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
    fn diff_sections_split_by_file_and_cap_long_sections() {
        let mut diff = String::from(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n",
        );
        for index in 0..(WORKTREE_DIFF_SECTION_LINES + 5) {
            diff.push_str(&format!("+line-{index}\n"));
        }
        diff.push_str(
            "diff --git a/src/ui.rs b/src/ui.rs\n--- a/src/ui.rs\n+++ b/src/ui.rs\n+ui\n",
        );

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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        state.input.set_buffer("hello".to_string());

        cycle_monitor_tab(&mut state, true);
        assert_eq!(state.monitor_tab, MonitorTab::Result);
        cycle_monitor_tab(&mut state, true);
        assert_eq!(state.monitor_tab, MonitorTab::Changes);
        cycle_monitor_tab(&mut state, true);
        assert_eq!(state.monitor_tab, MonitorTab::Usage);
        cycle_monitor_tab(&mut state, true);
        assert_eq!(state.monitor_tab, MonitorTab::Health);
        cycle_monitor_tab(&mut state, true);
        assert_eq!(state.monitor_tab, MonitorTab::Library);
        cycle_monitor_tab(&mut state, true);
        assert_eq!(state.monitor_tab, MonitorTab::Deliver);
        cycle_monitor_tab(&mut state, true);
        assert_eq!(state.monitor_tab, MonitorTab::Tools);
        assert_eq!(state.input.buffer(), "hello");
        cycle_monitor_tab(&mut state, false);
        assert_eq!(state.monitor_tab, MonitorTab::Deliver);
    }

    #[test]
    fn monitor_tabs_can_be_clicked_from_task_monitor_header() {
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
            selected_tool: None,
            selected_command: 2,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        let area = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 24,
        };
        let layout = chat_ui_layout(area);
        let tabs = format_monitor_tabs(state.monitor_tab);
        let changes_offset = tabs.find("Changes").unwrap() as u16;
        let result_offset = tabs.find("Result").unwrap() as u16;
        let (progress_tx, _progress_rx) = mpsc::channel();
        let (done_tx, _done_rx) = mpsc::channel();

        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: layout.tools.x + 1 + changes_offset,
                row: layout.tools.y + 1,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );
        assert_eq!(state.monitor_tab, MonitorTab::Changes);
        assert_eq!(state.selected_command, 0);
        assert_eq!(state.last_event, "monitor tab: Changes");

        state.input.set_buffer("/he".to_string());
        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: layout.tools.x + 1 + result_offset,
                row: layout.tools.y + 1,
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Health,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };

        let health = format_task_monitor_text(&state, None, 14);
        assert!(health.contains("[Health]"));
        assert!(
            health.contains("provider: active=healthtest model=session-model default=healthtest")
        );
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Health,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Library,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Library,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
    fn monitor_quick_actions_can_be_clicked_from_task_monitor() {
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Environment,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 24,
        };
        let layout = chat_ui_layout(area);
        let rendered = format_task_monitor_text(&state, None, layout.tools.height);
        let action_row = rendered
            .lines()
            .position(|line| line.contains("/env check docker --json"))
            .expect("environment check quick action should be visible");
        let (progress_tx, _progress_rx) = mpsc::channel();
        let (done_tx, _done_rx) = mpsc::channel();

        handle_tui_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: layout.tools.x + 4,
                row: layout.tools.y + 1 + action_row as u16,
                modifiers: KeyModifiers::NONE,
            },
            &progress_tx,
            &done_tx,
            area,
        );

        assert_eq!(state.selected_command, 0);
        assert!(state
            .last_event
            .contains("quick action submitted: /env check docker --json"));
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
            selected_tool: None,
            selected_command: 6,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Environment,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Approvals,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Approvals,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };

        assert_eq!(blocker_count(&state), Some(1));
        assert!(handle_approval_tab_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut state
        ));
        assert_eq!(state.last_event, "btw answer prompt opened");
        assert_eq!(state.input.buffer(), "");
        assert!(state.side_question_prompt.is_some());
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
    fn approvals_tab_mouse_selects_blockers_without_acting() {
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Approvals,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
                column: layout.tools.x + 2,
                row: layout.tools.y + 2,
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
                column: layout.tools.x + 2,
                row: layout.tools.y + 1 + 2,
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
                column: layout.tools.x + 2,
                row: layout.tools.y + 1 + 4,
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "running".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "running".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "running".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "running".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "running".to_string(),
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
                    "recommended_action": "/env setup compiler --smoke"
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Overview,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "running".to_string(),
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
        assert!(environment.contains("recommended: /env setup compiler --smoke"));
        let actions = environment_quick_actions(Some(&monitor));
        assert!(actions.iter().any(
            |action| action.command == "/env setup compiler --smoke" && action.edit_before_run
        ));
        assert!(actions
            .iter()
            .any(|action| action.command == "/env test compiler --json"));

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
                detail: "recommended: /env setup compiler --smoke".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Environment,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        let actions = environment_quick_actions(Some(&monitor));
        let setup_index = actions
            .iter()
            .position(|action| action.command == "/env setup compiler --smoke")
            .unwrap();
        assert!(actions[setup_index].edit_before_run);
        state.selected_command = setup_index;
        let (progress_tx, _progress_rx) = mpsc::channel();
        let (done_tx, _done_rx) = mpsc::channel();

        activate_selected_monitor_quick_action(&mut state, &actions, &progress_tx, &done_tx);

        assert_eq!(state.input.buffer(), "/env setup compiler --smoke");
        assert_eq!(
            state.last_event,
            "quick action ready for edit: /env setup compiler --smoke"
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Approvals,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "running".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Approvals,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "running".to_string(),
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
    fn task_monitor_renders_overview_and_tool_calls() {
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
            selected_tool: Some(0),
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Tools,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
        assert!(rendered.contains("Task Monitor"));
        assert!(rendered.contains("Tools"));
        assert!(rendered.contains("tool: read_file"));
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
            selected_tool: Some(0),
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Tools,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
            selected_tool: Some(7),
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Tools,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
            selected_tool: Some(0),
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Tools,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };

        let rendered = format_task_monitor_text(&state, None, 14);

        assert!(rendered.contains("selected detail: tool: run_tests [failed]"));
        assert!(rendered.contains("stderr line 0:"));
        assert!(rendered
            .contains("[detail truncated; Ctrl-O prefill full output, Ctrl-F failed tools]"));
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
            selected_tool: Some(0),
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Tools,
            selected_approval: 0,
            running: true,
            exit_requested: false,
            last_event: "ready".to_string(),
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
            selected_tool: None,
            selected_command: 0,
            selected_change: 0,
            change_patch_scroll: 0,
            monitor_tab: MonitorTab::Result,
            selected_approval: 0,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
            worker: None,
        };
        let area = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 24,
        };
        let layout = chat_ui_layout(area);
        let (list_area, _) = resume_picker_layout(layout.tools);
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
        assert_eq!(picker.selected, 1);

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
}
