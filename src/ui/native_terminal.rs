use crate::runtime::{
    AgentRuntime, RuntimeProgress, SessionObservationApproval, SessionObservationQuestion,
};
use anyhow::{anyhow, Result};
use crossterm::{
    cursor::{MoveToColumn, MoveUp},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute, queue,
    terminal::{self, disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;
use unicode_width::UnicodeWidthChar;

const NATIVE_INPUT_PROMPT_LABEL: &str = "you  ";
const NATIVE_INPUT_PROMPT: &str = "\x1b[1;36myou\x1b[0m  ";
const NATIVE_ASSISTANT_LABEL: &str = "\x1b[1;32mdeepcli\x1b[0m";
const NATIVE_ERROR_LABEL: &str = "\x1b[1;31merror\x1b[0m";
const NATIVE_PROGRESS_DETAIL_CHARS: usize = 120;

struct WorkerDone {
    runtime: AgentRuntime,
    result: Result<String, String>,
}

#[derive(Default)]
struct TerminalTextSanitizer {
    state: TerminalEscapeState,
}

#[derive(Default)]
enum TerminalEscapeState {
    #[default]
    Text,
    Escape,
    Csi,
    Osc,
    OscEscape,
    ControlString,
    ControlStringEscape,
}

impl TerminalTextSanitizer {
    fn sanitize_chunk(&mut self, value: &str) -> String {
        let mut sanitized = String::with_capacity(value.len());
        for ch in value.chars() {
            match self.state {
                TerminalEscapeState::Text => match ch {
                    '\x1b' => self.state = TerminalEscapeState::Escape,
                    '\u{009b}' => self.state = TerminalEscapeState::Csi,
                    '\u{009d}' => self.state = TerminalEscapeState::Osc,
                    '\u{0090}' | '\u{0098}' | '\u{009e}' | '\u{009f}' => {
                        self.state = TerminalEscapeState::ControlString;
                    }
                    '\n' | '\t' => sanitized.push(ch),
                    ch if ch.is_control() => {}
                    ch => sanitized.push(ch),
                },
                TerminalEscapeState::Escape => match ch {
                    '[' => self.state = TerminalEscapeState::Csi,
                    ']' => self.state = TerminalEscapeState::Osc,
                    'P' | 'X' | '^' | '_' => {
                        self.state = TerminalEscapeState::ControlString;
                    }
                    '\x1b' => {}
                    ch if (' '..='/').contains(&ch) => {}
                    _ => self.state = TerminalEscapeState::Text,
                },
                TerminalEscapeState::Csi => {
                    if ch == '\x1b' {
                        self.state = TerminalEscapeState::Escape;
                    } else if ('@'..='~').contains(&ch) {
                        self.state = TerminalEscapeState::Text;
                    }
                }
                TerminalEscapeState::Osc => match ch {
                    '\x07' | '\u{009c}' => self.state = TerminalEscapeState::Text,
                    '\x1b' => self.state = TerminalEscapeState::OscEscape,
                    _ => {}
                },
                TerminalEscapeState::OscEscape => match ch {
                    '\\' | '\u{009c}' => self.state = TerminalEscapeState::Text,
                    '\x1b' => {}
                    _ => self.state = TerminalEscapeState::Osc,
                },
                TerminalEscapeState::ControlString => match ch {
                    '\u{009c}' => self.state = TerminalEscapeState::Text,
                    '\x1b' => self.state = TerminalEscapeState::ControlStringEscape,
                    _ => {}
                },
                TerminalEscapeState::ControlStringEscape => match ch {
                    '\\' | '\u{009c}' => self.state = TerminalEscapeState::Text,
                    '\x1b' => {}
                    _ => self.state = TerminalEscapeState::ControlString,
                },
            }
        }
        sanitized
    }

    fn reset(&mut self) {
        self.state = TerminalEscapeState::Text;
    }
}

#[derive(Default)]
struct NativeRenderState {
    assistant_open: bool,
    saw_assistant_delta: bool,
    assistant_ended_with_newline: bool,
    assistant_sanitizer: TerminalTextSanitizer,
}

impl NativeRenderState {
    fn sanitize_assistant_delta(&mut self, delta: &str) -> Option<String> {
        let delta = self.assistant_sanitizer.sanitize_chunk(delta);
        if delta.is_empty() {
            return None;
        }
        self.saw_assistant_delta = true;
        self.assistant_ended_with_newline = delta.ends_with('\n');
        Some(delta)
    }
}

pub(super) async fn run_native_terminal(mut runtime: AgentRuntime) -> Result<()> {
    println!("{}", native_session_banner(&runtime.session_id()));
    println!();

    let (progress_tx, progress_rx) = mpsc::channel();
    let mut stdout = io::stdout();
    while let Some(input) = read_native_input(&mut stdout)? {
        if input.trim().is_empty() {
            continue;
        }
        if input.trim() == "/quit" {
            break;
        }
        if let Some(answer) = answer_native_side_question(&mut runtime, &input)? {
            println!("{}", terminal_safe_text(&answer.message));
            print_native_open_questions(&runtime)?;
            if answer.continue_planning {
                runtime.set_progress_sender(Some(progress_tx.clone()));
                let (done_tx, done_rx) = mpsc::channel();
                let mut task_runtime = runtime;
                tokio::spawn(async move {
                    let result = task_runtime
                        .continue_planning_after_side_question_answer()
                        .await
                        .map_err(|error| error.to_string());
                    let _ = done_tx.send(WorkerDone {
                        runtime: task_runtime,
                        result,
                    });
                });

                let mut render_state = NativeRenderState::default();
                runtime = wait_for_native_task(done_rx, &progress_rx, &mut render_state).await?;
            }
            continue;
        }

        runtime.set_progress_sender(Some(progress_tx.clone()));
        let (done_tx, done_rx) = mpsc::channel();
        let mut task_runtime = runtime;
        let task_input = input.clone();
        tokio::spawn(async move {
            let result = task_runtime
                .handle_input(&task_input)
                .await
                .map_err(|error| error.to_string());
            let _ = done_tx.send(WorkerDone {
                runtime: task_runtime,
                result,
            });
        });

        let mut render_state = NativeRenderState::default();
        runtime = wait_for_native_task(done_rx, &progress_rx, &mut render_state).await?;
    }

    Ok(())
}

#[derive(Default)]
struct NativeInputEditor {
    buffer: String,
    cursor: usize,
    preferred_column: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
enum NativeInputAction {
    Edited,
    Submitted(String),
    Exit,
    Noop,
}

impl NativeInputEditor {
    fn buffer(&self) -> &str {
        &self.buffer
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    #[cfg(test)]
    fn set_cursor(&mut self, cursor: usize) {
        self.cursor = self.clamp_to_char_boundary(cursor);
        self.preferred_column = None;
    }

    fn handle_key(&mut self, key: KeyEvent) -> NativeInputAction {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                NativeInputAction::Exit
            }
            KeyCode::Char('d')
                if key.modifiers.contains(KeyModifiers::CONTROL) && self.buffer.is_empty() =>
            {
                NativeInputAction::Exit
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_at_cursor();
                NativeInputAction::Edited
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                NativeInputAction::Submitted(self.buffer.trim_end().to_string())
            }
            KeyCode::Char('\n') | KeyCode::Char('\r') => {
                NativeInputAction::Submitted(self.buffer.trim_end().to_string())
            }
            KeyCode::Enter
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.insert_char('\n');
                NativeInputAction::Edited
            }
            KeyCode::Enter => NativeInputAction::Submitted(self.buffer.trim_end().to_string()),
            KeyCode::Char(ch)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL
                        | KeyModifiers::ALT
                        | KeyModifiers::SUPER
                        | KeyModifiers::HYPER
                        | KeyModifiers::META,
                ) =>
            {
                self.insert_char(ch);
                NativeInputAction::Edited
            }
            KeyCode::Backspace => {
                self.delete_before_cursor();
                NativeInputAction::Edited
            }
            KeyCode::Delete => {
                self.delete_at_cursor();
                NativeInputAction::Edited
            }
            KeyCode::Left => {
                self.cursor = self.previous_char_boundary();
                self.preferred_column = None;
                NativeInputAction::Edited
            }
            KeyCode::Right => {
                self.cursor = self.next_char_boundary();
                self.preferred_column = None;
                NativeInputAction::Edited
            }
            KeyCode::Up => {
                self.move_up();
                NativeInputAction::Edited
            }
            KeyCode::Down => {
                self.move_down();
                NativeInputAction::Edited
            }
            KeyCode::Home => {
                self.cursor = self.current_line_start();
                self.preferred_column = None;
                NativeInputAction::Edited
            }
            KeyCode::End => {
                self.cursor = self.current_line_end();
                self.preferred_column = None;
                NativeInputAction::Edited
            }
            _ => NativeInputAction::Noop,
        }
    }

    fn insert_char(&mut self, ch: char) {
        let mut encoded = [0; 4];
        self.insert_str(ch.encode_utf8(&mut encoded));
    }

    fn insert_str(&mut self, value: &str) {
        let value = terminal_safe_text(value);
        if value.is_empty() {
            return;
        }
        self.buffer.insert_str(self.cursor, &value);
        self.cursor += value.len();
        self.preferred_column = None;
    }

    fn delete_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = self.previous_char_boundary();
        self.buffer.drain(previous..self.cursor);
        self.cursor = previous;
        self.preferred_column = None;
    }

    fn delete_at_cursor(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = self.next_char_boundary();
        self.buffer.drain(self.cursor..next);
        self.preferred_column = None;
    }

    fn move_up(&mut self) {
        let target_column = self
            .preferred_column
            .unwrap_or_else(|| visual_column(&self.buffer, self.current_line_start(), self.cursor));
        let current_start = self.current_line_start();
        if current_start == 0 {
            self.cursor = 0;
            self.preferred_column = Some(target_column);
            return;
        }

        let previous_end = current_start.saturating_sub(1);
        let previous_start = self.line_start_at(previous_end);
        self.cursor =
            byte_index_for_visual_column(&self.buffer, previous_start, previous_end, target_column);
        self.preferred_column = Some(target_column);
    }

    fn move_down(&mut self) {
        let target_column = self
            .preferred_column
            .unwrap_or_else(|| visual_column(&self.buffer, self.current_line_start(), self.cursor));
        let current_end = self.current_line_end();
        if current_end >= self.buffer.len() {
            self.cursor = current_end;
            self.preferred_column = Some(target_column);
            return;
        }

        let next_start = current_end + 1;
        let next_end = self.line_end_at(next_start);
        self.cursor =
            byte_index_for_visual_column(&self.buffer, next_start, next_end, target_column);
        self.preferred_column = Some(target_column);
    }

    fn current_line_start(&self) -> usize {
        self.line_start_at(self.cursor)
    }

    fn current_line_end(&self) -> usize {
        self.line_end_at(self.cursor)
    }

    fn line_start_at(&self, cursor: usize) -> usize {
        let cursor = self.clamp_to_char_boundary(cursor);
        self.buffer[..cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0)
    }

    fn line_end_at(&self, cursor: usize) -> usize {
        let cursor = self.clamp_to_char_boundary(cursor);
        self.buffer[cursor..]
            .find('\n')
            .map(|index| cursor + index)
            .unwrap_or(self.buffer.len())
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
            .chars()
            .next()
            .map(|ch| self.cursor + ch.len_utf8())
            .unwrap_or(self.buffer.len())
    }

    fn clamp_to_char_boundary(&self, cursor: usize) -> usize {
        let mut cursor = cursor.min(self.buffer.len());
        while cursor > 0 && !self.buffer.is_char_boundary(cursor) {
            cursor -= 1;
        }
        cursor
    }
}

