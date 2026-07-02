use anyhow::Result;
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::Write;

const ENABLE_TUI_MOUSE_CAPTURE: &[u8] = b"\x1b[?1003l\x1b[?1015l\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const DISABLE_TUI_MOUSE_CAPTURE: &[u8] = b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TuiTerminalCommand {
    EnterAlternateScreen,
    EnableMouseScrollCapture,
    EnableBracketedPaste,
    LeaveAlternateScreen,
    DisableMouseScrollCapture,
    DisableBracketedPaste,
}

const TUI_TERMINAL_SETUP_COMMANDS: &[TuiTerminalCommand] = &[
    TuiTerminalCommand::EnterAlternateScreen,
    TuiTerminalCommand::EnableMouseScrollCapture,
    TuiTerminalCommand::EnableBracketedPaste,
];

const TUI_TERMINAL_TEARDOWN_COMMANDS: &[TuiTerminalCommand] = &[
    TuiTerminalCommand::LeaveAlternateScreen,
    TuiTerminalCommand::DisableMouseScrollCapture,
    TuiTerminalCommand::DisableBracketedPaste,
];

pub(super) fn tui_terminal_setup_commands() -> &'static [TuiTerminalCommand] {
    TUI_TERMINAL_SETUP_COMMANDS
}

pub(super) fn tui_terminal_teardown_commands() -> &'static [TuiTerminalCommand] {
    TUI_TERMINAL_TEARDOWN_COMMANDS
}

pub(super) fn apply_tui_terminal_setup<W: Write>(writer: &mut W) -> Result<()> {
    for command in tui_terminal_setup_commands() {
        match command {
            TuiTerminalCommand::EnterAlternateScreen => execute!(writer, EnterAlternateScreen)?,
            TuiTerminalCommand::EnableMouseScrollCapture => {
                writer.write_all(ENABLE_TUI_MOUSE_CAPTURE)?
            }
            TuiTerminalCommand::EnableBracketedPaste => execute!(writer, EnableBracketedPaste)?,
            TuiTerminalCommand::LeaveAlternateScreen
            | TuiTerminalCommand::DisableMouseScrollCapture
            | TuiTerminalCommand::DisableBracketedPaste => {}
        }
    }
    Ok(())
}

pub(super) fn apply_tui_terminal_teardown<W: Write>(writer: &mut W) -> Result<()> {
    for command in tui_terminal_teardown_commands() {
        match command {
            TuiTerminalCommand::LeaveAlternateScreen => execute!(writer, LeaveAlternateScreen)?,
            TuiTerminalCommand::DisableMouseScrollCapture => {
                writer.write_all(DISABLE_TUI_MOUSE_CAPTURE)?
            }
            TuiTerminalCommand::DisableBracketedPaste => execute!(writer, DisableBracketedPaste)?,
            TuiTerminalCommand::EnterAlternateScreen
            | TuiTerminalCommand::EnableMouseScrollCapture
            | TuiTerminalCommand::EnableBracketedPaste => {}
        }
    }
    Ok(())
}
