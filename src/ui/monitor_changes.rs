use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::session::{SessionDiffRecord, SessionStore};

use super::monitor::{append_monitor_quick_actions, MonitorQuickAction, MonitorTab};
use super::monitor_shell::format_task_monitor_text;
use super::{
    active_session_ref, compact_ui_text, rect_contains, rect_content_row_contains,
    session_monitor_for_state, short_id, slash_command_suggestions_for_state, ActiveSessionRef,
    TuiState,
};

pub(super) const WORKTREE_CHANGES_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const WORKTREE_DIFF_PREVIEW_LINES: usize = 18;
pub(super) const WORKTREE_DIFF_SECTION_LINES: usize = 180;
const CHANGE_PATCH_WINDOW_LINES: usize = 12;
pub(super) const CHANGE_PATCH_SCROLL_STEP: usize = 8;
pub(super) const CHANGE_PATCH_MOUSE_SCROLL_STEP: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceChangesSnapshot {
    pub(super) available: bool,
    pub(super) detail: Option<String>,
    pub(super) changed: usize,
    pub(super) staged: usize,
    pub(super) unstaged: usize,
    pub(super) untracked: usize,
    pub(super) paths: Vec<String>,
    pub(super) diff_preview: Vec<String>,
    pub(super) diff_preview_truncated: bool,
    pub(super) diff_sections: Vec<WorkspaceDiffSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceDiffSection {
    pub(super) label: String,
    pub(super) path: String,
    pub(super) lines: Vec<String>,
    pub(super) truncated: bool,
}

struct ChangesPanelState {
    session_id: String,
    total_diff_records: usize,
    recent: Vec<SessionDiffRecord>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct DiffPanelStats {
    insertions: usize,
    deletions: usize,
    paths: Vec<String>,
}

pub(super) fn handle_changes_tab_key(key: KeyEvent, state: &mut TuiState) -> bool {
    if state.monitor_tab != MonitorTab::Changes || !state.input.buffer().trim().is_empty() {
        return false;
    }
    match key.code {
        KeyCode::Char(']') if key.modifiers.is_empty() => {
            select_next_change_patch(state);
            true
        }
        KeyCode::Char('[') if key.modifiers.is_empty() => {
            select_previous_change_patch(state);
            true
        }
        KeyCode::PageDown => {
            scroll_change_patch_down(state, CHANGE_PATCH_SCROLL_STEP);
            true
        }
        KeyCode::PageUp => {
            scroll_change_patch_up(state, CHANGE_PATCH_SCROLL_STEP);
            true
        }
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.change_patch_scroll = 0;
            state.last_event = change_patch_scroll_event(state);
            true
        }
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.change_patch_scroll = selected_change_patch_line_count(state).saturating_sub(1);
            state.last_event = change_patch_scroll_event(state);
            true
        }
        _ => false,
    }
}

fn select_next_change_patch(state: &mut TuiState) {
    let Some(count) = change_patch_count(state) else {
        state.last_event = "change patch unavailable".to_string();
        return;
    };
    state.selected_change = (state.selected_change + 1).min(count.saturating_sub(1));
    state.change_patch_scroll = 0;
    state.last_event = selected_change_patch_event(state);
}

fn select_previous_change_patch(state: &mut TuiState) {
    if change_patch_count(state).is_none() {
        state.last_event = "change patch unavailable".to_string();
        return;
    }
    state.selected_change = state.selected_change.saturating_sub(1);
    state.change_patch_scroll = 0;
    state.last_event = selected_change_patch_event(state);
}

pub(super) fn scroll_change_patch_down(state: &mut TuiState, amount: usize) {
    let max_scroll = selected_change_patch_line_count(state).saturating_sub(1);
    if max_scroll == 0 {
        state.last_event = change_patch_scroll_event(state);
        return;
    }
    state.change_patch_scroll = state
        .change_patch_scroll
        .saturating_add(amount)
        .min(max_scroll);
    state.last_event = change_patch_scroll_event(state);
}

