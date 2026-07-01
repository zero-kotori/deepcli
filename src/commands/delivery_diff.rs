use super::*;
use anyhow::{bail, Result};

pub(crate) const SESSION_DIFF_FALLBACK_LIMIT: usize = 20;

pub(crate) struct SessionDiffSource {
    pub(crate) session: Session,
    pub(crate) note: Option<String>,
    pub(crate) records: Vec<SessionDiffRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffOptions {
    pub(crate) staged: bool,
    pub(crate) path_filters: Vec<String>,
    pub(crate) view: DiffView,
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffView {
    Full,
    Stat,
    NameOnly,
}

pub(crate) fn parse_diff_args(args: &[String]) -> Result<DiffOptions> {
    let mut staged = false;
    let mut path_filters = Vec::new();
    let mut view = DiffView::Full;
    let mut limit = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--staged" => {
                staged = true;
                index += 1;
            }
            "--stat" | "--summary" => {
                if view == DiffView::NameOnly {
                    bail!("choose only one /diff display mode: --stat or --name-only");
                }
                view = DiffView::Stat;
                index += 1;
            }
            "--name-only" | "--names" => {
                if view == DiffView::Stat {
                    bail!("choose only one /diff display mode: --stat or --name-only");
                }
                view = DiffView::NameOnly;
                index += 1;
            }
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = Some(parse_positive_usize(raw, "limit")?.clamp(1, 20_000));
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                let raw = value.trim_start_matches("--limit=");
                limit = Some(parse_positive_usize(raw, "limit")?.clamp(1, 20_000));
                index += 1;
            }
            "--path" | "--scope" => {
                let path = required_arg(args, index + 1, "path")?;
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 2;
            }
            value if value.starts_with("--path=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--path="),
                )?);
                index += 1;
            }
            value if value.starts_with("--scope=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--scope="),
                )?);
                index += 1;
            }
            other => bail!("unsupported /diff option `{other}`"),
        }
    }
    Ok(DiffOptions {
        staged,
        path_filters,
        view,
        limit,
    })
}

pub(crate) fn parse_review_args(args: &[String]) -> Result<Vec<String>> {
    let mut path_filters = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--path" | "--scope" => {
                let path = required_arg(args, index + 1, "path")?;
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 2;
            }
            value if value.starts_with("--path=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--path="),
                )?);
                index += 1;
            }
            value if value.starts_with("--scope=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--scope="),
                )?);
                index += 1;
            }
            other => bail!("unsupported /review option `{other}`"),
        }
    }
    Ok(path_filters)
}

pub(crate) fn normalize_scope_path_filter(raw: &str) -> Result<String> {
    let mut path = raw.trim().replace('\\', "/");
    while let Some(stripped) = path.strip_prefix("./") {
        path = stripped.to_string();
    }
    while path.ends_with('/') {
        path.pop();
    }
    if path.is_empty() || path == "." {
        bail!("--path requires a workspace-relative path");
    }
    if path.starts_with('/') {
        bail!("--path must be workspace-relative: {raw}");
    }
    if path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("--path must not contain empty, `.` or `..` segments: {raw}");
    }
    Ok(path)
}

pub(crate) fn filter_diff_by_paths(diff: &str, filters: &[String]) -> String {
    if filters.is_empty() || diff.trim().is_empty() {
        return diff.to_string();
    }

    let mut output = Vec::new();
    let mut section = Vec::new();
    let mut include_section = false;
    let mut in_section = false;

    for line in diff.lines() {
        if let Some(path) = diff_section_path_from_line(line) {
            if in_section && include_section {
                output.append(&mut section);
            } else {
                section.clear();
            }
            include_section = path_matches_verify_filters(&path, filters);
            in_section = true;
            section.push(line.to_string());
        } else if in_section {
            section.push(line.to_string());
        }
    }

    if in_section && include_section {
        output.extend(section);
    }
    output.join("\n")
}

