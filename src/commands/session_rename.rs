use super::short_id;
use crate::session::SessionStore;
use anyhow::{bail, Result};

pub(crate) fn handle_session_rename(
    store: &SessionStore,
    current: Option<String>,
    args: &[String],
) -> Result<String> {
    let (id, title) = parse_session_rename_args(args, current)?;
    let mut session = store.load(&id)?;
    session.rename(&title)?;
    Ok(format!(
        "renamed session id={} full={} title={}",
        short_id(&session.id()),
        session.id(),
        title
    ))
}

fn parse_session_rename_args(args: &[String], current: Option<String>) -> Result<(String, String)> {
    if args.len() < 2 {
        bail!("usage: /session rename <session_id|--current> <title>");
    }
    let id = if args[0] == "--current" {
        current.ok_or_else(|| anyhow::anyhow!("no active session is available"))?
    } else {
        args[0].clone()
    };
    let title = args[1..].join(" ").trim().to_string();
    if title.is_empty() {
        bail!("session title cannot be empty");
    }
    Ok((id, title))
}
