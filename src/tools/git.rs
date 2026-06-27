use anyhow::{bail, Result};

pub(super) fn validate_branch_name(name: &str) -> Result<()> {
    if name.starts_with('-')
        || name.contains("..")
        || name.contains('@')
        || name.contains('\\')
        || name.contains(' ')
        || name.trim().is_empty()
    {
        bail!("invalid branch name `{name}`");
    }
    Ok(())
}

pub(super) fn generate_commit_message(status: &str, changed_files: &str) -> String {
    let files = changed_files
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let status_lines = status
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let file_count = files.len().max(status_lines.len());

    if file_count == 0 {
        return "chore: record workspace state".to_string();
    }

    let joined = files
        .iter()
        .chain(status_lines.iter())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let scope = if joined.contains("test") {
        "test"
    } else if joined.contains("doc") || joined.contains("README") {
        "docs"
    } else if joined.contains("Cargo.toml") || joined.contains("Cargo.lock") {
        "build"
    } else {
        "cli"
    };

    let verb = if status_lines.iter().any(|line| line.starts_with("A ")) {
        "add"
    } else if status_lines.iter().any(|line| line.starts_with("D ")) {
        "remove"
    } else {
        "update"
    };

    format!(
        "{scope}: {verb} {file_count} workspace file{}",
        if file_count == 1 { "" } else { "s" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_commit_message_from_changed_files() {
        let message = generate_commit_message("A  src/main.rs\n", "src/main.rs\nCargo.toml\n");
        assert_eq!(message, "build: add 2 workspace files");
        let docs =
            generate_commit_message("M  docs/ai/REQUIREMENTS.md\n", "docs/ai/REQUIREMENTS.md\n");
        assert_eq!(docs, "docs: update 1 workspace file");
    }
}