fn diff_section_path_from_line(line: &str) -> Option<String> {
    if line.starts_with("diff --git ") || line.starts_with("diff --session ") {
        review_path_from_diff_line(line)
    } else {
        None
    }
}

fn path_matches_verify_filters(path: &str, filters: &[String]) -> bool {
    let Some(path) = normalize_diff_path_for_filter(path) else {
        return false;
    };
    filters
        .iter()
        .any(|filter| path == *filter || path.starts_with(&format!("{filter}/")))
}

pub(crate) fn normalize_diff_path_for_filter(raw: &str) -> Option<String> {
    let mut path = raw.trim().trim_matches('"').replace('\\', "/");
    if path == "/dev/null" || path.is_empty() {
        return None;
    }
    if let Some(stripped) = path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")) {
        path = stripped.to_string();
    }
    while let Some(stripped) = path.strip_prefix("./") {
        path = stripped.to_string();
    }
    Some(path)
}

pub(crate) fn format_verify_path_filters(filters: &[String]) -> String {
    filters.join(", ")
}

pub(crate) fn format_path_scope_args(filters: &[String]) -> String {
    filters
        .iter()
        .map(|filter| format!(" --path {}", shell_words::quote(filter)))
        .collect::<String>()
}

pub(crate) fn scoped_report_prefix(filters: &[String]) -> String {
    if filters.is_empty() {
        String::new()
    } else {
        format!("scope: paths={}\n", format_verify_path_filters(filters))
    }
}

pub(crate) fn format_diff_display(diff: &str, options: &DiffOptions) -> String {
    match options.view {
        DiffView::Full => limit_display_lines(diff, options.limit, "diff"),
        DiffView::Stat => format_diff_stat(diff, options.limit),
        DiffView::NameOnly => format_diff_name_only(diff, options.limit),
    }
}

