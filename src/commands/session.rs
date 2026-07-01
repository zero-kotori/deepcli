use super::*;
use anyhow::{bail, Result};

pub(crate) fn handle_session(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let store = SessionStore::new(workspace);
    match args.first().map(String::as_str) {
        None => handle_session_default_list(&store),
        Some("list") => handle_session_list(workspace, &store, &args[1..]),
        Some("search") => handle_session_search(workspace, &store, &args[1..]),
        Some("next") => handle_session_next(workspace, &store, current, &args[1..]),
        Some("diagnose") => handle_session_diagnose(workspace, &store, current, &args[1..]),
        Some("rename") => handle_session_rename(&store, current, &args[1..]),
        Some("prune-empty") | Some("prune") => {
            handle_session_prune_empty(workspace, &store, current.as_deref(), &args[1..])
        }
        Some("show") => handle_session_show(workspace, &store, current, &args[1..]),
        Some("history") => handle_session_history(workspace, &store, current, &args[1..]),
        Some("summary") => handle_session_summary(workspace, &store, current, &args[1..]),
        Some("tools") => handle_session_tools(workspace, &store, current, &args[1..]),
        Some("tests") => handle_session_tests(workspace, &store, current, &args[1..]),
        Some("diffs") | Some("diff") => {
            handle_session_diffs(workspace, &store, current, &args[1..])
        }
        Some("backups") | Some("backup") => {
            handle_session_backups(workspace, &store, current, &args[1..])
        }
        Some("export") => handle_session_export(workspace, &store, current, &args[1..]),
        Some(other) => bail!("unsupported /session action `{other}`"),
    }
}

pub(crate) async fn handle_session_command(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    if matches!(
        args.first().map(String::as_str),
        Some("restore-backup" | "restore")
    ) {
        return handle_restore_backup(workspace, current, executor, &args[1..]).await;
    }
    handle_session(workspace, current, args)
}
