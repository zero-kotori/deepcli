use crate::session::SessionStore;
use anyhow::{bail, Result};
use std::path::Path;

pub(crate) fn handle_plan_command(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("show") => show_plan(workspace, current),
        Some(_) => bail!(
            "`/plan <requirement>` uses the model-backed planning flow and requires an active runtime session"
        ),
    }
}

fn show_plan(workspace: &Path, current: Option<String>) -> Result<String> {
    let Some(session_id) = current else {
        return Ok("no active plan".to_string());
    };
    let store = SessionStore::new(workspace);
    let session = store.load(&session_id)?;
    if let Some(document) = session.load_plan_document()? {
        return Ok(document);
    }
    if let Some(plan) = session.load_plan()? {
        return Ok(serde_json::to_string_pretty(&plan)?);
    }
    Ok("no active plan".to_string())
}
