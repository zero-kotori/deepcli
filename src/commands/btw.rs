use super::{
    dedup_preserve_order, local_action_checklist, parse_scoped_action_args, parse_scoped_list_args,
    prefix_session_note, required_arg, resolve_session_for_inspection,
    resolve_session_for_optional_inspection, resolve_session_for_side_question_action,
    session_activity_json, session_inspect_metadata_json, set_command_output_path, short_id,
    write_command_output, SessionFallbackKind,
};
use crate::privacy::redact_sensitive_text;
use crate::session::{Session, SessionStore, SideQuestion, SideQuestionStatus};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
struct BtwAnswerOptions {
    current_only: bool,
    json_output: bool,
    output_path: Option<String>,
    answer: String,
}

fn parse_btw_answer_args(args: &[String], usage: &str) -> Result<BtwAnswerOptions> {
    if args.is_empty() {
        bail!("usage: {usage}");
    }
    let mut current_only = false;
    let mut json_output = false;
    let mut output_path = None;
    let mut answer = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--current" => {
                current_only = true;
                index += 1;
            }
            "--json" => {
                json_output = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(&mut output_path, value.trim_start_matches("--output="))?;
                index += 1;
            }
            "--" => {
                answer.extend(args.iter().skip(index + 1).cloned());
                break;
            }
            value => {
                answer.push(value.to_string());
                index += 1;
            }
        }
    }
    Ok(BtwAnswerOptions {
        current_only,
        json_output,
        output_path,
        answer: answer.join(" "),
    })
}

