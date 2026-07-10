use anyhow::{bail, Context, Result};
use similar::TextDiff;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

pub fn resolve_workspace_path(workspace: &Path, raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("path traversal is not allowed: {raw}");
    }
    let workspace = workspace
        .canonicalize()
        .with_context(|| format!("failed to canonicalize workspace {}", workspace.display()))?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    let resolved = canonicalize_nearest_existing_ancestor(&candidate)?;
    if !resolved.starts_with(workspace) {
        bail!("path is outside workspace: {}", resolved.display());
    }
    Ok(resolved)
}

fn canonicalize_nearest_existing_ancestor(path: &Path) -> Result<PathBuf> {
    let mut ancestor = path.to_path_buf();
    let mut missing_tail = Vec::<OsString>::new();

    loop {
        match fs::symlink_metadata(&ancestor) {
            Ok(_) => {
                let mut resolved = ancestor.canonicalize().with_context(|| {
                    format!("failed to canonicalize path {}", ancestor.display())
                })?;
                for component in missing_tail.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                let component = ancestor.file_name().ok_or_else(|| {
                    anyhow::anyhow!("failed to find an existing ancestor for {}", path.display())
                })?;
                missing_tail.push(component.to_os_string());
                if !ancestor.pop() {
                    bail!("failed to find an existing ancestor for {}", path.display());
                }
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect path {}", ancestor.display()));
            }
        }
    }
}

pub(super) fn patch_target_paths(patch: &str) -> Result<Vec<PathBuf>> {
    let mut targets = Vec::new();
    let mut remaining_hunk_lines = (0usize, 0usize);

    for (line_index, line) in patch.lines().enumerate() {
        if let Some(counts) = parse_hunk_line_counts(line) {
            remaining_hunk_lines = counts;
            continue;
        }
        if remaining_hunk_lines != (0, 0) {
            match line.as_bytes().first().copied() {
                Some(b' ') => {
                    remaining_hunk_lines.0 = remaining_hunk_lines.0.saturating_sub(1);
                    remaining_hunk_lines.1 = remaining_hunk_lines.1.saturating_sub(1);
                }
                Some(b'-') => {
                    remaining_hunk_lines.0 = remaining_hunk_lines.0.saturating_sub(1);
                }
                Some(b'+') => {
                    remaining_hunk_lines.1 = remaining_hunk_lines.1.saturating_sub(1);
                }
                _ => {}
            }
            continue;
        }
        let Some(header) = line
            .strip_prefix("--- ")
            .or_else(|| line.strip_prefix("+++ "))
        else {
            continue;
        };
        let raw_path = header
            .split_once('\t')
            .map(|(path, _)| path)
            .unwrap_or(header);
        if raw_path == "/dev/null" {
            continue;
        }
        if raw_path.starts_with('"') {
            bail!(
                "quoted patch path is not supported at line {}",
                line_index + 1
            );
        }
        let raw_path = raw_path
            .strip_prefix("a/")
            .or_else(|| raw_path.strip_prefix("b/"))
            .unwrap_or(raw_path);
        if raw_path.is_empty() {
            bail!("empty patch path at line {}", line_index + 1);
        }
        targets.push(PathBuf::from(raw_path));
    }

    Ok(targets)
}

pub(super) fn validate_patch_paths(workspace: &Path, patch: &str) -> Result<()> {
    reject_unsupported_patch_metadata(patch)?;
    let targets = patch_target_paths(patch)?;
    if targets.is_empty() {
        bail!("patch must contain at least one supported text file header");
    }
    let header_targets = targets.iter().cloned().collect::<BTreeSet<_>>();
    let diff_targets = diff_git_target_paths(patch)?;
    if !diff_targets.is_empty() && diff_targets != header_targets {
        bail!("diff --git paths must exactly match the text patch headers");
    }
    for target in targets {
        let raw = target.to_str().ok_or_else(|| {
            anyhow::anyhow!("patch path is not valid UTF-8: {}", target.display())
        })?;
        if let Err(error) = resolve_workspace_path(workspace, raw) {
            bail!("invalid patch path {}: {error}", target.display());
        }
    }
    Ok(())
}