pub(super) fn scroll_change_patch_up(state: &mut TuiState, amount: usize) {
    if selected_change_patch_line_count(state) == 0 {
        state.last_event = change_patch_scroll_event(state);
        return;
    }
    state.change_patch_scroll = state.change_patch_scroll.saturating_sub(amount);
    state.last_event = change_patch_scroll_event(state);
}

fn change_patch_count(state: &TuiState) -> Option<usize> {
    let count = state
        .workspace_changes
        .as_ref()
        .filter(|snapshot| snapshot.available)
        .map(|snapshot| snapshot.diff_sections.len())
        .unwrap_or_default();
    (count > 0).then_some(count)
}

fn selected_change_patch_line_count(state: &TuiState) -> usize {
    selected_change_patch_section(state)
        .map(|section| section.lines.len())
        .unwrap_or_default()
}

fn selected_change_patch_section(state: &TuiState) -> Option<&WorkspaceDiffSection> {
    let snapshot = state.workspace_changes.as_ref()?;
    if snapshot.diff_sections.is_empty() {
        return None;
    }
    snapshot
        .diff_sections
        .get(state.selected_change.min(snapshot.diff_sections.len() - 1))
}

fn selected_change_patch_event(state: &TuiState) -> String {
    selected_change_patch_section(state)
        .map(|section| {
            format!(
                "selected change patch {}",
                compact_ui_text(&section.path, 70)
            )
        })
        .unwrap_or_else(|| "change patch unavailable".to_string())
}

fn change_patch_scroll_event(state: &TuiState) -> String {
    if selected_change_patch_section(state).is_none() {
        return "change patch unavailable".to_string();
    }
    if state.change_patch_scroll == 0 {
        "change patch at top".to_string()
    } else {
        format!("change patch scrolled {}", state.change_patch_scroll)
    }
}

pub(super) fn select_change_patch_at_row(
    state: &mut TuiState,
    tools_area: Rect,
    column: u16,
    row: u16,
) -> bool {
    if state.monitor_tab != MonitorTab::Changes
        || !state.input.buffer().trim().is_empty()
        || state.resume_picker.is_some()
        || state.credential_prompt.is_some()
        || state.side_question_prompt.is_some()
        || slash_command_suggestions_for_state(state.input.buffer(), state.running).is_some()
        || !rect_contains(tools_area, column, row)
        || !rect_content_row_contains(tools_area, row)
    {
        return false;
    }

    let content_index = row.saturating_sub(tools_area.y + 1) as usize;
    let monitor = session_monitor_for_state(state);
    let text = format_task_monitor_text(state, monitor.as_ref(), tools_area.height);
    let lines = text.lines().collect::<Vec<_>>();
    let Some(worktree_files_row) = lines
        .iter()
        .position(|line| line.trim_start() == "worktree files:")
    else {
        return false;
    };

    let mut clicked_path_index = None;
    for (path_index, (line_index, _)) in lines
        .iter()
        .enumerate()
        .skip(worktree_files_row + 1)
        .take_while(|(_, line)| line.trim_start().starts_with("- "))
        .enumerate()
    {
        if line_index == content_index {
            clicked_path_index = Some(path_index);
            break;
        }
    }
    let Some(path_index) = clicked_path_index else {
        return false;
    };

    let Some((path, section_index)) = state
        .workspace_changes
        .as_ref()
        .filter(|snapshot| snapshot.available)
        .and_then(|snapshot| {
            let path = snapshot.paths.get(path_index)?.clone();
            let section_index = snapshot
                .diff_sections
                .iter()
                .position(|section| section.path == path);
            Some((path, section_index))
        })
    else {
        return false;
    };

    if let Some(section_index) = section_index {
        state.selected_change = section_index;
        state.change_patch_scroll = 0;
        state.last_event = selected_change_patch_event(state);
    } else {
        state.last_event = format!("no patch for {}", compact_ui_text(&path, 70));
    }
    true
}

