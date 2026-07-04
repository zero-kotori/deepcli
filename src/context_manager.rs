use crate::config::AppConfig;
use crate::providers::{ChatRequest, ProviderClient, ProviderMessage, ToolCall, ToolSpec};
use crate::session::{CompactBoundaryRecord, ProviderTranscriptRecord, ProviderTranscriptToolCall};
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;

#[derive(Debug, Clone)]
pub(crate) struct ContextManager {
    options: ContextCompactionOptions,
}

impl ContextManager {
    pub(crate) fn from_config(config: &AppConfig) -> Self {
        Self {
            options: ContextCompactionOptions::from_config(config),
        }
    }

    pub(crate) async fn prepare(
        &self,
        provider: &dyn ProviderClient,
        messages: &[ProviderMessage],
        tools: &[ToolSpec],
        provider_turn_timeout: Duration,
    ) -> Result<ContextPreparation> {
        prepare_messages_with_options(
            provider,
            messages,
            tools,
            &self.options,
            provider_turn_timeout,
        )
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextCompactionOptions {
    pub(crate) max_context_tokens: usize,
    pub(crate) reserved_output_tokens: usize,
    pub(crate) microcompact_keep_recent_tool_results: usize,
    pub(crate) microcompact_tool_output_chars: usize,
    pub(crate) full_compact_keep_recent_groups: usize,
}

impl ContextCompactionOptions {
    pub(crate) fn from_config(config: &AppConfig) -> Self {
        Self {
            max_context_tokens: config.agent.max_context_tokens.max(1),
            reserved_output_tokens: config.agent.reserved_output_tokens.max(1),
            microcompact_keep_recent_tool_results: env_usize(
                "DEEPCLI_MICROCOMPACT_KEEP_RECENT_TOOL_RESULTS",
                1,
                8,
            ),
            microcompact_tool_output_chars: env_usize(
                "DEEPCLI_MICROCOMPACT_TOOL_OUTPUT_CHARS",
                80,
                2_000,
            ),
            full_compact_keep_recent_groups: env_usize(
                "DEEPCLI_FULL_COMPACT_KEEP_RECENT_GROUPS",
                1,
                6,
            ),
        }
    }

    pub(crate) fn input_token_budget(&self) -> usize {
        if self.reserved_output_tokens >= self.max_context_tokens {
            return (self.max_context_tokens / 2).max(8_000);
        }
        self.max_context_tokens
            .saturating_sub(self.reserved_output_tokens)
            .max(8_000)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ContextPreparation {
    pub(crate) messages: Vec<ProviderMessage>,
    pub(crate) estimated_tokens: usize,
    pub(crate) threshold_tokens: usize,
    pub(crate) microcompacted_tool_results: usize,
    pub(crate) full_compacted: bool,
    pub(crate) tail_compacted: bool,
    pub(crate) full_compact_error: Option<String>,
    pub(crate) compact_boundary: Option<CompactBoundaryRecord>,
}

impl ContextPreparation {
    pub(crate) fn compacted(&self) -> bool {
        self.microcompacted_tool_results > 0 || self.full_compacted || self.tail_compacted
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MicrocompactOutcome {
    pub(crate) messages: Vec<ProviderMessage>,
    pub(crate) compacted_tool_results: usize,
}

#[derive(Debug, Clone)]
struct FullCompactAttempt {
    messages: Option<Vec<ProviderMessage>>,
    error: Option<String>,
    summary: Option<String>,
    omitted_group_count: usize,
}

#[derive(Debug, Clone)]
struct TailCompactOutcome {
    messages: Vec<ProviderMessage>,
    omitted_group_count: usize,
}

#[cfg(test)]
pub(crate) async fn prepare_messages_for_provider(
    provider: &dyn ProviderClient,
    messages: &[ProviderMessage],
    tools: &[ToolSpec],
    config: &AppConfig,
    provider_turn_timeout: Duration,
) -> Result<ContextPreparation> {
    ContextManager::from_config(config)
        .prepare(provider, messages, tools, provider_turn_timeout)
        .await
}

async fn prepare_messages_with_options(
    provider: &dyn ProviderClient,
    messages: &[ProviderMessage],
    tools: &[ToolSpec],
    options: &ContextCompactionOptions,
    provider_turn_timeout: Duration,
) -> Result<ContextPreparation> {
    let threshold_tokens = options.input_token_budget();
    let mut prepared_messages = messages.to_vec();
    let mut estimated_tokens = estimate_request_tokens(provider, &prepared_messages, tools);
    let mut microcompacted_tool_results = 0usize;
    let mut full_compacted = false;
    let mut tail_compacted = false;
    let mut full_compact_error = None;
    let mut compact_boundary_summary = None;
    let mut compact_boundary_omitted_groups = 0usize;
    let mut compact_boundary_reasons = Vec::new();
    let message_count_before = messages.len();

    if should_microcompact_before_provider_request(estimated_tokens, threshold_tokens) {
        let microcompact = microcompact_tool_outputs(messages, options);
        prepared_messages = microcompact.messages;
        microcompacted_tool_results = microcompact.compacted_tool_results;
        estimated_tokens = estimate_request_tokens(provider, &prepared_messages, tools);
    }

    if estimated_tokens > threshold_tokens {
        let attempt = full_compact_messages_with_provider(
            provider,
            &prepared_messages,
            options,
            provider_turn_timeout,
        )
        .await;
        full_compact_error = attempt.error.clone();
        if let Some(compacted) = attempt.messages {
            compact_boundary_summary = attempt.summary;
            compact_boundary_omitted_groups += attempt.omitted_group_count;
            compact_boundary_reasons.push("full_compact");
            prepared_messages = compacted;
            full_compacted = true;
            estimated_tokens = estimate_request_tokens(provider, &prepared_messages, tools);
        }
    }

    if estimated_tokens > threshold_tokens {
        let compacted = compact_messages_for_provider_with_details(&prepared_messages);
        if compacted.messages.len() != prepared_messages.len() {
            compact_boundary_omitted_groups += compacted.omitted_group_count;
            compact_boundary_reasons.push("tail_compact");
            prepared_messages = compacted.messages;
            tail_compacted = true;
            estimated_tokens = estimate_request_tokens(provider, &prepared_messages, tools);
        }
    }

    let compact_boundary = if compact_boundary_reasons.is_empty() {
        None
    } else {
        Some(CompactBoundaryRecord {
            id: uuid::Uuid::new_v4(),
            reason: compact_boundary_reasons.join("+"),
            summary: compact_boundary_summary.unwrap_or_else(|| {
                "Older completed assistant/tool exchange groups were omitted to keep the provider request within the context budget."
                    .to_string()
            }),
            omitted_group_count: compact_boundary_omitted_groups,
            message_count_before,
            message_count_after: prepared_messages.len(),
            retained_segment: provider_messages_to_retained_segment(&prepared_messages),
            created_at: Utc::now(),
        })
    };

    Ok(ContextPreparation {
        messages: prepared_messages,
        estimated_tokens,
        threshold_tokens,
        microcompacted_tool_results,
        full_compacted,
        tail_compacted,
        full_compact_error,
        compact_boundary,
    })
}

fn should_microcompact_before_provider_request(
    estimated_tokens: usize,
    threshold_tokens: usize,
) -> bool {
    let near_threshold_tokens = threshold_tokens.saturating_mul(9) / 10;
    estimated_tokens >= near_threshold_tokens
}

pub(crate) fn microcompact_tool_outputs(
    messages: &[ProviderMessage],
    options: &ContextCompactionOptions,
) -> MicrocompactOutcome {
    let tool_indices = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| (message.role == "tool").then_some(index))
        .collect::<Vec<_>>();
    let keep_from = tool_indices
        .len()
        .saturating_sub(options.microcompact_keep_recent_tool_results);
    let mut compacted_tool_results = 0usize;
    let mut compacted = messages.to_vec();

    for (ordinal, index) in tool_indices.into_iter().enumerate() {
        if ordinal >= keep_from {
            continue;
        }
        let Some(content) = compacted[index].content.clone() else {
            continue;
        };
        if should_preserve_tool_output_for_microcompact(&compacted[index], &content) {
            continue;
        }
        if content.chars().count() <= options.microcompact_tool_output_chars {
            continue;
        }
        compacted[index].content = Some(compact_tool_output_content(
            &compacted[index],
            &content,
            options.microcompact_tool_output_chars,
        ));
        compacted_tool_results += 1;
    }

    MicrocompactOutcome {
        messages: compacted,
        compacted_tool_results,
    }
}

fn should_preserve_tool_output_for_microcompact(message: &ProviderMessage, content: &str) -> bool {
    if matches!(
        message.name.as_deref(),
        Some(
            "write_file"
                | "apply_patch_or_write"
                | "run_tests"
                | "git_commit"
                | "git_create_branch"
                | "todo_write"
                | "ask_user_question"
        )
    ) {
        return true;
    }

    let Ok(value) = serde_json::from_str::<Value>(content) else {
        return false;
    };
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        return true;
    }
    if value.pointer("/data/passed").and_then(Value::as_bool) == Some(false) {
        return true;
    }
    if matches!(
        value.get("kind").and_then(Value::as_str),
        Some(
            "file_diff"
                | "git_diff"
                | "git_commit_message"
                | "todo_list"
                | "question"
                | "subagent_task"
                | "environment_setup"
        )
    ) {
        return true;
    }
    matches!(
        value.get("tool").and_then(Value::as_str),
        Some(
            "write_file"
                | "apply_patch_or_write"
                | "run_tests"
                | "git_commit"
                | "git_create_branch"
                | "todo_write"
                | "ask_user_question"
        )
    )
}

fn compact_tool_output_content(message: &ProviderMessage, content: &str, limit: usize) -> String {
    let char_count = content.chars().count();
    let budget = limit.max(80);
    let head_limit = (budget * 2 / 3).max(1);
    let tail_limit = budget.saturating_sub(head_limit).max(1);
    let head = content.chars().take(head_limit).collect::<String>();
    let tail = content
        .chars()
        .rev()
        .take(tail_limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    let tool = message.name.as_deref().unwrap_or("<unknown>");
    let tool_call_id = message.tool_call_id.as_deref().unwrap_or("<unknown>");
    format!(
        "{head}\n\n[deepcli compacted tool output: tool={tool} tool_call_id={tool_call_id} original_chars={char_count} kept_head={head_limit} kept_tail={tail_limit}. Re-run a focused read/search command if exact omitted output is needed.]\n\n{tail}"
    )
}

pub(crate) fn provider_messages_to_retained_segment(
    messages: &[ProviderMessage],
) -> Vec<ProviderTranscriptRecord> {
    let limit = env_usize("DEEPCLI_COMPACT_BOUNDARY_RETAINED_MESSAGES", 1, 16);
    let skip = messages.len().saturating_sub(limit);
    messages
        .iter()
        .skip(skip)
        .filter(|message| message.role != "system")
        .map(provider_message_to_transcript_record)
        .collect()
}

pub(crate) fn provider_message_to_transcript_record(
    message: &ProviderMessage,
) -> ProviderTranscriptRecord {
    let reasoning_limit = provider_transcript_reasoning_limit();
    ProviderTranscriptRecord {
        role: message.role.clone(),
        content: message
            .content
            .as_deref()
            .map(|content| truncate_chars(content, provider_transcript_content_limit())),
        reasoning_content: if reasoning_limit == 0 {
            None
        } else {
            message
                .reasoning_content
                .as_deref()
                .map(|content| truncate_chars(content, reasoning_limit))
        },
        name: message.name.clone(),
        tool_call_id: message.tool_call_id.clone(),
        tool_calls: message
            .tool_calls
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(provider_tool_call_to_transcript)
            .collect(),
        synthetic: false,
        created_at: Utc::now(),
    }
}

fn provider_tool_call_to_transcript(call: &ToolCall) -> ProviderTranscriptToolCall {
    ProviderTranscriptToolCall {
        id: call.id.clone(),
        call_type: call.call_type.clone(),
        name: call.function.name.clone(),
        arguments: call.function.arguments.clone(),
    }
}

fn provider_transcript_content_limit() -> usize {
    env_usize("DEEPCLI_PROVIDER_TRANSCRIPT_CONTENT_CHARS", 200, 4_000)
}

fn provider_transcript_reasoning_limit() -> usize {
    env_usize("DEEPCLI_PROVIDER_TRANSCRIPT_REASONING_CHARS", 0, 0)
}

async fn full_compact_messages_with_provider(
    provider: &dyn ProviderClient,
    messages: &[ProviderMessage],
    options: &ContextCompactionOptions,
    provider_turn_timeout: Duration,
) -> FullCompactAttempt {
    let (summary_source, omitted_groups) =
        full_compact_summary_source(messages, options.full_compact_keep_recent_groups);
    if omitted_groups == 0 {
        return FullCompactAttempt {
            messages: None,
            error: None,
            summary: None,
            omitted_group_count: 0,
        };
    }

    let mut summary_messages = summary_source;
    summary_messages.push(ProviderMessage {
        role: "user".to_string(),
        content: Some(full_compact_summary_prompt()),
        reasoning_content: None,
        name: None,
        tool_call_id: None,
        tool_calls: None,
    });

    let response = match timeout(
        provider_turn_timeout,
        provider.chat(ChatRequest {
            messages: summary_messages,
            tools: Vec::new(),
            json_mode: false,
        }),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            return FullCompactAttempt {
                messages: None,
                error: Some(error.to_string()),
                summary: None,
                omitted_group_count: 0,
            };
        }
        Err(_) => {
            return FullCompactAttempt {
                messages: None,
                error: Some(format!(
                    "provider compact summary timed out after {} seconds",
                    provider_turn_timeout.as_secs()
                )),
                summary: None,
                omitted_group_count: 0,
            };
        }
    };
    let summary = response.content.unwrap_or_default();
    if summary.trim().is_empty() {
        return FullCompactAttempt {
            messages: None,
            error: Some("provider compact summary returned empty content".to_string()),
            summary: None,
            omitted_group_count: 0,
        };
    }

    FullCompactAttempt {
        messages: Some(build_full_compacted_messages(
            messages,
            summary.trim(),
            options.full_compact_keep_recent_groups,
        )),
        error: None,
        summary: Some(summary.trim().to_string()),
        omitted_group_count: omitted_groups,
    }
}

fn full_compact_summary_source(
    messages: &[ProviderMessage],
    keep_recent_groups: usize,
) -> (Vec<ProviderMessage>, usize) {
    let base_count = messages.len().min(2);
    let groups = message_groups(&messages[base_count..]);
    let keep_count = keep_recent_groups.min(groups.len());
    let omitted_groups = groups.len().saturating_sub(keep_count);
    if omitted_groups == 0 {
        return (messages.to_vec(), 0);
    }

    let mut source = messages[..base_count].to_vec();
    for group in groups.iter().take(omitted_groups) {
        source.extend(group.clone());
    }
    (source, omitted_groups)
}

pub(crate) fn build_full_compacted_messages(
    messages: &[ProviderMessage],
    summary: &str,
    keep_recent_groups: usize,
) -> Vec<ProviderMessage> {
    let base_count = messages.len().min(2);
    let base = messages[..base_count].to_vec();
    let groups = message_groups(&messages[base_count..]);
    let keep_count = keep_recent_groups.min(groups.len());
    let omitted_groups = groups.len().saturating_sub(keep_count);
    if omitted_groups == 0 {
        return messages.to_vec();
    }

    let mut compacted = base;
    compacted.push(ProviderMessage {
        role: "user".to_string(),
        content: Some(format!(
            "[deepcli compacted conversation summary]\nEarlier assistant/tool exchange groups were summarized to keep the provider request within the context budget.\n\n{summary}"
        )),
        reasoning_content: None,
        name: None,
        tool_call_id: None,
        tool_calls: None,
    });
    for group in groups.into_iter().skip(omitted_groups) {
        compacted.extend(group);
    }
    compacted
}

pub(crate) fn full_compact_summary_prompt() -> String {
    [
        "Create a compact recovery summary of the conversation so far.",
        "Use these exact section labels:",
        "User goal:",
        "Changed files:",
        "Tool findings:",
        "Errors and fixes:",
        "Pending work:",
        "Next step:",
        "Preserve explicit user requests, important constraints, files inspected or changed, failing and passing verification evidence, open questions, and the exact next action needed to continue.",
        "Do not call tools. Return only the summary text.",
    ]
    .join("\n")
}

pub(crate) fn compact_messages_for_context_retry(
    messages: &[ProviderMessage],
) -> Vec<ProviderMessage> {
    let compacted = compact_messages_for_provider(messages);
    if compacted != messages {
        return compacted;
    }
    compact_messages_to_recent_groups(
        messages,
        4,
        "[deepcli context retry compacted: provider rejected the previous request as too long. Older completed assistant/tool exchange groups were omitted. Re-read specific files or rerun focused commands if exact omitted output is needed.]",
    )
}

pub(crate) fn append_output_recovery_prompt(messages: &[ProviderMessage]) -> Vec<ProviderMessage> {
    let mut recovered = messages.to_vec();
    recovered.push(ProviderMessage {
        role: "user".to_string(),
        content: Some(
            "[deepcli output recovery]\nThe previous provider turn exceeded the output limit before completion. Continue with a concise response, preserve critical facts, and prefer focused tool calls over long prose if more work is required."
                .to_string(),
        ),
        reasoning_content: None,
        name: None,
        tool_call_id: None,
        tool_calls: None,
    });
    recovered
}

pub(crate) fn compact_messages_to_recent_groups(
    messages: &[ProviderMessage],
    keep_recent_groups: usize,
    marker: &str,
) -> Vec<ProviderMessage> {
    if messages.len() <= 4 {
        return messages.to_vec();
    }
    let base_count = messages.len().min(2);
    let base = messages[..base_count].to_vec();
    let groups = message_groups(&messages[base_count..]);
    let keep_count = keep_recent_groups.min(groups.len());
    let omitted = groups.len().saturating_sub(keep_count);
    if omitted == 0 {
        return messages.to_vec();
    }

    let mut compacted = base;
    compacted.push(ProviderMessage {
        role: "user".to_string(),
        content: Some(format!("{marker}\nomitted_groups={omitted}")),
        reasoning_content: None,
        name: None,
        tool_call_id: None,
        tool_calls: None,
    });
    for group in groups.into_iter().skip(omitted) {
        compacted.extend(group);
    }
    compacted
}

pub(crate) fn message_groups_omitted_after_compaction(
    before: &[ProviderMessage],
    after: &[ProviderMessage],
) -> usize {
    let before_base_count = before.len().min(2);
    let after_base_count = after.len().min(2);
    let before_groups = message_groups(&before[before_base_count..]).len();
    let after_groups = message_groups(&after[after_base_count..])
        .into_iter()
        .filter(|group| {
            !group.first().is_some_and(|message| {
                message
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains("[deepcli context"))
            })
        })
        .count();
    before_groups.saturating_sub(after_groups)
}

pub(crate) fn compact_messages_for_provider(messages: &[ProviderMessage]) -> Vec<ProviderMessage> {
    compact_messages_for_provider_with_details(messages).messages
}

fn compact_messages_for_provider_with_details(messages: &[ProviderMessage]) -> TailCompactOutcome {
    let limit = std::env::var("DEEPCLI_MAX_PROVIDER_REQUEST_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= 16_000)
        .unwrap_or(96_000);
    let current = serde_json::to_vec(messages)
        .map(|value| value.len())
        .unwrap_or_default();
    if current <= limit || messages.len() <= 4 {
        return TailCompactOutcome {
            messages: messages.to_vec(),
            omitted_group_count: 0,
        };
    }

    let base_count = messages.len().min(2);
    let base = messages[..base_count].to_vec();
    let base_bytes = serde_json::to_vec(&base)
        .map(|value| value.len())
        .unwrap_or_default();
    let summary_budget = 512;
    let target_tail_budget = limit.saturating_sub(base_bytes + summary_budget).max(8_000);
    let groups = message_groups(&messages[base_count..]);
    let mut kept_groups = Vec::new();
    let mut kept_bytes = 0usize;
    for group in groups.iter().rev() {
        let group_bytes = serde_json::to_vec(group)
            .map(|value| value.len())
            .unwrap_or_default();
        if !kept_groups.is_empty() && kept_bytes + group_bytes > target_tail_budget {
            break;
        }
        kept_bytes += group_bytes;
        kept_groups.push(group.clone());
    }
    kept_groups.reverse();

    let omitted = groups.len().saturating_sub(kept_groups.len());
    if omitted == 0 {
        return TailCompactOutcome {
            messages: messages.to_vec(),
            omitted_group_count: 0,
        };
    }

    let mut compacted = base;
    compacted.push(ProviderMessage {
        role: "user".to_string(),
        content: Some(format!(
            "[deepcli context compacted: omitted {omitted} earlier completed assistant/tool exchange group(s). The omitted exchanges were older diagnostic reads, shell probes, or test outputs. Re-read specific files or rerun focused commands if needed.]"
        )),
        reasoning_content: None,
        name: None,
        tool_call_id: None,
        tool_calls: None,
    });
    for group in kept_groups {
        compacted.extend(group);
    }
    TailCompactOutcome {
        messages: compacted,
        omitted_group_count: omitted,
    }
}

fn message_groups(messages: &[ProviderMessage]) -> Vec<Vec<ProviderMessage>> {
    let mut groups = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        let mut group = vec![messages[index].clone()];
        index += 1;
        while index < messages.len() && messages[index].role == "tool" {
            group.push(messages[index].clone());
            index += 1;
        }
        groups.push(group);
    }
    groups
}

fn estimate_request_tokens(
    provider: &dyn ProviderClient,
    messages: &[ProviderMessage],
    tools: &[ToolSpec],
) -> usize {
    provider
        .count_tokens(messages)
        .max(rough_json_tokens(messages))
        + rough_json_tokens(tools)
}

fn rough_json_tokens<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_string(value)
        .map(|value| value.chars().count().div_ceil(4))
        .unwrap_or_default()
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let head = limit.saturating_sub(16);
    format!(
        "{}...[truncated]",
        value.chars().take(head.max(1)).collect::<String>()
    )
}

fn env_usize(name: &str, min: usize, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= min)
        .unwrap_or(default)
}