#[derive(Default)]
struct NativeInputRenderState {
    rendered: bool,
    cursor_row: u16,
}

struct NativeInputMetrics {
    total_rows: u16,
    cursor_row: u16,
    cursor_col: u16,
}

fn native_input_prompt() -> &'static str {
    NATIVE_INPUT_PROMPT
}

fn native_assistant_label() -> &'static str {
    NATIVE_ASSISTANT_LABEL
}

fn native_session_banner(session_id: &str) -> String {
    let short_id = session_id.chars().take(8).collect::<String>();
    format!("{NATIVE_ASSISTANT_LABEL}  \x1b[2msession {short_id}\x1b[0m")
}

fn read_native_input(stdout: &mut io::Stdout) -> io::Result<Option<String>> {
    enable_raw_mode()?;
    if let Err(error) = execute!(stdout, EnableBracketedPaste) {
        let _ = disable_raw_mode();
        return Err(error);
    }
    let result = read_native_input_raw(stdout);
    let disable_paste = execute!(stdout, DisableBracketedPaste);
    let disable_raw = disable_raw_mode();
    match result {
        Ok(value) => {
            disable_paste?;
            disable_raw?;
            Ok(value)
        }
        Err(error) => {
            let _ = disable_paste;
            let _ = disable_raw;
            Err(error)
        }
    }
}