pub(crate) fn handle_btw(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let store = SessionStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let options = parse_scoped_list_args(&args[1..], current, "/btw list")?;
            let fallback = if options.include_all {
                SessionFallbackKind::SideQuestions
            } else {
                SessionFallbackKind::OpenSideQuestions
            };
            let (session, note) = resolve_session_for_optional_inspection(
                &store,
                options.session_id.as_deref(),
                options.explicit_session,
                fallback,
            )?;
            let questions = session.load_side_questions()?;
            let report = prefix_session_note(
                format_side_questions(&questions, options.include_all),
                &session,
                note.clone(),
            );
            let output = if options.json_output {
                format_btw_list_json(
                    workspace,
                    &session,
                    note.as_deref(),
                    options.include_all,
                    &questions,
                    &report,
                )?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("ask") => {
            let question = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if question.trim().is_empty() {
                bail!("/btw ask requires a question");
            }
            let id = current.ok_or_else(|| anyhow::anyhow!("no active session is available"))?;
            let (session, _note) = resolve_session_for_inspection(
                &store,
                &id,
                false,
                SessionFallbackKind::RecordedActivity,
            )?;
            let item = session.enqueue_side_question(question.trim())?;
            Ok(format!(
                "queued by-the-way question {} in session {}: {}",
                short_id(&item.id),
                session.id(),
                item.question
            ))
        }
        Some("answer") => {
            let question_id = required_arg(&args, 1, "side question id")?;
            let options = parse_btw_answer_args(
                &args[2..],
                "/btw answer <id> [--current] [--json] [--output path] <answer>",
            )?;
            if options.answer.trim().is_empty() {
                bail!("/btw answer requires an answer");
            }
            let session = resolve_session_for_side_question_action(
                &store,
                current.as_deref(),
                question_id,
                options.current_only,
            )?;
            let item = session.answer_side_question(question_id, options.answer.trim())?;
            let report = format!(
                "answered by-the-way question {} in session {}",
                short_id(&item.id),
                session.id()
            );
            let output = if options.json_output {
                format_btw_action_json(workspace, &session, "answer", &item, &report)?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("clear") => {
            let options = parse_scoped_action_args(
                &args[1..],
                current,
                "/btw clear [--json] [--output path] [session_id|--current]",
            )?;
            let (session, _note) = resolve_session_for_inspection(
                &store,
                &options.session_id,
                options.explicit_session,
                SessionFallbackKind::OpenSideQuestions,
            )?;
            let cleared = session.clear_side_questions()?;
            let report = format!(
                "cleared {cleared} open by-the-way question(s) in session {}",
                session.id()
            );
            let output = if options.json_output {
                format_btw_clear_json(workspace, &session, cleared, &report)?
            } else {
                report
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(other) => bail!("unsupported /btw action `{other}`"),
    }
}

pub(super) fn format_side_questions(items: &[SideQuestion], include_all: bool) -> String {
    let rows = items
        .iter()
        .filter(|item| include_all || item.status == SideQuestionStatus::Open)
        .map(|item| {
            let mut line = format!(
                "{} [{}] {}",
                short_id(&item.id),
                side_question_status_label(&item.status),
                redact_sensitive_text(&item.question)
            );
            if let Some(answer) = &item.answer {
                line.push_str(&format!("\n  answer: {}", redact_sensitive_text(answer)));
            }
            line
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        "no by-the-way questions".to_string()
    } else {
        rows.join("\n")
    }
}

fn format_btw_list_json(
    workspace: &Path,
    session: &Session,
    note: Option<&str>,
    include_all: bool,
    questions: &[SideQuestion],
    report: &str,
) -> Result<String> {
    let items = questions
        .iter()
        .filter(|item| include_all || item.status == SideQuestionStatus::Open)
        .map(side_question_json)
        .collect::<Vec<_>>();
    let activity = session.activity_summary()?;
    let next_actions = btw_list_next_actions(session, include_all);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.btw.list.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "note": note,
        "includeAll": include_all,
        "session": session_inspect_metadata_json(session),
        "activity": session_activity_json(&activity),
        "itemCount": items.len(),
        "totalCount": questions.len(),
        "openCount": questions
            .iter()
            .filter(|item| item.status == SideQuestionStatus::Open)
            .count(),
        "questions": items,
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
    }))?)
}

fn btw_list_next_actions(session: &Session, include_all: bool) -> Vec<String> {
    let session_id = session.id().to_string();
    let mut actions = vec![format!("deepcli btw list {session_id} --json")];
    if !include_all {
        actions.push(format!("deepcli btw list {session_id} --all --json"));
    }
    actions.push("deepcli help btw".to_string());
    dedup_preserve_order(actions)
}

fn btw_action_next_actions(session: &Session) -> Vec<String> {
    let session_id = session.id().to_string();
    vec![
        format!("deepcli btw list {session_id} --json"),
        format!("deepcli btw list {session_id} --all --json"),
        "deepcli help btw".to_string(),
    ]
}

fn format_btw_action_json(
    workspace: &Path,
    session: &Session,
    action: &str,
    item: &SideQuestion,
    report: &str,
) -> Result<String> {
    let next_actions = btw_action_next_actions(session);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.btw.action.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "action": action,
        "session": session_inspect_metadata_json(session),
        "question": side_question_json(item),
        "clearedCount": Value::Null,
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
    }))?)
}

fn format_btw_clear_json(
    workspace: &Path,
    session: &Session,
    cleared: usize,
    report: &str,
) -> Result<String> {
    let next_actions = btw_action_next_actions(session);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.btw.action.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "action": "clear",
        "session": session_inspect_metadata_json(session),
        "question": Value::Null,
        "clearedCount": cleared,
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
    }))?)
}

fn side_question_json(item: &SideQuestion) -> Value {
    json!({
        "id": item.id.to_string(),
        "shortId": short_id(&item.id),
        "status": &item.status,
        "question": redact_sensitive_text(&item.question),
        "answer": item.answer.as_deref().map(redact_sensitive_text),
        "createdAt": &item.created_at,
        "updatedAt": &item.updated_at,
    })
}

fn side_question_status_label(status: &SideQuestionStatus) -> &'static str {
    match status {
        SideQuestionStatus::Open => "open",
        SideQuestionStatus::Answered => "answered",
        SideQuestionStatus::Cleared => "cleared",
    }
}