pub(super) fn clamp_selected_change_patch(state: &mut TuiState) {
    let count = state
        .workspace_changes
        .as_ref()
        .map(|snapshot| snapshot.diff_sections.len())
        .unwrap_or_default();
    if count == 0 {
        state.selected_change = 0;
        state.change_patch_scroll = 0;
        return;
    }
    state.selected_change = state.selected_change.min(count - 1);
    let max_scroll = selected_change_patch_line_count(state).saturating_sub(1);
    state.change_patch_scroll = state.change_patch_scroll.min(max_scroll);
}

pub(super) fn refresh_workspace_changes_snapshot(state: &mut TuiState) {
    if state.monitor_tab != MonitorTab::Changes {
        return;
    }
    let now = Instant::now();
    if state
        .workspace_changes_checked_at
        .is_some_and(|checked_at| {
            now.duration_since(checked_at) < WORKTREE_CHANGES_REFRESH_INTERVAL
        })
    {
        return;
    }
    state.workspace_changes_checked_at = Some(now);
    state.workspace_changes = active_workspace_for_state(state)
        .as_deref()
        .map(load_workspace_changes_snapshot);
    clamp_selected_change_patch(state);
}

pub(super) fn format_changes_tab_lines(
    state: &TuiState,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    append_workspace_changes_lines(
        &mut lines,
        state.workspace_changes.as_ref(),
        state.selected_change,
        state.change_patch_scroll,
    );
    match load_changes_panel_state(state) {
        Ok(Some(panel)) => {
            lines.push(format!(
                "session: {} diff_records={} showing={}",
                short_id(&panel.session_id),
                panel.total_diff_records,
                panel.recent.len()
            ));
            if panel.total_diff_records == 0 {
                lines.push("no session diff records yet".to_string());
                lines.push("run /diff --stat to inspect current Git worktree".to_string());
            } else {
                let aggregate = summarize_diff_records(&panel.recent);
                lines.push(format!(
                    "recent summary: files={} +{} -{}",
                    aggregate.paths.len(),
                    aggregate.insertions,
                    aggregate.deletions
                ));
                if !aggregate.paths.is_empty() {
                    lines.push("files:".to_string());
                    for path in aggregate.paths.iter().take(4) {
                        lines.push(format!("  - {}", compact_ui_text(path, 96)));
                    }
                    if aggregate.paths.len() > 4 {
                        lines.push(format!("  ... {} more", aggregate.paths.len() - 4));
                    }
                }
                lines.push("recent diffs:".to_string());
                for record in panel.recent.iter().rev().take(4) {
                    let stats = summarize_diff_content(&record.content);
                    lines.push(format!(
                        "  * {} {} +{} -{}",
                        record.modified_at.format("%H:%M:%S"),
                        compact_diff_record_name(record),
                        stats.insertions,
                        stats.deletions
                    ));
                }
            }
        }
        Ok(None) => {
            lines.push("changes unavailable: no active session".to_string());
            lines.push("run /diff --stat to inspect current Git worktree".to_string());
        }
        Err(error) => {
            lines.push(format!(
                "changes unavailable: {}",
                compact_ui_text(&error.to_string(), 100)
            ));
            lines.push("run /session diffs --limit 5 for a detailed error".to_string());
        }
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

pub(super) fn append_workspace_changes_lines(
    lines: &mut Vec<String>,
    snapshot: Option<&WorkspaceChangesSnapshot>,
    selected_change: usize,
    patch_scroll: usize,
) {
    match snapshot {
        Some(snapshot) if snapshot.available && snapshot.changed == 0 => {
            lines.push("worktree: clean".to_string());
        }
        Some(snapshot) if snapshot.available => {
            lines.push(format!(
                "worktree: dirty changed={} staged={} unstaged={} untracked={}",
                snapshot.changed, snapshot.staged, snapshot.unstaged, snapshot.untracked
            ));
            if !snapshot.paths.is_empty() {
                lines.push("worktree files:".to_string());
                for path in snapshot.paths.iter().take(4) {
                    lines.push(format!("  - {}", compact_ui_text(path, 96)));
                }
                if snapshot.paths.len() > 4 {
                    lines.push(format!("  ... {} more", snapshot.paths.len() - 4));
                }
            }
            append_worktree_patch_preview_lines(lines, snapshot, selected_change, patch_scroll);
        }
        Some(snapshot) => {
            lines.push(format!(
                "worktree: unavailable{}",
                snapshot
                    .detail
                    .as_ref()
                    .map(|detail| format!(" ({})", compact_ui_text(detail, 90)))
                    .unwrap_or_default()
            ));
        }
        None => {
            lines.push("worktree: not checked yet".to_string());
        }
    }
}

fn append_worktree_patch_preview_lines(
    lines: &mut Vec<String>,
    snapshot: &WorkspaceChangesSnapshot,
    selected_change: usize,
    patch_scroll: usize,
) {
    if !snapshot.diff_sections.is_empty() {
        append_selected_worktree_patch_lines(lines, snapshot, selected_change, patch_scroll);
        return;
    }
    if snapshot.diff_preview.is_empty() {
        if snapshot.untracked > 0 && snapshot.changed == snapshot.untracked {
            lines.push("worktree patch: none (untracked files only)".to_string());
        } else {
            lines.push("worktree patch: none".to_string());
        }
        return;
    }
    lines.push(format!(
        "worktree patch preview{}:",
        if snapshot.diff_preview_truncated {
            " (truncated)"
        } else {
            ""
        }
    ));
    for line in &snapshot.diff_preview {
        lines.push(format!("  {}", compact_ui_text(line, 118)));
    }
}

fn append_selected_worktree_patch_lines(
    lines: &mut Vec<String>,
    snapshot: &WorkspaceChangesSnapshot,
    selected_change: usize,
    patch_scroll: usize,
) {
    let selected = selected_change.min(snapshot.diff_sections.len() - 1);
    let section = &snapshot.diff_sections[selected];
    let max_scroll = section.lines.len().saturating_sub(1);
    let start = patch_scroll.min(max_scroll);
    let end = (start + CHANGE_PATCH_WINDOW_LINES).min(section.lines.len());
    let below = section.lines.len().saturating_sub(end);
    lines.push(format!(
        "selected patch: {}/{} {} {}{}",
        selected + 1,
        snapshot.diff_sections.len(),
        section.label,
        compact_ui_text(&section.path, 72),
        if section.truncated {
            " (truncated)"
        } else {
            ""
        }
    ));
    if start > 0 {
        lines.push(format!("  [above: {start} line(s)]"));
    }
    for line in section.lines.iter().skip(start).take(end - start) {
        lines.push(format!("  {}", compact_ui_text(line, 118)));
    }
    if below > 0 {
        lines.push(format!("  [below: {below} line(s)]"));
    }
}

fn load_changes_panel_state(state: &TuiState) -> Result<Option<ChangesPanelState>> {
    let Some(active) = active_session_ref_for_state(state) else {
        return Ok(None);
    };
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let total_diff_records = session.activity_summary()?.diff_count;
    let recent = session.load_recent_diffs(5)?;
    Ok(Some(ChangesPanelState {
        session_id: active.session_id,
        total_diff_records,
        recent,
    }))
}

fn active_session_ref_for_state(state: &TuiState) -> Option<ActiveSessionRef> {
    state
        .runtime
        .as_ref()
        .map(active_session_ref)
        .or_else(|| state.active_session.clone())
}

fn active_workspace_for_state(state: &TuiState) -> Option<PathBuf> {
    state
        .runtime
        .as_ref()
        .map(|runtime| runtime.workspace().to_path_buf())
        .or_else(|| {
            state
                .active_session
                .as_ref()
                .map(|active| active.workspace.clone())
        })
}

pub(super) fn load_workspace_changes_snapshot(workspace: &Path) -> WorkspaceChangesSnapshot {
    match Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=normal"])
        .current_dir(workspace)
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut snapshot = parse_git_status_snapshot(&stdout);
            if snapshot.changed > snapshot.untracked {
                let (preview, truncated, sections) = load_git_diff_preview(workspace);
                snapshot.diff_preview = preview;
                snapshot.diff_preview_truncated = truncated;
                snapshot.diff_sections = sections;
            }
            snapshot
        }
        Ok(output) => WorkspaceChangesSnapshot {
            available: false,
            detail: Some(compact_ui_text(
                &String::from_utf8_lossy(&output.stderr),
                120,
            )),
            changed: 0,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            paths: Vec::new(),
            diff_preview: Vec::new(),
            diff_preview_truncated: false,
            diff_sections: Vec::new(),
        },
        Err(error) => WorkspaceChangesSnapshot {
            available: false,
            detail: Some(error.to_string()),
            changed: 0,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            paths: Vec::new(),
            diff_preview: Vec::new(),
            diff_preview_truncated: false,
            diff_sections: Vec::new(),
        },
    }
}