fn read_native_input_raw(stdout: &mut io::Stdout) -> io::Result<Option<String>> {
    let mut editor = NativeInputEditor::default();
    let mut render_state = NativeInputRenderState::default();
    render_native_input(stdout, &editor, &mut render_state)?;

    loop {
        match event::read()? {
            Event::Key(key) => match editor.handle_key(key) {
                NativeInputAction::Edited => {
                    render_native_input(stdout, &editor, &mut render_state)?
                }
                NativeInputAction::Submitted(input) => {
                    commit_native_input(stdout, &editor, &mut render_state)?;
                    return Ok(Some(input));
                }
                NativeInputAction::Exit => {
                    cancel_native_input(stdout, &mut render_state)?;
                    return Ok(None);
                }
                NativeInputAction::Noop => {}
            },
            Event::Paste(text) => {
                editor.insert_str(&normalize_native_paste(&text));
                render_native_input(stdout, &editor, &mut render_state)?;
            }
            Event::Resize(_, _) => render_native_input(stdout, &editor, &mut render_state)?,
            _ => {}
        }
    }
}

fn render_native_input(
    stdout: &mut io::Stdout,
    editor: &NativeInputEditor,
    state: &mut NativeInputRenderState,
) -> io::Result<()> {
    reset_native_input_area(stdout, state)?;
    write!(
        stdout,
        "{}{}",
        native_input_prompt(),
        render_input_buffer(editor.buffer())
    )?;
    let metrics = native_input_metrics(editor.buffer(), editor.cursor(), terminal_width());
    move_to_native_input_cursor(stdout, &metrics)?;
    stdout.flush()?;
    state.rendered = true;
    state.cursor_row = metrics.cursor_row;
    Ok(())
}

