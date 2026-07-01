use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

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