fn load_git_diff_preview(workspace: &Path) -> (Vec<String>, bool, Vec<WorkspaceDiffSection>) {
    let mut lines = Vec::new();
    let mut truncated = false;
    let mut sections = Vec::new();
    append_git_diff_preview_section(
        workspace,
        "unstaged",
        &["diff", "--no-ext-diff", "--unified=3", "--"],
        &mut lines,
        &mut truncated,
        &mut sections,
    );
    append_git_diff_preview_section(
        workspace,
        "staged",
        &["diff", "--cached", "--no-ext-diff", "--unified=3", "--"],
        &mut lines,
        &mut truncated,
        &mut sections,
    );
    (lines, truncated, sections)
}

fn append_git_diff_preview_section(
    workspace: &Path,
    label: &str,
    args: &[&str],
    lines: &mut Vec<String>,
    truncated: &mut bool,
    sections: &mut Vec<WorkspaceDiffSection>,
) {
    if *truncated {
        return;
    }
    let output = match Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            push_limited_preview_line(
                lines,
                truncated,
                format!("{label} diff unavailable: {error}"),
            );
            return;
        }
    };
    if !output.status.success() {
        let detail = compact_ui_text(&String::from_utf8_lossy(&output.stderr), 100);
        push_limited_preview_line(
            lines,
            truncated,
            format!("{label} diff unavailable: {detail}"),
        );
        return;
    }
    let content = String::from_utf8_lossy(&output.stdout);
    if content.trim().is_empty() {
        return;
    }
    sections.extend(parse_diff_sections(label, &content));
    push_limited_preview_line(lines, truncated, format!("{label} diff:"));
    for line in content.lines() {
        push_limited_preview_line(lines, truncated, line.to_string());
        if *truncated {
            return;
        }
    }
}

