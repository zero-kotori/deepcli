use crate::commands::list_resumable_sessions;
use crate::session::SessionMetadata;
use anyhow::Result;
use std::io::{self, Write};
use std::path::Path;

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
        if let Some(session_id) = native_resume_selection(sessions, input) {
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

fn short_id(value: &str) -> &str {
    value.get(..8).unwrap_or(value)
}

fn compact_ui_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut compact = String::new();
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return compact;
        };
        compact.push(ch);
    }
    if chars.next().is_some() {
        compact.push_str("...");
    }
    compact
}
