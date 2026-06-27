use anyhow::{bail, Result};
use similar::TextDiff;
use std::path::{Component, Path, PathBuf};

pub fn resolve_workspace_path(workspace: &Path, raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("path traversal is not allowed: {raw}");
    }
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    if !resolved.starts_with(workspace) {
        bail!("path is outside workspace: {}", resolved.display());
    }
    Ok(resolved)
}

pub(super) fn normalize_patch_input(patch: &str, path: Option<&str>) -> String {
    let patch = if patch.trim_start().starts_with("@@") {
        if let Some(path) = path {
            format!("--- a/{path}\n+++ b/{path}\n{patch}")
        } else {
            patch.to_string()
        }
    } else {
        patch.to_string()
    };
    normalize_unified_diff_hunk_counts(&patch)
}

pub(super) fn normalize_unified_diff_hunk_counts(patch: &str) -> String {
    let mut output = Vec::new();
    let mut active_hunk: Option<HunkAccumulator> = None;

    for line in patch.lines() {
        if let Some(header) = parse_hunk_header(line) {
            if let Some(hunk) = active_hunk.take() {
                output.extend(hunk.into_lines());
            }
            active_hunk = Some(HunkAccumulator {
                header,
                body: Vec::new(),
            });
            continue;
        }

        if let Some(hunk) = active_hunk.as_mut() {
            if line.is_empty() {
                hunk.body.push(" ".to_string());
            } else {
                hunk.body.push(line.to_string());
            }
        } else {
            output.push(line.to_string());
        }
    }

    if let Some(hunk) = active_hunk {
        output.extend(hunk.into_lines());
    }

    let mut normalized = output.join("\n");
    if patch.ends_with('\n') {
        normalized.push('\n');
    }
    normalized
}

#[derive(Debug)]
struct HunkAccumulator {
    header: HunkHeader,
    body: Vec<String>,
}

impl HunkAccumulator {
    fn into_lines(self) -> Vec<String> {
        let mut old_count = 0usize;
        let mut new_count = 0usize;
        for line in &self.body {
            match line.as_bytes().first().copied() {
                Some(b' ') => {
                    old_count += 1;
                    new_count += 1;
                }
                Some(b'-') => old_count += 1,
                Some(b'+') => new_count += 1,
                _ => {}
            }
        }
        let mut lines = vec![format!(
            "@@ -{},{} +{},{} @@{}",
            self.header.old_start, old_count, self.header.new_start, new_count, self.header.suffix
        )];
        lines.extend(self.body);
        lines
    }
}

#[derive(Debug)]
struct HunkHeader {
    old_start: usize,
    new_start: usize,
    suffix: String,
}

fn parse_hunk_header(line: &str) -> Option<HunkHeader> {
    let rest = line.strip_prefix("@@ -")?;
    let (old_start, rest) = parse_hunk_start(rest)?;
    let rest = rest.strip_prefix(" +")?;
    let (new_start, rest) = parse_hunk_start(rest)?;
    let suffix = rest.strip_prefix(" @@")?;
    Some(HunkHeader {
        old_start,
        new_start,
        suffix: suffix.to_string(),
    })
}

fn parse_hunk_start(input: &str) -> Option<(usize, &str)> {
    let digits = input
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    let start = input[..digits].parse::<usize>().ok()?;
    let rest = &input[digits..];
    if let Some(rest) = rest.strip_prefix(',') {
        let count_digits = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if count_digits == 0 {
            return None;
        }
        Some((start, &rest[count_digits..]))
    } else {
        Some((start, rest))
    }
}

pub(super) fn reject_placeholder_overwrite(path: &Path, before: &str, content: &str) -> Result<()> {
    if before.len() < 1024 || content.len() * 10 >= before.len() {
        return Ok(());
    }
    if !looks_like_placeholder_content(content) {
        return Ok(());
    }
    bail!(
        "refusing to overwrite existing large file {} with placeholder-like content",
        path.display()
    )
}

pub(super) fn reject_large_destructive_rewrite(
    path: &Path,
    before: &str,
    content: &str,
) -> Result<()> {
    if before.len() < 8 * 1024 || content.len() * 100 >= before.len() * 80 {
        return Ok(());
    }
    bail!(
        "refusing to overwrite existing large file {} with much shorter content; use a unified diff patch instead",
        path.display()
    )
}

pub(super) fn reject_large_existing_rewrite(
    path: &Path,
    before: &str,
    content: &str,
) -> Result<()> {
    if before.len() < 8 * 1024 || before == content {
        return Ok(());
    }
    bail!(
        "refusing to rewrite existing large file {} with full file content; use a unified diff patch instead",
        path.display()
    )
}

fn looks_like_placeholder_content(content: &str) -> bool {
    let normalized = content.trim().to_ascii_lowercase();
    let stripped = normalized
        .trim_start_matches("//")
        .trim_start_matches('#')
        .trim_start_matches("/*")
        .trim_end_matches("*/")
        .trim();
    stripped == "placeholder"
        || stripped == "todo"
        || stripped == "..."
        || stripped == "<omitted>"
        || stripped == "omitted"
        || stripped.contains("placeholder")
        || stripped.contains("content omitted")
}

pub(super) fn slice_text_by_line(content: &str, start_line: usize, limit: Option<usize>) -> String {
    if start_line <= 1 && limit.is_none() {
        return content.to_string();
    }
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let start_index = start_line.saturating_sub(1).min(lines.len());
    let end_index = match limit {
        Some(limit) => start_index.saturating_add(limit).min(lines.len()),
        None => lines.len(),
    };
    let mut selected = lines[start_index..end_index].join("\n");
    if content.ends_with('\n') && end_index == lines.len() {
        selected.push('\n');
    }
    if start_index > 0 || end_index < lines.len() {
        format!(
            "[deepcli read_file slice: lines {}-{} of {}]\n{}",
            start_index + 1,
            end_index,
            lines.len(),
            selected
        )
    } else {
        selected
    }
}

pub(super) fn unified_diff(before: &str, after: &str, path: &Path) -> String {
    TextDiff::from_lines(before, after)
        .unified_diff()
        .header(
            &format!("a/{}", path.display()),
            &format!("b/{}", path.display()),
        )
        .to_string()
}