fn commit_native_input(
    stdout: &mut io::Stdout,
    editor: &NativeInputEditor,
    state: &mut NativeInputRenderState,
) -> io::Result<()> {
    reset_native_input_area(stdout, state)?;
    write!(
        stdout,
        "{}{}\r\n",
        native_input_prompt(),
        render_input_buffer(editor.buffer())
    )?;
    stdout.flush()?;
    state.rendered = false;
    state.cursor_row = 0;
    Ok(())
}

fn cancel_native_input(
    stdout: &mut io::Stdout,
    state: &mut NativeInputRenderState,
) -> io::Result<()> {
    reset_native_input_area(stdout, state)?;
    write!(stdout, "\r\n")?;
    stdout.flush()?;
    state.rendered = false;
    state.cursor_row = 0;
    Ok(())
}

fn reset_native_input_area(
    stdout: &mut io::Stdout,
    state: &NativeInputRenderState,
) -> io::Result<()> {
    if state.rendered && state.cursor_row > 0 {
        queue!(stdout, MoveUp(state.cursor_row))?;
    }
    if state.rendered {
        queue!(stdout, MoveToColumn(0), Clear(ClearType::FromCursorDown))?;
    }
    Ok(())
}

fn move_to_native_input_cursor(
    stdout: &mut io::Stdout,
    metrics: &NativeInputMetrics,
) -> io::Result<()> {
    let rows_below = metrics
        .total_rows
        .saturating_sub(1)
        .saturating_sub(metrics.cursor_row);
    if rows_below > 0 {
        queue!(stdout, MoveUp(rows_below))?;
    }
    queue!(stdout, MoveToColumn(metrics.cursor_col))?;
    Ok(())
}

fn render_input_buffer(buffer: &str) -> String {
    buffer.replace('\n', "\r\n")
}

fn normalize_native_paste(value: &str) -> String {
    terminal_safe_text(&value.replace("\r\n", "\n").replace('\r', "\n"))
}

fn terminal_safe_text(value: &str) -> String {
    TerminalTextSanitizer::default().sanitize_chunk(value)
}

fn terminal_width() -> usize {
    terminal::size()
        .map(|(width, _)| usize::from(width.max(1)))
        .unwrap_or(80)
}

fn native_input_metrics(buffer: &str, cursor: usize, width: usize) -> NativeInputMetrics {
    let cursor = clamp_str_boundary(buffer, cursor);
    let cursor_position = native_input_position(&buffer[..cursor], width);
    let end_position = native_input_position(buffer, width);
    NativeInputMetrics {
        total_rows: end_position.row.saturating_add(1),
        cursor_row: cursor_position.row,
        cursor_col: cursor_position.col,
    }
}

#[derive(Clone, Copy)]
struct NativeInputPosition {
    row: u16,
    col: u16,
}

fn native_input_position(buffer: &str, width: usize) -> NativeInputPosition {
    let mut row = 0u16;
    let mut col = 0u16;
    advance_position(&mut row, &mut col, NATIVE_INPUT_PROMPT_LABEL, width);
    advance_position(&mut row, &mut col, buffer, width);
    NativeInputPosition { row, col }
}

fn advance_position(row: &mut u16, col: &mut u16, value: &str, width: usize) {
    let width = width.max(1) as u16;
    for ch in value.chars() {
        if ch == '\n' {
            *row = row.saturating_add(1);
            *col = 0;
            continue;
        }
        let char_width = ch.width().unwrap_or(0).max(1) as u16;
        let char_width = char_width.min(width);
        if col.saturating_add(char_width) > width {
            *row = row.saturating_add(1);
            *col = 0;
        }
        *col = col.saturating_add(char_width);
        if *col >= width {
            *row = row.saturating_add(*col / width);
            *col %= width;
        }
    }
}

fn visual_column(buffer: &str, start: usize, end: usize) -> usize {
    buffer[start..end]
        .chars()
        .map(|ch| ch.width().unwrap_or(0).max(1))
        .sum()
}

fn byte_index_for_visual_column(
    buffer: &str,
    start: usize,
    end: usize,
    target_column: usize,
) -> usize {
    let mut column = 0usize;
    for (offset, ch) in buffer[start..end].char_indices() {
        let width = ch.width().unwrap_or(0).max(1);
        if column + width > target_column {
            return start + offset;
        }
        column += width;
        if column == target_column {
            return start + offset + ch.len_utf8();
        }
    }
    end
}

