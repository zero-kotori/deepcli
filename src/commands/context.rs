use crate::workspace::{FileSummary, WorkspaceManager};
use anyhow::Result;
use std::path::Path;

pub(super) fn handle_context(workspace: &Path) -> Result<String> {
    let manager = WorkspaceManager::new(workspace)?;
    let context = manager.collect_context()?;
    Ok(format!(
        "workspace: {}\nagents: {}\nreadme: {}\ndocs: {}\ngit diff present: {}",
        context.root.display(),
        format_files(workspace, &context.agents_files),
        format_files(workspace, &context.readme_files),
        format_files(workspace, &context.docs_files),
        context.git_diff_present
    ))
}

fn format_files(workspace: &Path, files: &[FileSummary]) -> String {
    if files.is_empty() {
        "<none>".to_string()
    } else {
        files
            .iter()
            .map(|file| {
                file.path
                    .strip_prefix(workspace)
                    .unwrap_or(&file.path)
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}
