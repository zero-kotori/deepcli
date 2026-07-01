use super::{resolve_session_for_optional_inspection, SessionFallbackKind};
use crate::session::{Session, SessionStore};
use anyhow::{bail, Result};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn handle_session_export(
    workspace: &Path,
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let (id, path, explicit) = parse_export_args(workspace, current, args)?;
    let (session, note) = resolve_session_for_optional_inspection(
        store,
        id.as_deref(),
        explicit,
        SessionFallbackKind::RecordedActivity,
    )?;
    let path = export_session(workspace, &session, path.as_deref())?;
    Ok(match note {
        Some(note) => format!(
            "exported session {} ({note}) to {}",
            session.id(),
            path.display()
        ),
        None => format!("exported session {} to {}", session.id(), path.display()),
    })
}

pub(crate) fn parse_export_args(
    workspace: &Path,
    current: Option<String>,
    args: &[String],
) -> Result<(Option<String>, Option<PathBuf>, bool)> {
    let store = SessionStore::new(workspace);
    let mut session_id = None;
    let mut explicit = false;
    let mut path = None;
    for (index, arg) in args.iter().enumerate() {
        if arg == "--current" {
            if session_id.is_some() {
                bail!("multiple session ids were provided");
            }
            session_id = Some(
                current
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
            );
            explicit = true;
            continue;
        }
        if index == 0 && session_id.is_none() {
            if let Ok(resolved) = store.resolve_id(arg) {
                session_id = Some(resolved);
                explicit = true;
                continue;
            }
        }
        if index == 0
            && workspace
                .join(".deepcli")
                .join("sessions")
                .join(arg)
                .exists()
        {
            session_id = Some(arg.clone());
            explicit = true;
            continue;
        }
        if path.is_some() {
            bail!("multiple export paths were provided");
        }
        path = Some(resolve_export_path(workspace, arg)?);
    }
    Ok((session_id.or(current), path, explicit))
}

fn resolve_export_path(workspace: &Path, raw: &str) -> Result<PathBuf> {
    let raw_path = PathBuf::from(raw);
    if raw_path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("export path must stay inside the workspace");
    }
    let path = if raw_path.is_absolute() {
        raw_path
    } else {
        workspace.join(raw_path)
    };
    if !path.starts_with(workspace) {
        bail!("export path must stay inside the workspace");
    }
    Ok(path)
}

fn export_session(workspace: &Path, session: &Session, path: Option<&Path>) -> Result<PathBuf> {
    let path = path.map(Path::to_path_buf).unwrap_or_else(|| {
        workspace
            .join(".deepcli")
            .join("exports")
            .join(format!("session-{}.json", session.id()))
    });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let export = json!({
        "metadata": &session.metadata,
        "activity": session.activity_summary()?,
        "summary": session.load_summary()?,
        "plan": session.load_plan()?,
        "messages": session.load_messages()?,
        "tools": session.load_tool_calls()?,
        "tests": session.load_test_runs()?,
        "diffs": session.load_diffs()?,
        "backups": session.load_backups()?,
        "audit": session.load_audit_events()?
    });
    fs::write(&path, serde_json::to_vec_pretty(&export)?)?;
    Ok(path)
}
