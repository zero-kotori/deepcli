use crate::runtime::{AgentRuntime, RuntimeProgress};
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io::{self, Stdout, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageBoxAction {
    Inserted,
    Submitted(String),
    Noop,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageBox {
    buffer: String,
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

    pub fn handle_key(&mut self, key: KeyEvent) -> MessageBoxAction {
        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.buffer.push('\n');
                MessageBoxAction::Inserted
            }
            KeyCode::Enter => {
                let submitted = self.buffer.trim_end().to_string();
                self.buffer.clear();
                self.history_index = None;
                if !submitted.is_empty() {
                    self.history.push(submitted.clone());
                }
                MessageBoxAction::Submitted(submitted)
            }
            KeyCode::Char(ch) => {
                self.buffer.push(ch);
                MessageBoxAction::Inserted
            }
            KeyCode::Backspace => {
                self.buffer.pop();
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
                MessageBoxAction::Inserted
            }
            KeyCode::Down => {
                let Some(index) = self.history_index else {
                    return MessageBoxAction::Noop;
                };
                if index + 1 >= self.history.len() {
                    self.history_index = None;
                    self.buffer.clear();
                } else {
                    self.history_index = Some(index + 1);
                    self.buffer = self.history[index + 1].clone();
                }
                MessageBoxAction::Inserted
            }
            _ => MessageBoxAction::Noop,
        }
    }
}

pub async fn run_basic_repl(runtime: &mut AgentRuntime) -> Result<()> {
    println!("deep-cli session {}", runtime.session_id());
    println!("Type /help for commands, Ctrl-D to exit.");
    let stdin = io::stdin();
    loop {
        print!("deep-cli> ");
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

#[derive(Debug)]
struct ChatLine {
    role: String,
    content: String,
}

struct TuiState {
    runtime: Option<AgentRuntime>,
    input: MessageBox,
    chat: Vec<ChatLine>,
    tool_log: Vec<ToolLogItem>,
    selected_tool: Option<usize>,
    running: bool,
    exit_requested: bool,
    last_event: String,
}

struct WorkerDone {
    runtime: AgentRuntime,
    result: std::result::Result<String, String>,
}

pub async fn run_tui(mut runtime: AgentRuntime) -> Result<()> {
    let (progress_tx, progress_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    runtime.set_progress_sender(Some(progress_tx.clone()));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_tui_loop(
        &mut terminal,
        TuiState {
            runtime: Some(runtime),
            input: MessageBox::new(),
            chat: Vec::new(),
            tool_log: Vec::new(),
            selected_tool: None,
            running: false,
            exit_requested: false,
            last_event: "ready".to_string(),
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
        DisableMouseCapture
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

        terminal.draw(|frame| render_chat_ui(frame, &state))?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => handle_tui_key(key, &mut state, &progress_tx, &done_tx)?,
                Event::Mouse(mouse) => {
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                        let size = terminal.size()?;
                        let areas = chat_ui_layout(Rect {
                            x: 0,
                            y: 0,
                            width: size.width,
                            height: size.height,
                        });
                        toggle_tool_at_row(&mut state, areas.tools, mouse.row);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn handle_tui_key(
    key: KeyEvent,
    state: &mut TuiState,
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
) -> Result<()> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'd'))
    {
        state.exit_requested = !state.running;
        return Ok(());
    }
    if key.code == KeyCode::Esc {
        state.exit_requested = !state.running;
        return Ok(());
    }
    match key.code {
        KeyCode::Tab => {
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
                submit_tui_input(state, input, progress_tx.clone(), done_tx.clone())
            }
            MessageBoxAction::Inserted | MessageBoxAction::Noop => {}
        },
    }
    Ok(())
}

fn submit_tui_input(
    state: &mut TuiState,
    input: String,
    progress_tx: Sender<RuntimeProgress>,
    done_tx: Sender<WorkerDone>,
) {
    if input.trim().is_empty() || state.running {
        return;
    }
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
    tokio::spawn(async move {
        let result = runtime
            .handle_input(&input)
            .await
            .map_err(|error| error.to_string());
        let _ = done_tx.send(WorkerDone { runtime, result });
    });
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
        state.runtime = Some(done.runtime);
        state.running = false;
        match done.result {
            Ok(output) => state.chat.push(ChatLine {
                role: "deep-cli".to_string(),
                content: output,
            }),
            Err(error) => state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error,
            }),
        }
        state.last_event = "ready".to_string();
    }
}

fn toggle_selected_tool(state: &mut TuiState) {
    if let Some(index) = state.selected_tool {
        if let Some(item) = state.tool_log.get_mut(index) {
            item.expanded = !item.expanded;
        }
    }
}

fn toggle_tool_at_row(state: &mut TuiState, tools_area: Rect, row: u16) {
    if row <= tools_area.y || row >= tools_area.y + tools_area.height.saturating_sub(1) {
        return;
    }
    let index = row.saturating_sub(tools_area.y + 1) as usize;
    if let Some(item) = state.tool_log.get_mut(index) {
        item.expanded = !item.expanded;
        state.selected_tool = Some(index);
    }
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

fn render_chat_ui(frame: &mut Frame<'_>, state: &TuiState) {
    let areas = chat_ui_layout(frame.area());
    let runtime = state.runtime.as_ref();
    let session = runtime
        .map(|runtime| runtime.session_id())
        .unwrap_or_else(|| "<running>".to_string());
    let provider = runtime
        .map(|runtime| runtime.provider_name().to_string())
        .unwrap_or_else(|| "<running>".to_string());
    let model = runtime
        .and_then(|runtime| runtime.model_name().map(str::to_string))
        .unwrap_or_else(|| "<unset>".to_string());
    let state_label = runtime
        .map(|runtime| runtime.state_label())
        .unwrap_or_else(|| "Running".to_string());

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "deep-cli",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  session={session} provider={provider} model={model} state={state_label}"
        )),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(header, areas.header);

    let transcript = state
        .chat
        .iter()
        .rev()
        .take(12)
        .rev()
        .map(|line| format!("{}: {}", line.role, line.content))
        .collect::<Vec<_>>()
        .join("\n\n");
    frame.render_widget(
        Paragraph::new(transcript)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title("Messages")),
        areas.transcript,
    );

    let tools = state
        .tool_log
        .iter()
        .enumerate()
        .rev()
        .take(7)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|(index, item)| {
            let marker = if item.expanded { "v" } else { ">" };
            let selected = if state.selected_tool == Some(index) {
                "*"
            } else {
                " "
            };
            let text = if item.expanded {
                format!("{selected} {marker} {}\n  {}", item.title, item.detail)
            } else {
                format!("{selected} {marker} {}", item.title)
            };
            ListItem::new(text)
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(tools).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Tool Calls (click or Ctrl-Enter to expand)"),
        ),
        areas.tools,
    );

    let input_title = if state.running {
        "Message Box (running; Ctrl-C after completion to exit)"
    } else {
        "Message Box (Enter send, Shift-Enter newline, Esc exit)"
    };
    frame.render_widget(
        Paragraph::new(state.input.buffer())
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(input_title)),
        areas.input,
    );
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
        Span::styled("deep-cli", Style::default().fg(Color::Cyan)),
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
    use ratatui::{backend::TestBackend, Terminal};

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