fn reject_unsupported_patch_metadata(patch: &str) -> Result<()> {
    const UNSUPPORTED_PREFIXES: &[&str] = &[
        "GIT binary patch",
        "Binary files ",
        "rename from ",
        "rename to ",
        "copy from ",
        "copy to ",
        "similarity index ",
        "dissimilarity index ",
        "old mode ",
        "new mode ",
        "new file mode ",
        "deleted file mode ",
    ];
    if let Some(line) = patch.lines().find(|line| {
        UNSUPPORTED_PREFIXES
            .iter()
            .any(|prefix| line.starts_with(prefix))
    }) {
        bail!("unsupported patch metadata: {line}");
    }
    Ok(())
}

fn diff_git_target_paths(patch: &str) -> Result<BTreeSet<PathBuf>> {
    let mut targets = BTreeSet::new();
    for (line_index, line) in patch.lines().enumerate() {
        if !line.starts_with("diff --git ") {
            continue;
        }
        let parts = shell_words::split(line)
            .with_context(|| format!("failed to parse diff header at line {}", line_index + 1))?;
        if parts.len() != 4 || parts[0] != "diff" || parts[1] != "--git" {
            bail!("malformed diff --git header at line {}", line_index + 1);
        }
        for raw in &parts[2..] {
            let normalized = raw
                .strip_prefix("a/")
                .or_else(|| raw.strip_prefix("b/"))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "diff --git path must use an a/ or b/ prefix at line {}",
                        line_index + 1
                    )
                })?;
            targets.insert(PathBuf::from(normalized));
        }
    }
    Ok(targets)
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
    let (start, _, rest) = parse_hunk_range(input)?;
    Some((start, rest))
}

fn parse_hunk_line_counts(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@ -")?;
    let (_, old_count, rest) = parse_hunk_range(rest)?;
    let rest = rest.strip_prefix(" +")?;
    let (_, new_count, rest) = parse_hunk_range(rest)?;
    rest.strip_prefix(" @@")?;
    Some((old_count, new_count))
}