fn clamp_str_boundary(value: &str, cursor: usize) -> usize {
    let mut cursor = cursor.min(value.len());
    while cursor > 0 && !value.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

async fn wait_for_native_task(
    done_rx: Receiver<WorkerDone>,
    progress_rx: &Receiver<RuntimeProgress>,
    render_state: &mut NativeRenderState,
) -> Result<AgentRuntime> {
    loop {
        drain_native_progress(progress_rx, render_state)?;
        match done_rx.try_recv() {
            Ok(done) => {
                drain_native_progress(progress_rx, render_state)?;
                finish_native_stream_line(render_state)?;
                let runtime = done.runtime;
                let state = runtime.state_label();
                match done.result {
                    Ok(output) => {
                        if should_print_native_task_output(render_state.saw_assistant_delta, &state)
                        {
                            print_native_assistant_output(&output)?;
                        }
                    }
                    Err(error) => {
                        print_native_error(&error)?;
                    }
                }
                print_native_pending_approvals(&runtime)?;
                print_native_open_questions(&runtime)?;
                return Ok(runtime);
            }
            Err(TryRecvError::Empty) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(TryRecvError::Disconnected) => {
                finish_native_stream_line(render_state)?;
                return Err(anyhow!("native terminal worker disconnected"));
            }
        }
    }
}

fn drain_native_progress(
    progress_rx: &Receiver<RuntimeProgress>,
    render_state: &mut NativeRenderState,
) -> io::Result<()> {
    while let Ok(event) = progress_rx.try_recv() {
        render_native_progress(event, render_state)?;
    }
    Ok(())
}

fn render_native_progress(
    event: RuntimeProgress,
    render_state: &mut NativeRenderState,
) -> io::Result<()> {
    match event {
        RuntimeProgress::AssistantDelta { delta } => {
            let Some(delta) = render_state.sanitize_assistant_delta(&delta) else {
                return Ok(());
            };
            if !render_state.assistant_open {
                println!("{}", native_assistant_label());
                render_state.assistant_open = true;
            }
            print!("{delta}");
            io::stdout().flush()?;
        }
        other => {
            for line in native_progress_lines(&other, render_state) {
                finish_native_stream_line(render_state)?;
                print_native_progress_line(&line);
            }
        }
    }
    Ok(())
}

fn native_progress_lines(
    event: &RuntimeProgress,
    _render_state: &mut NativeRenderState,
) -> Vec<String> {
    match event {
        RuntimeProgress::AssistantDelta { .. }
        | RuntimeProgress::ProviderStreamStarted
        | RuntimeProgress::ProviderTurnStarted { .. }
        | RuntimeProgress::ProviderTurnCompleted { .. }
        | RuntimeProgress::ToolStarted { .. }
        | RuntimeProgress::ToolBatchCompleted { .. } => Vec::new(),
        RuntimeProgress::ToolCompleted { tool, ok, summary } => {
            if *ok {
                return Vec::new();
            }
            let tool = terminal_safe_text(tool);
            let detail = native_progress_detail(summary)
                .map(|summary| format!(": {summary}"))
                .unwrap_or_default();
            vec![format!("error  {}{detail}", tool.trim())]
        }
    }
}

fn native_progress_detail(value: &str) -> Option<String> {
    let safe_value = terminal_safe_text(value);
    let detail = safe_value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    Some(compact_native_progress_detail(detail))
}

fn compact_native_progress_detail(value: &str) -> String {
    let mut compacted = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= NATIVE_PROGRESS_DETAIL_CHARS {
            compacted.push_str("...");
            return compacted;
        }
        compacted.push(ch);
    }
    compacted
}

fn finish_native_stream_line(render_state: &mut NativeRenderState) -> io::Result<()> {
    if render_state.assistant_open {
        if !render_state.assistant_ended_with_newline {
            println!();
        }
        println!();
        io::stdout().flush()?;
        render_state.assistant_open = false;
        render_state.assistant_ended_with_newline = false;
    }
    render_state.assistant_sanitizer.reset();
    Ok(())
}

fn print_native_assistant_output(output: &str) -> io::Result<()> {
    let output = terminal_safe_text(output);
    let output = output.trim_end_matches('\n');
    if output.is_empty() {
        return Ok(());
    }
    println!("{}", native_assistant_label());
    println!("{output}");
    println!();
    io::stdout().flush()
}

fn print_native_error(error: &str) -> io::Result<()> {
    println!("{NATIVE_ERROR_LABEL}  {}", terminal_safe_text(error));
    println!();
    io::stdout().flush()
}

fn print_native_progress_line(line: &str) {
    let line = terminal_safe_text(line);
    if let Some(message) = line.strip_prefix("error  ") {
        println!("{NATIVE_ERROR_LABEL}  {message}");
    } else {
        println!("{line}");
    }
}

