use crate::runtime::AgentRuntime;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::io::{self, Write};

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