fn parse_hunk_range(input: &str) -> Option<(usize, usize, &str)> {
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
        let count = rest[..count_digits].parse::<usize>().ok()?;
        Some((start, count, &rest[count_digits..]))
    } else {
        Some((start, 1, rest))
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

#[cfg(test)]
mod tests {
    use super::{patch_target_paths, resolve_workspace_path, validate_patch_paths};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn resolve_workspace_path_rejects_parent_components() {
        let workspace = tempdir().unwrap();

        let error = resolve_workspace_path(workspace.path(), "src/../outside.txt").unwrap_err();

        assert!(error.to_string().contains("path traversal"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_workspace_path_canonicalizes_existing_target() {
        let workspace = tempdir().unwrap();
        let actual = workspace.path().join("actual");
        fs::create_dir(&actual).unwrap();
        fs::write(actual.join("note.txt"), "content").unwrap();
        symlink(&actual, workspace.path().join("alias")).unwrap();

        let resolved = resolve_workspace_path(workspace.path(), "alias/note.txt").unwrap();

        assert_eq!(resolved, actual.join("note.txt").canonicalize().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_workspace_path_canonicalizes_nearest_existing_ancestor() {
        let workspace = tempdir().unwrap();
        let actual = workspace.path().join("actual");
        fs::create_dir(&actual).unwrap();
        symlink(&actual, workspace.path().join("alias")).unwrap();

        let resolved = resolve_workspace_path(workspace.path(), "alias/new/note.txt").unwrap();

        assert_eq!(
            resolved,
            actual.canonicalize().unwrap().join("new/note.txt")
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_workspace_path_rejects_existing_target_through_escaping_symlink() {
        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        symlink(outside.path(), workspace.path().join("escape")).unwrap();

        let error = resolve_workspace_path(workspace.path(), "escape/secret.txt").unwrap_err();

        assert!(error.to_string().contains("outside workspace"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_workspace_path_rejects_new_target_through_escaping_symlink() {
        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        symlink(outside.path(), workspace.path().join("escape")).unwrap();

        let error = resolve_workspace_path(workspace.path(), "escape/new.txt").unwrap_err();

        assert!(error.to_string().contains("outside workspace"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_workspace_path_canonicalizes_symlinked_workspace() {
        let root = tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let workspace_alias = root.path().join("workspace-alias");
        fs::create_dir(&workspace).unwrap();
        symlink(&workspace, &workspace_alias).unwrap();

        let resolved = resolve_workspace_path(&workspace_alias, "new.txt").unwrap();

        assert_eq!(resolved, workspace.canonicalize().unwrap().join("new.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_workspace_path_rejects_dangling_symlink() {
        let workspace = tempdir().unwrap();
        symlink(
            workspace.path().join("missing-target"),
            workspace.path().join("dangling"),
        )
        .unwrap();

        assert!(resolve_workspace_path(workspace.path(), "dangling/new.txt").is_err());
    }

    #[test]
    fn patch_target_paths_extracts_headers_and_ignores_dev_null() {
        let patch = concat!(
            "--- a/src/lib.rs\n",
            "+++ b/src/lib.rs\n",
            "@@ -1 +1 @@\n",
            "-old\n",
            "+new\n",
            "--- /dev/null\n",
            "+++ b/new file.txt\t2026-07-10 00:00:00\n",
            "@@ -0,0 +1 @@\n",
            "+created\n"
        );

        let targets = patch_target_paths(patch).unwrap();

        assert_eq!(
            targets,
            vec![
                PathBuf::from("src/lib.rs"),
                PathBuf::from("src/lib.rs"),
                PathBuf::from("new file.txt"),
            ]
        );
    }

    #[test]
    fn patch_target_paths_ignores_header_like_hunk_content() {
        let patch = concat!(
            "--- a/src/lib.rs\n",
            "+++ b/src/lib.rs\n",
            "@@ -1 +1 @@\n",
            "--- ../old-comment\n",
            "+++ ../new-comment\n"
        );

        let targets = patch_target_paths(patch).unwrap();

        assert_eq!(
            targets,
            vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/lib.rs")]
        );
    }

    #[test]
    fn validate_patch_paths_rejects_parent_traversal() {
        let workspace = tempdir().unwrap();
        let patch = "--- a/src/lib.rs\n+++ b/../outside.rs\n@@ -1 +1 @@\n-old\n+new\n";

        let error = validate_patch_paths(workspace.path(), patch).unwrap_err();

        assert!(error.to_string().contains("path traversal"));
    }

    #[test]
    fn validate_patch_paths_accepts_safe_new_file() {
        let workspace = tempdir().unwrap();
        let patch = "--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1 @@\n+new\n";

        validate_patch_paths(workspace.path(), patch).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn validate_patch_paths_rejects_target_through_escaping_symlink() {
        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        symlink(outside.path(), workspace.path().join("escape")).unwrap();
        let patch = "--- a/escape/secret.txt\n+++ b/safe.txt\n@@ -1 +1 @@\n-secret\n+safe\n";

        let error = validate_patch_paths(workspace.path(), patch).unwrap_err();

        assert!(error.to_string().contains("outside workspace"));
    }

    #[test]
    fn patch_target_paths_rejects_quoted_git_paths() {
        let patch = "--- \"a/src/name\\t.rs\"\n+++ b/src/name.rs\n@@ -1 +1 @@\n-old\n+new\n";

        let error = patch_target_paths(patch).unwrap_err();

        assert!(error.to_string().contains("quoted patch path"));
    }

    #[test]
    fn validate_patch_paths_rejects_non_text_and_headerless_patches() {
        let workspace = tempdir().unwrap();
        for patch in [
            "diff --git a/secret.bin b/secret.bin\nGIT binary patch\nliteral 0\n",
            "diff --git a/old.txt b/new.txt\nrename from old.txt\nrename to new.txt\n",
            "diff --git a/script.sh b/script.sh\nold mode 100644\nnew mode 100755\n",
            "diff --git a/note.txt b/note.txt\nindex 1..2 100644\n",
        ] {
            assert!(
                validate_patch_paths(workspace.path(), patch).is_err(),
                "{patch}"
            );
        }
    }

    #[test]
    fn validate_patch_paths_rejects_mismatched_git_and_text_headers() {
        let workspace = tempdir().unwrap();
        let patch = concat!(
            "diff --git a/.env b/.env\n",
            "--- a/safe.txt\n",
            "+++ b/safe.txt\n",
            "@@ -1 +1 @@\n",
            "-old\n",
            "+new\n"
        );

        let error = validate_patch_paths(workspace.path(), patch).unwrap_err();

        assert!(error.to_string().contains("must exactly match"));
    }
}
