use crate::runtime::{SessionObservationEnvironment, SessionObservationUsage};

use super::TuiState;

pub(super) fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub(super) fn format_optional_bytes(value: Option<usize>) -> String {
    value
        .map(|value| format!("{}KiB", value.div_ceil(1024)))
        .unwrap_or_else(|| "-".to_string())
}

pub(super) fn format_cache_hit_rate(usage: &SessionObservationUsage) -> String {
    let Some(hit) = usage.prompt_cache_hit_tokens else {
        return "-".to_string();
    };
    let miss = usage.prompt_cache_miss_tokens.unwrap_or_default();
    let total = hit + miss;
    if total == 0 {
        "-".to_string()
    } else {
        format!("{:.1}%", hit as f64 * 100.0 / total as f64)
    }
}

pub(super) fn format_latest_environment(environment: &SessionObservationEnvironment) -> String {
    let ready = environment
        .ready
        .map(|ready| format!(" ready={ready}"))
        .unwrap_or_default();
    let detail = if environment.detail.is_empty() {
        String::new()
    } else {
        format!(" {}", compact_ui_text(&environment.detail, 86))
    };
    format!(
        "{} target={} status={}{}{}",
        environment.tool, environment.target, environment.status, ready, detail
    )
}

pub(super) fn compact_ui_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let keep = limit.saturating_sub(3);
    let mut output = value.chars().take(keep).collect::<String>();
    output.push_str("...");
    output
}

pub(super) fn format_action_event(prefix: &str, output: &str) -> String {
    let summary = first_non_empty_line(output).unwrap_or("<empty>");
    format!("{prefix}: {}", compact_ui_text(summary, 80))
}

pub(super) fn latest_action_result_line(state: &TuiState) -> Option<String> {
    latest_action_result(state).map(|result| {
        format!(
            "last output: {} {}",
            result.status,
            compact_ui_text(result.summary, 92)
        )
    })
}

pub(super) struct LatestActionResult<'a> {
    pub(super) status: &'static str,
    pub(super) summary: &'a str,
    pub(super) content: &'a str,
}

pub(super) fn latest_action_result(state: &TuiState) -> Option<LatestActionResult<'_>> {
    state.chat.iter().rev().find_map(|line| {
        let status = match line.role.as_str() {
            "deepcli" => "ok",
            "error" => "error",
            _ => return None,
        };
        let summary = first_non_empty_line(&line.content)?;
        Some(LatestActionResult {
            status,
            summary,
            content: &line.content,
        })
    })
}

pub(super) fn non_empty_output_lines(value: &str) -> Vec<&str> {
    value
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect()
}

pub(super) fn first_non_empty_line(value: &str) -> Option<&str> {
    value.lines().map(str::trim).find(|line| !line.is_empty())
}

pub(super) fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