pub(crate) fn format_session_diff_display(
    source: &SessionDiffSource,
    options: &DiffOptions,
) -> String {
    if options.view == DiffView::Full {
        let mut output = format_session_diff_fallback(source);
        if !options.path_filters.is_empty() {
            output.push_str(&format!(
                "\nscope: paths={}",
                format_verify_path_filters(&options.path_filters)
            ));
        }
        return limit_display_lines(&output, options.limit, "session diff");
    }

    let mut output = session_diff_fallback_header(source);
    if !options.path_filters.is_empty() {
        output.push_str(&format!(
            "\nscope: paths={}",
            format_verify_path_filters(&options.path_filters)
        ));
    }
    output.push('\n');

    let body = match options.view {
        DiffView::Full => unreachable!("full session diff view returns above"),
        DiffView::Stat => {
            format_diff_stat(&session_diff_review_input(&source.records), options.limit)
        }
        DiffView::NameOnly => {
            format_diff_name_only(&session_diff_review_input(&source.records), options.limit)
        }
    };
    output.push_str(&body);
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffFileSummary {
    pub(crate) path: String,
    added: usize,
    removed: usize,
}

pub(crate) fn diff_file_summaries(diff: &str) -> Vec<DiffFileSummary> {
    let mut summaries = Vec::new();
    let mut current: Option<DiffFileSummary> = None;

    for line in diff.lines() {
        if let Some(path) = diff_section_path_from_line(line) {
            if let Some(summary) = current.take() {
                summaries.push(summary);
            }
            current = Some(DiffFileSummary {
                path,
                added: 0,
                removed: 0,
            });
            continue;
        }

        if let Some(summary) = current.as_mut() {
            if is_added_diff_line(line) {
                summary.added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                summary.removed += 1;
            }
        }
    }

    if let Some(summary) = current {
        summaries.push(summary);
    }
    summaries
}

pub(crate) fn format_diff_stat(diff: &str, limit: Option<usize>) -> String {
    let summaries = diff_file_summaries(diff);
    if summaries.is_empty() {
        return "diff stat: no file sections found".to_string();
    }

    let total_added = summaries.iter().map(|summary| summary.added).sum::<usize>();
    let total_removed = summaries
        .iter()
        .map(|summary| summary.removed)
        .sum::<usize>();
    let mut lines = vec![format!(
        "diff stat: {} file(s), +{} -{}",
        summaries.len(),
        total_added,
        total_removed
    )];
    append_limited_diff_entries(&mut lines, &summaries, limit, |summary| {
        format!("- {} +{} -{}", summary.path, summary.added, summary.removed)
    });
    lines.join("\n")
}

pub(crate) fn format_diff_name_only(diff: &str, limit: Option<usize>) -> String {
    let mut summaries = diff_file_summaries(diff);
    summaries.dedup_by(|left, right| left.path == right.path);
    if summaries.is_empty() {
        return "diff files: no file sections found".to_string();
    }

    let mut lines = vec![format!("diff files: {} file(s)", summaries.len())];
    append_limited_diff_entries(&mut lines, &summaries, limit, |summary| {
        format!("- {}", summary.path)
    });
    lines.join("\n")
}

fn append_limited_diff_entries<F>(
    lines: &mut Vec<String>,
    summaries: &[DiffFileSummary],
    limit: Option<usize>,
    mut format_entry: F,
) where
    F: FnMut(&DiffFileSummary) -> String,
{
    let shown = limit.unwrap_or(summaries.len()).min(summaries.len());
    lines.extend(summaries.iter().take(shown).map(&mut format_entry));
    if summaries.len() > shown {
        lines.push(format!(
            "... {} more file(s). Increase with `/diff --limit {}`.",
            summaries.len() - shown,
            summaries.len()
        ));
    }
}

pub(crate) fn limit_display_lines(text: &str, limit: Option<usize>, label: &str) -> String {
    let Some(limit) = limit else {
        return text.to_string();
    };
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() <= limit {
        return text.to_string();
    }

    let mut output = lines.into_iter().take(limit).collect::<Vec<_>>().join("\n");
    output.push_str(&format!(
        "\n[deepcli {label} truncated: kept {limit} of {} line(s). Increase with `/diff --limit {}` or inspect `/diff --stat` first.]",
        text.lines().count(),
        text.lines().count()
    ));
    output
}

pub(crate) fn no_scoped_diff_detail(filters: &[String]) -> Option<String> {
    (!filters.is_empty()).then(|| {
        format!(
            "no changes matched path scope {}",
            format_verify_path_filters(filters)
        )
    })
}

pub(crate) fn resolve_scoped_session_diff_source(
    workspace: &Path,
    current: Option<&str>,
    limit: usize,
    filters: &[String],
) -> Result<Option<SessionDiffSource>> {
    Ok(resolve_session_diff_source(workspace, current, limit)?
        .and_then(|source| filter_session_diff_source_by_paths(source, filters)))
}

pub(crate) fn filter_session_diff_source_by_paths(
    mut source: SessionDiffSource,
    filters: &[String],
) -> Option<SessionDiffSource> {
    if filters.is_empty() {
        return Some(source);
    }
    source
        .records
        .retain(|record| session_diff_record_matches_filters(record, filters));
    if source.records.is_empty() {
        None
    } else {
        Some(source)
    }
}

fn session_diff_record_matches_filters(record: &SessionDiffRecord, filters: &[String]) -> bool {
    if path_matches_verify_filters(&record.name, filters) {
        return true;
    }
    let record_name = record.name.replace('\\', "/");
    if filters.iter().any(|filter| {
        let sanitized = filter
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        record_name.contains(&sanitized)
    }) {
        return true;
    }
    !filter_diff_by_paths(&record.content, filters)
        .trim()
        .is_empty()
}

pub(crate) fn resolve_session_diff_source(
    workspace: &Path,
    current: Option<&str>,
    limit: usize,
) -> Result<Option<SessionDiffSource>> {
    let store = SessionStore::new(workspace);
    if let Some(id) = current {
        let current_session = store.load(id)?;
        let records = load_non_empty_session_diffs(&current_session, limit)?;
        if !records.is_empty() {
            return Ok(Some(SessionDiffSource {
                session: current_session,
                note: None,
                records,
            }));
        }
        if let Some(mut source) = latest_session_with_diffs(&store, Some(id), limit)? {
            source.note = Some(format!(
                "latest session with {}; current session {id} had none",
                session_fallback_label(SessionFallbackKind::Diffs)
            ));
            return Ok(Some(source));
        }
        return Ok(None);
    }

    latest_session_with_diffs(&store, None, limit).map(|option| {
        option.map(|mut source| {
            source.note = Some("latest session with diff records; no current session".to_string());
            source
        })
    })
}

fn latest_session_with_diffs(
    store: &SessionStore,
    skip_id: Option<&str>,
    limit: usize,
) -> Result<Option<SessionDiffSource>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        let records = load_non_empty_session_diffs(&session, limit)?;
        if !records.is_empty() {
            return Ok(Some(SessionDiffSource {
                session,
                note: None,
                records,
            }));
        }
    }
    Ok(None)
}