fn should_print_native_task_output(saw_assistant_delta: bool, state: &str) -> bool {
    !saw_assistant_delta || matches!(state, "AwaitingApproval" | "WaitingUser" | "Failed")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSideQuestionAnswer {
    message: String,
    continue_planning: bool,
}

fn answer_native_side_question(
    runtime: &mut AgentRuntime,
    input: &str,
) -> Result<Option<NativeSideQuestionAnswer>> {
    let answer = input.trim();
    if answer.is_empty() || answer.starts_with('/') {
        return Ok(None);
    }
    let monitor = runtime.session_monitor()?;
    let Some(question) = monitor.open_questions.first() else {
        return Ok(None);
    };
    let resolved_answer = native_side_question_answer(question, answer);
    let message = runtime.answer_current_side_question(&question.id, &resolved_answer)?;
    let remaining = monitor.open_questions.len().saturating_sub(1);
    if remaining == 0 {
        Ok(Some(NativeSideQuestionAnswer {
            message: format!("{message}\ndeepcli | plan interview answered"),
            continue_planning: true,
        }))
    } else {
        Ok(Some(NativeSideQuestionAnswer {
            message: format!(
                "{message}\ndeepcli | plan interview answered | remaining {remaining}"
            ),
            continue_planning: false,
        }))
    }
}

fn print_native_open_questions(runtime: &AgentRuntime) -> Result<()> {
    let monitor = runtime.session_monitor()?;
    for line in native_open_question_lines(&monitor.open_questions) {
        println!("{}", terminal_safe_text(&line));
    }
    Ok(())
}

fn print_native_pending_approvals(runtime: &AgentRuntime) -> Result<()> {
    let monitor = runtime.session_monitor()?;
    for line in native_pending_approval_lines(&monitor.pending_approvals) {
        println!("{}", terminal_safe_text(&line));
    }
    Ok(())
}

fn native_pending_approval_lines(approvals: &[SessionObservationApproval]) -> Vec<String> {
    if approvals.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "deepcli | pending tool approvals | requests {}",
        approvals.len()
    )];
    for approval in approvals {
        let short_id = approval.id.chars().take(8).collect::<String>();
        lines.push(format!(
            "approval {short_id} | {} | {} | {}",
            terminal_safe_text(&approval.tool),
            terminal_safe_text(&approval.risk),
            terminal_safe_text(&approval.reason)
        ));
    }
    lines
}

fn native_open_question_lines(questions: &[SessionObservationQuestion]) -> Vec<String> {
    if questions.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "deepcli | waiting for plan interview answer | questions {}",
        questions.len()
    )];
    for (question_index, question) in questions.iter().enumerate() {
        let label = if questions.len() == 1 {
            "plan question".to_string()
        } else {
            format!("plan question {}", question_index + 1)
        };
        lines.push(format!(
            "{label}: {}",
            terminal_safe_text(&question.question)
        ));
        for (option_index, option) in question.options.iter().enumerate() {
            lines.push(format!(
                "  {}. {}",
                option_index + 1,
                terminal_safe_text(option)
            ));
        }
        if !question.options.is_empty() {
            lines.push(format!(
                "  {}. 自定义输入（直接输入文本）",
                question.options.len() + 1
            ));
        }
    }
    lines.push("deepcli | answer with option number or free text".to_string());
    lines
}

