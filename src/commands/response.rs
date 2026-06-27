use crate::tools::resolve_workspace_path;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{output}")]
pub struct CommandExit {
    pub output: String,
    pub code: u8,
}

impl CommandExit {
    pub(super) fn new(output: String, code: u8) -> Self {
        Self { output, code }
    }
}

pub(super) fn set_command_output_path(output_path: &mut Option<String>, raw: &str) -> Result<()> {
    let path = raw.trim();
    if path.is_empty() {
        bail!("--output requires a path");
    }
    let raw_path = PathBuf::from(path);
    if raw_path.is_absolute()
        || raw_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("path traversal is not allowed");
    }
    if output_path.is_some() {
        bail!("multiple output paths were provided");
    }
    *output_path = Some(path.to_string());
    Ok(())
}

pub(super) fn write_command_output(
    workspace: &Path,
    raw_path: &str,
    output: &str,
) -> Result<PathBuf> {
    let path = resolve_workspace_path(workspace, raw_path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, output).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}