fn load_non_empty_session_diffs(session: &Session, limit: usize) -> Result<Vec<SessionDiffRecord>> {
    Ok(session
        .load_recent_diffs(limit)?
        .into_iter()
        .filter(|record| !record.content.trim().is_empty())
        .collect())
}

pub(crate) fn format_session_diff_fallback(source: &SessionDiffSource) -> String {
    let mut output = session_diff_fallback_header(source);
    output.push('\n');
    output.push_str(&format_session_diffs(
        &source.records,
        SESSION_DIFF_FALLBACK_LIMIT,
    ));
    output
}

pub(crate) fn session_diff_fallback_header(source: &SessionDiffSource) -> String {
    let mut output = format!("session diff fallback: session {}", source.session.id());
    if let Some(title) = source.session.metadata.title.as_deref() {
        output.push_str(&format!(" ({title})"));
    }
    if let Some(note) = source.note.as_deref() {
        output.push_str(&format!("\nnote: {note}"));
    }
    output
}

pub(crate) fn session_diff_review_input(records: &[SessionDiffRecord]) -> String {
    records
        .iter()
        .map(|record| {
            format!(
                "diff --session {}\n{}",
                session_diff_record_display_path(record),
                record.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn session_diff_record_display_path(record: &SessionDiffRecord) -> String {
    record
        .content
        .lines()
        .find_map(diff_section_path_from_line)
        .unwrap_or_else(|| {
            record
                .name
                .split_once("Z-")
                .map(|(_, rest)| rest)
                .filter(|rest| !rest.is_empty())
                .unwrap_or(&record.name)
                .to_string()
        })
}

pub(crate) fn is_added_diff_line(line: &str) -> bool {
    line.starts_with('+') && !line.starts_with("+++")
}

pub(crate) fn review_path_from_diff_line(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("diff --git ") {
        let mut parts = rest.split_whitespace();
        let first = parts.next();
        let second = parts.next();
        return second
            .and_then(normalize_review_diff_path)
            .or_else(|| first.and_then(normalize_review_diff_path));
    }
    if let Some(rest) = line.strip_prefix("diff --session ") {
        let path = rest.trim();
        return (!path.is_empty()).then(|| path.to_string());
    }
    if let Some(rest) = line.strip_prefix("+++ ") {
        return normalize_review_diff_path(rest.trim());
    }
    None
}

pub(crate) fn normalize_review_diff_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches('"');
    if trimmed == "/dev/null" || trimmed.is_empty() {
        return None;
    }
    Some(
        trimmed
            .strip_prefix("a/")
            .or_else(|| trimmed.strip_prefix("b/"))
            .unwrap_or(trimmed)
            .to_string(),
    )
}