fn native_side_question_answer(question: &SessionObservationQuestion, input: &str) -> String {
    let trimmed = input.trim();
    if let Ok(index) = trimmed.parse::<usize>() {
        if (1..=question.options.len()).contains(&index) {
            return question.options[index - 1].clone();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::runtime::RuntimeOptions;
    use crate::session::SessionStore;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tempfile::tempdir;

    #[test]
    fn native_provider_transport_progress_is_hidden() {
        let mut render_state = NativeRenderState::default();
        let stream_started = RuntimeProgress::ProviderStreamStarted;
        let started = RuntimeProgress::ProviderTurnStarted {
            iteration: 2,
            message_count: 14,
            tool_count: 9,
            request_kib: 128,
            compacted: true,
        };
        let completed = RuntimeProgress::ProviderTurnCompleted {
            elapsed_ms: 1250,
            tool_calls: 3,
        };

        assert!(native_progress_lines(&stream_started, &mut render_state).is_empty());
        assert!(native_progress_lines(&started, &mut render_state).is_empty());
        assert!(native_progress_lines(&completed, &mut render_state).is_empty());
    }

    #[test]
    fn native_routine_tool_progress_is_hidden() {
        let mut render_state = NativeRenderState::default();
        let started = RuntimeProgress::ToolStarted {
            tool: "read_file".to_string(),
            detail: Some("# deepcli 架构".to_string()),
        };
        let completed = RuntimeProgress::ToolCompleted {
            tool: "read_file".to_string(),
            ok: true,
            summary: "[deepcli read_file slice: lines 1-80 of 5671]".to_string(),
        };
        let completed_batch = RuntimeProgress::ToolBatchCompleted { tool_calls: 1 };

        assert!(native_progress_lines(&started, &mut render_state).is_empty());
        assert!(native_progress_lines(&completed, &mut render_state).is_empty());
        assert!(native_progress_lines(&completed_batch, &mut render_state).is_empty());
    }

    #[test]
    fn native_tool_failures_remain_actionable_without_batch_telemetry() {
        let mut render_state = NativeRenderState::default();
        let failed = RuntimeProgress::ToolCompleted {
            tool: "run_shell\x1b[31m".to_string(),
            ok: false,
            summary: "permission denied\x1b[0m\nignored detail".to_string(),
        };

        assert_eq!(
            native_progress_lines(&failed, &mut render_state),
            vec!["error  run_shell: permission denied".to_string()]
        );
    }

    #[test]
    fn native_open_question_lines_include_options_and_custom_input() {
        let question = SessionObservationQuestion {
            id: "question-id".to_string(),
            question: "先验证现有模块还是直接实现 runner？".to_string(),
            options: vec!["先验证".to_string(), "直接实现 runner".to_string()],
        };

        let lines = native_open_question_lines(&[question]);

        assert_eq!(
            lines,
            vec![
                "deepcli | waiting for plan interview answer | questions 1".to_string(),
                "plan question: 先验证现有模块还是直接实现 runner？".to_string(),
                "  1. 先验证".to_string(),
                "  2. 直接实现 runner".to_string(),
                "  3. 自定义输入（直接输入文本）".to_string(),
                "deepcli | answer with option number or free text".to_string(),
            ]
        );
    }

    #[test]
    fn terminal_safe_text_removes_terminal_control_sequences() {
        let value = concat!(
            "plain\x1b[31mred\x1b[0m\n",
            "tab\tvalue\x1b]52;c;YXR0YWNr\x07done",
            "\x08\r\x7f\u{009b}2Jvisible"
        );

        assert_eq!(terminal_safe_text(value), "plainred\ntab\tvaluedonevisible");
    }

    #[test]
    fn terminal_sanitizer_handles_sequences_split_across_stream_deltas() {
        let mut sanitizer = TerminalTextSanitizer::default();

        assert_eq!(sanitizer.sanitize_chunk("before\x1b]52;c;"), "before");
        assert_eq!(sanitizer.sanitize_chunk("YXR0YWNr\x1b\\after"), "after");
        assert_eq!(sanitizer.sanitize_chunk("\x1b[3"), "");
        assert_eq!(sanitizer.sanitize_chunk("1mred"), "red");
    }

    #[test]
    fn native_assistant_state_ignores_control_only_deltas_and_tracks_newlines() {
        let mut state = NativeRenderState::default();

        assert_eq!(state.sanitize_assistant_delta("\x1b[31m"), None);
        assert!(!state.saw_assistant_delta);
        assert!(!state.assistant_ended_with_newline);

        assert_eq!(
            state.sanitize_assistant_delta("answer\n"),
            Some("answer\n".to_string())
        );
        assert!(state.saw_assistant_delta);
        assert!(state.assistant_ended_with_newline);
    }

    #[test]
    fn native_paste_is_normalized_before_editor_metrics_and_rendering() {
        let pasted = normalize_native_paste("one\r\n\x1b[2Jtwo\r\x1b]52;c;YXR0YWNr\x07three\x08");
        assert_eq!(pasted, "one\ntwo\nthree");

        let mut editor = NativeInputEditor::default();
        editor.insert_str(&pasted);
        assert_eq!(editor.buffer(), "one\ntwo\nthree");
        assert_eq!(render_input_buffer(editor.buffer()), "one\r\ntwo\r\nthree");

        let metrics = native_input_metrics(editor.buffer(), editor.cursor(), 80);
        assert_eq!(metrics.total_rows, 3);
        assert_eq!(metrics.cursor_row, 2);
        assert_eq!(metrics.cursor_col, 5);
    }

    #[test]
    fn native_editor_drops_control_sequences_even_without_paste_path() {
        let mut editor = NativeInputEditor::default();
        editor.insert_str("\x1b[31mred\x1b[0m\x08");

        assert_eq!(editor.buffer(), "red");
        assert_eq!(editor.cursor(), 3);
    }

    #[test]
    fn native_task_prints_terminal_states_after_streaming() {
        assert!(!should_print_native_task_output(true, "Running"));
        assert!(should_print_native_task_output(true, "AwaitingApproval"));
        assert!(should_print_native_task_output(true, "WaitingUser"));
        assert!(should_print_native_task_output(true, "Failed"));
        assert!(should_print_native_task_output(false, "Completed"));
    }

    #[test]
    fn native_question_and_approval_lines_sanitize_untrusted_fields() {
        let question = SessionObservationQuestion {
            id: "question-id".to_string(),
            question: "choose\x1b[2J now".to_string(),
            options: vec!["safe\x1b]52;c;YXR0YWNr\x07 option".to_string()],
        };
        let approval = SessionObservationApproval {
            id: "12345678-0000-0000-0000-000000000000".to_string(),
            tool: "run_shell\x1b[31m".to_string(),
            risk: "High\x1b[0m".to_string(),
            reason: "reason\x08".to_string(),
        };

        let question_lines = native_open_question_lines(&[question]);
        assert!(question_lines.iter().all(|line| !line.contains('\x1b')));
        assert!(question_lines
            .iter()
            .any(|line| line.contains("choose now")));
        assert!(question_lines
            .iter()
            .any(|line| line.contains("safe option")));

        assert_eq!(
            native_pending_approval_lines(&[approval]),
            vec![
                "deepcli | pending tool approvals | requests 1".to_string(),
                "approval 12345678 | run_shell | High | reason".to_string(),
            ]
        );
    }

    #[test]
    fn native_side_question_answer_maps_option_numbers() {
        let question = SessionObservationQuestion {
            id: "question-id".to_string(),
            question: "选择路线".to_string(),
            options: vec!["路线 A".to_string(), "路线 B".to_string()],
        };

        assert_eq!(native_side_question_answer(&question, "2"), "路线 B");
        assert_eq!(native_side_question_answer(&question, "3"), "3");
        assert_eq!(
            native_side_question_answer(&question, "自定义路线"),
            "自定义路线"
        );
    }

    #[test]
    fn native_answer_last_plan_question_requests_continuation() {
        let dir = tempdir().unwrap();
        let mut runtime = AgentRuntime::new(
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
        let session = SessionStore::new(dir.path())
            .load(&runtime.session_id())
            .unwrap();
        session
            .enqueue_side_question_with_options(
                "优先增强哪个 plan 方向？",
                vec!["JSON 输出".to_string(), "质量校验".to_string()],
            )
            .unwrap();

        let outcome = answer_native_side_question(&mut runtime, "2")
            .unwrap()
            .unwrap();

        assert!(outcome.continue_planning);
        assert!(outcome.message.contains("plan interview answered"));
        assert!(!outcome.message.contains("btw"));
        let session = SessionStore::new(dir.path())
            .load(&runtime.session_id())
            .unwrap();
        let answered = session.load_side_questions().unwrap();
        assert_eq!(answered[0].answer.as_deref(), Some("质量校验"));
    }

    #[test]
    fn native_conversation_chrome_uses_compact_role_labels() {
        let banner = native_session_banner("d2deb496-9a30-44a1-8785-e9b0cdcf00f3");
        let prompt = native_input_prompt();
        let assistant = native_assistant_label();

        assert_eq!(strip_ansi_for_test(&banner), "deepcli  session d2deb496");
        assert_eq!(strip_ansi_for_test(prompt), "you  ");
        assert_eq!(strip_ansi_for_test(assistant), "deepcli");
        assert!(banner.contains("\x1b["));
        assert!(prompt.contains("\x1b["));
        assert!(assistant.contains("\x1b["));
        assert!(!strip_ansi_for_test(prompt).contains('>'));
    }

    #[test]
    fn native_input_editor_moves_left_and_right_by_character() {
        let mut editor = NativeInputEditor::default();
        editor.insert_str("abc");

        assert_eq!(editor.cursor(), 3);
        editor.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(editor.cursor(), 2);
        editor.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(editor.cursor(), 3);
    }

    #[test]
    fn native_input_editor_moves_up_and_down_between_lines() {
        let mut editor = NativeInputEditor::default();
        editor.insert_str("abc\ndefg\nhi");
        editor.set_cursor("abc\ndef".len());

        editor.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(editor.cursor(), "abc".len());

        editor.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(editor.cursor(), "abc\ndef".len());
    }

    #[test]
    fn native_input_editor_up_on_first_line_moves_to_line_start() {
        let mut editor = NativeInputEditor::default();
        editor.insert_str("abc\ndef");
        editor.set_cursor(2);

        editor.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(editor.cursor(), 0);
    }

    #[test]
    fn native_input_editor_submits_raw_newline_char_from_pty_pipe() {
        let mut editor = NativeInputEditor::default();
        editor.insert_str("/quit");

        assert_eq!(
            editor.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            NativeInputAction::Submitted("/quit".to_string())
        );
        assert_eq!(
            editor.handle_key(KeyEvent::new(KeyCode::Char('\n'), KeyModifiers::NONE)),
            NativeInputAction::Submitted("/quit".to_string())
        );
        assert_eq!(
            editor.handle_key(KeyEvent::new(KeyCode::Char('\r'), KeyModifiers::NONE)),
            NativeInputAction::Submitted("/quit".to_string())
        );
    }

    fn strip_ansi_for_test(value: &str) -> String {
        let mut stripped = String::new();
        let mut chars = value.chars();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            } else {
                stripped.push(ch);
            }
        }
        stripped
    }
}