fn push_limited_preview_line(lines: &mut Vec<String>, truncated: &mut bool, line: String) {
    if lines.len() >= WORKTREE_DIFF_PREVIEW_LINES {
        *truncated = true;
        return;
    }
    lines.push(line);
}

pub(super) fn parse_diff_sections(label: &str, content: &str) -> Vec<WorkspaceDiffSection> {
    let mut sections = Vec::new();
    let mut current: Option<WorkspaceDiffSection> = None;
    for line in content.lines() {
        if line.starts_with("diff --git ") {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some(WorkspaceDiffSection {
                label: label.to_string(),
                path: diff_header_display_path(line).unwrap_or_else(|| "<unknown>".to_string()),
                lines: vec![line.to_string()],
                truncated: false,
            });
            continue;
        }
        if let Some(section) = current.as_mut() {
            push_limited_diff_section_line(section, line);
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }
    sections
}

fn push_limited_diff_section_line(section: &mut WorkspaceDiffSection, line: &str) {
    if section.lines.len() >= WORKTREE_DIFF_SECTION_LINES {
        section.truncated = true;
        return;
    }
    section.lines.push(line.to_string());
}

fn diff_header_display_path(line: &str) -> Option<String> {
    line.split_whitespace()
        .find_map(|part| part.strip_prefix("b/"))
        .map(|path| path.trim_matches('"').to_string())
        .filter(|path| !path.is_empty() && path != "/dev/null")
}

pub(super) fn parse_git_status_snapshot(status: &str) -> WorkspaceChangesSnapshot {
    let mut snapshot = WorkspaceChangesSnapshot {
        available: true,
        detail: None,
        changed: 0,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        paths: Vec::new(),
        diff_preview: Vec::new(),
        diff_preview_truncated: false,
        diff_sections: Vec::new(),
    };
    for line in status.lines().filter(|line| !line.trim().is_empty()) {
        snapshot.changed += 1;
        let bytes = line.as_bytes();
        let index_status = bytes.first().copied().unwrap_or(b' ') as char;
        let worktree_status = bytes.get(1).copied().unwrap_or(b' ') as char;
        if line.starts_with("?? ") {
            snapshot.untracked += 1;
        } else {
            if index_status != ' ' {
                snapshot.staged += 1;
            }
            if worktree_status != ' ' {
                snapshot.unstaged += 1;
            }
        }
        if let Some(path) = git_status_display_path(line) {
            push_unique_diff_path(&mut snapshot.paths, path);
        }
    }
    snapshot
}

fn git_status_display_path(line: &str) -> Option<String> {
    let path = line.get(3..)?.trim();
    if path.is_empty() {
        return None;
    }
    let display = path
        .rsplit(" -> ")
        .next()
        .unwrap_or(path)
        .trim_matches('"')
        .to_string();
    if display.is_empty() {
        None
    } else {
        Some(display)
    }
}

fn summarize_diff_records(records: &[SessionDiffRecord]) -> DiffPanelStats {
    let mut aggregate = DiffPanelStats::default();
    for record in records {
        let stats = summarize_diff_content(&record.content);
        aggregate.insertions += stats.insertions;
        aggregate.deletions += stats.deletions;
        for path in stats.paths {
            push_unique_diff_path(&mut aggregate.paths, path);
        }
    }
    aggregate
}

fn summarize_diff_content(content: &str) -> DiffPanelStats {
    let mut stats = DiffPanelStats::default();
    for line in content.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            push_unique_diff_path(&mut stats.paths, path.to_string());
            continue;
        }
        if line.starts_with("+++") {
            continue;
        }
        if let Some(path) = line.strip_prefix("--- a/") {
            push_unique_diff_path(&mut stats.paths, path.to_string());
            continue;
        }
        if line.starts_with("---") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            for part in rest.split_whitespace() {
                if let Some(path) = part.strip_prefix("b/") {
                    push_unique_diff_path(&mut stats.paths, path.to_string());
                }
            }
            continue;
        }
        if line.starts_with('+') {
            stats.insertions += 1;
        } else if line.starts_with('-') {
            stats.deletions += 1;
        }
    }
    stats
        .paths
        .retain(|path| path != "/dev/null" && !path.is_empty());
    stats
}

fn push_unique_diff_path(paths: &mut Vec<String>, path: String) {
    if path == "/dev/null" || path.is_empty() || paths.iter().any(|existing| existing == &path) {
        return;
    }
    paths.push(path);
}

fn compact_diff_record_name(record: &SessionDiffRecord) -> String {
    let display = record
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&record.name);
    compact_ui_text(display, 70)
}
