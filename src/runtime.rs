use crate::commands::{CommandContext, CommandRouter};
use crate::config::AppConfig;
use crate::permissions::PermissionEngine;
use crate::providers::{create_provider, ChatRequest, ProviderMessage, ToolCall, Usage};
use crate::session::{Plan, PlanStep, PlanStepStatus, Session, SessionState, SessionStore};
use crate::tools::{ToolExecutor, ToolRegistry};
use crate::workspace::WorkspaceManager;
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use tokio::time::timeout;

pub struct RuntimeOptions {
    pub workspace: PathBuf,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub assume_yes: bool,
    pub resume_session: Option<String>,
    pub stream_output: bool,
}

pub struct AgentRuntime {
    workspace: PathBuf,
    config: AppConfig,
    registry: ToolRegistry,
    session: Session,
    executor: ToolExecutor,
    stream_output: bool,
    progress_tx: Option<Sender<RuntimeProgress>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeProgress {
    ProviderStreamStarted,
    ProviderTurnStarted {
        iteration: usize,
        max_iterations: usize,
        message_count: usize,
        tool_count: usize,
        request_kib: usize,
        compacted: bool,
    },
    ProviderTurnCompleted {
        elapsed_ms: u128,
        tool_calls: usize,
    },
    ToolStarted {
        tool: String,
    },
    ToolCompleted {
        tool: String,
        ok: bool,
        summary: String,
    },
}

impl RuntimeProgress {
    pub fn plain_text(&self) -> String {
        match self {
            RuntimeProgress::ProviderStreamStarted => {
                "deep-cli: provider stream started".to_string()
            }
            RuntimeProgress::ProviderTurnStarted {
                iteration,
                max_iterations,
                message_count,
                tool_count,
                request_kib,
                compacted,
            } => format!(
                "deep-cli: provider turn {iteration}/{max_iterations} (messages={message_count}, tools={tool_count}, request~{request_kib} KiB{})",
                if *compacted { ", compacted" } else { "" }
            ),
            RuntimeProgress::ProviderTurnCompleted {
                elapsed_ms,
                tool_calls,
            } => format!(
                "deep-cli: provider response in {:.1}s (tool_calls={tool_calls})",
                *elapsed_ms as f64 / 1000.0
            ),
            RuntimeProgress::ToolStarted { tool } => {
                format!("deep-cli: running tool {tool}")
            }
            RuntimeProgress::ToolCompleted { tool, ok, .. } => {
                let status = if *ok { "completed" } else { "failed" };
                format!("deep-cli: tool {tool} {status}")
            }
        }
    }
}

impl AgentRuntime {
    pub fn new(config: AppConfig, options: RuntimeOptions) -> Result<Self> {
        let workspace = options.workspace;
        let workspace_manager = WorkspaceManager::new(&workspace)?;
        if workspace_manager.load_authorization()?.is_none() && options.assume_yes {
            workspace_manager.grant_authorization("read")?;
        }

        let mut provider_runtime =
            config.provider_runtime(&workspace, options.provider.as_deref())?;
        if let Some(model) = options.model {
            provider_runtime.model = Some(model);
        }

        let store = SessionStore::new(&workspace);
        let session = if let Some(id) = options.resume_session {
            store.load(&id)?
        } else {
            store.create(
                &workspace,
                provider_runtime.name.clone(),
                provider_runtime.model.clone(),
            )?
        };

        let permissions = PermissionEngine::new(
            &workspace,
            config.permissions.clone(),
            config.sandbox.clone(),
        );
        let executor = ToolExecutor::new(
            &workspace,
            permissions,
            Some(session.clone()),
            config.agent.max_subagent_depth,
        )
        .with_assume_yes(options.assume_yes);

        Ok(Self {
            workspace,
            config,
            registry: ToolRegistry::mvp(),
            session,
            executor,
            stream_output: options.stream_output,
            progress_tx: None,
        })
    }

    pub fn session_id(&self) -> String {
        self.session.id().to_string()
    }

    pub fn provider_name(&self) -> &str {
        &self.session.metadata.provider
    }

    pub fn model_name(&self) -> Option<&str> {
        self.session.metadata.model.as_deref()
    }

    pub fn state_label(&self) -> String {
        format!("{:?}", self.session.metadata.state)
    }

    pub fn set_progress_sender(&mut self, progress_tx: Option<Sender<RuntimeProgress>>) {
        self.progress_tx = progress_tx;
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub async fn handle_input(&mut self, input: &str) -> Result<String> {
        if let Some(command) = CommandRouter::parse(input)? {
            return CommandRouter::handle(
                command,
                CommandContext {
                    workspace: &self.workspace,
                    config: &self.config,
                    registry: &self.registry,
                    executor: &self.executor,
                    session_id: Some(self.session_id()),
                },
            )
            .await;
        }
        if is_low_information_input(input) && !self.has_open_user_context()? {
            return self.handle_low_information_input(input);
        }
        self.run_agent_task(input).await
    }

    fn handle_low_information_input(&mut self, input: &str) -> Result<String> {
        let message =
            "我不确定你想执行什么。请说明要我分析、修改、测试、继续上次任务，或使用 /help 查看命令。";
        self.session.append_message("user", input)?;
        self.session.append_message("assistant", message)?;
        self.session.write_summary(message)?;
        self.session.set_state(SessionState::WaitingUser)?;
        Ok(message.to_string())
    }

    fn has_open_user_context(&self) -> Result<bool> {
        if matches!(
            self.session.metadata.state,
            SessionState::AwaitingApproval | SessionState::Paused
        ) {
            return Ok(true);
        }
        if self
            .session
            .load_side_questions()?
            .iter()
            .any(|item| item.status == crate::session::SideQuestionStatus::Open)
        {
            return Ok(true);
        }
        if self
            .session
            .load_approval_requests()?
            .iter()
            .any(|item| item.status == crate::session::ApprovalStatus::Pending)
        {
            return Ok(true);
        }
        Ok(self.session.load_plan()?.is_some_and(|plan| {
            plan.steps.iter().any(|step| {
                matches!(
                    step.status,
                    PlanStepStatus::InProgress | PlanStepStatus::Failed
                )
            })
        }))
    }

    pub async fn run_agent_task(&mut self, task: &str) -> Result<String> {
        self.session.set_state(SessionState::ContextLoading)?;
        self.session.append_message("user", task)?;

        let workspace_context = WorkspaceManager::new(&self.workspace)?.collect_context()?;
        if self.config.agent.require_plan_for_complex_tasks && is_complex_task(task) {
            self.session.set_state(SessionState::Planning)?;
            self.session.save_plan(&default_plan(task))?;
        }

        let provider_runtime = self
            .config
            .provider_runtime(&self.workspace, Some(&self.session.metadata.provider))?;
        let provider = create_provider(provider_runtime)?;
        let mut messages = vec![
            ProviderMessage {
                role: "system".to_string(),
                content: Some(system_prompt(&workspace_context, &self.config)),
                reasoning_content: None,
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            ProviderMessage {
                role: "user".to_string(),
                content: Some(task.to_string()),
                reasoning_content: None,
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        ];
        let estimated_tokens = provider.count_tokens(&messages);
        let provider_turn_timeout = self.provider_turn_timeout();

        self.session.set_state(SessionState::Executing)?;
        if self.stream_output && !is_complex_task(task) {
            self.emit_progress(RuntimeProgress::ProviderStreamStarted);
            self.session
                .append_audit_event("provider_stream_started", json!({}))?;
            let events = match timeout(
                provider_turn_timeout,
                provider.stream(ChatRequest {
                    messages: messages.clone(),
                    tools: Vec::new(),
                    json_mode: false,
                }),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => {
                    self.session.set_state(SessionState::Failed)?;
                    return Err(anyhow!(
                        "provider stream timed out after {} seconds",
                        provider_turn_timeout.as_secs()
                    ));
                }
            };
            let content = events
                .into_iter()
                .filter_map(|event| event.content_delta)
                .collect::<String>();
            self.session.append_message("assistant", &content)?;
            self.session.write_summary(&content)?;
            self.session.complete_pending_plan_steps()?;
            self.session.set_state(SessionState::Completed)?;
            return Ok(with_token_estimate(content, estimated_tokens));
        }

        let context_tool_limit = context_tool_limit();
        let verification_tool_limit = verification_tool_limit();
        let budget_skip_turn_limit = budget_skip_turn_limit();
        let mut context_tool_calls_before_action = 0usize;
        let mut verification_calls_before_action = 0usize;
        let mut consecutive_budget_skipped_turns = 0usize;
        for iteration in 0..self.config.agent.max_tool_iterations {
            let tool_specs = self.registry.tool_specs();
            let compacted_messages = compact_messages_for_provider(&messages);
            let compacted = compacted_messages.len() != messages.len();
            let request_stats = provider_request_stats(&compacted_messages, &tool_specs);
            self.emit_progress(RuntimeProgress::ProviderTurnStarted {
                iteration: iteration + 1,
                max_iterations: self.config.agent.max_tool_iterations,
                message_count: request_stats.message_count,
                tool_count: request_stats.tool_count,
                request_kib: request_stats.total_bytes.div_ceil(1024),
                compacted,
            });
            self.session.append_audit_event(
                "provider_turn_started",
                json!({
                    "iteration": iteration + 1,
                    "timeout_seconds": provider_turn_timeout.as_secs(),
                    "request": {
                        "message_count": request_stats.message_count,
                        "message_bytes": request_stats.message_bytes,
                        "tool_count": request_stats.tool_count,
                        "tool_bytes": request_stats.tool_bytes,
                        "total_bytes": request_stats.total_bytes,
                        "compacted": compacted
                    }
                }),
            )?;
            let started = Instant::now();
            let response = match timeout(
                provider_turn_timeout,
                provider.chat(ChatRequest {
                    messages: compacted_messages,
                    tools: tool_specs,
                    json_mode: false,
                }),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => {
                    self.session.set_state(SessionState::Failed)?;
                    return Err(anyhow!(
                        "provider chat timed out after {} seconds",
                        provider_turn_timeout.as_secs()
                    ));
                }
            };
            let elapsed = started.elapsed();
            self.emit_progress(RuntimeProgress::ProviderTurnCompleted {
                elapsed_ms: elapsed.as_millis(),
                tool_calls: response.tool_calls.len(),
            });
            self.session.append_audit_event(
                "provider_turn_completed",
                json!({
                    "iteration": iteration + 1,
                    "elapsed_ms": elapsed.as_millis(),
                    "tool_calls": response.tool_calls.len(),
                    "usage": response.usage
                }),
            )?;

            if response.tool_calls.is_empty() {
                let mut content = response.content.unwrap_or_default();
                append_usage_report(
                    &mut content,
                    &response.usage,
                    self.config.usage.token_warning_threshold,
                );
                let content = with_token_estimate(content, estimated_tokens);
                self.session.append_message("assistant", &content)?;
                self.session
                    .update_plan_step("review", PlanStepStatus::Completed)?;
                self.session.complete_pending_plan_steps()?;
                self.session.set_state(SessionState::Completed)?;
                self.session.write_summary(&content)?;
                return Ok(content);
            }

            messages.push(ProviderMessage {
                role: "assistant".to_string(),
                content: response.content.clone(),
                reasoning_content: response.reasoning_content.clone(),
                name: None,
                tool_call_id: None,
                tool_calls: Some(response.tool_calls.clone()),
            });

            let mut budget_skipped_this_turn = 0usize;
            for call in response.tool_calls {
                if call.function.name == "run_tests" {
                    self.session.set_state(SessionState::Testing)?;
                }
                let tool_output = if is_context_gathering_call(&call)
                    && context_tool_calls_before_action >= context_tool_limit
                {
                    budget_skipped_this_turn += 1;
                    format!(
                        "tool `{}` skipped: context-gathering budget exceeded after {} context-only tool calls without a patch or verification action. Stop gathering context and either apply a focused patch, run a focused verification command, or report the concrete blocker.",
                        call.function.name, context_tool_limit
                    )
                } else if is_verification_call(&call)
                    && verification_calls_before_action >= verification_tool_limit
                {
                    budget_skipped_this_turn += 1;
                    format!(
                        "tool `{}` skipped: verification budget exceeded after {} verification-only tool calls without a project write. Stop running more tests, apply a focused patch to the current failure, or report the concrete blocker.",
                        call.function.name, verification_tool_limit
                    )
                } else {
                    self.execute_tool_call(&call).await?
                };
                if call.function.name == "run_tests" {
                    self.session.set_state(SessionState::Executing)?;
                }
                let tool_failed = tool_output_indicates_failure(&tool_output);
                if is_context_gathering_call(&call) {
                    context_tool_calls_before_action += 1;
                } else if is_progress_action_call(&call) && !tool_failed {
                    context_tool_calls_before_action = 0;
                }
                if is_project_mutating_call(&call) && !tool_failed {
                    verification_calls_before_action = 0;
                } else if is_verification_call(&call) {
                    verification_calls_before_action += 1;
                }
                let tool_output = truncate_tool_output_for_prompt(&tool_output);
                messages.push(ProviderMessage {
                    role: "tool".to_string(),
                    content: Some(tool_output),
                    reasoning_content: None,
                    name: Some(call.function.name),
                    tool_call_id: Some(call.id),
                    tool_calls: None,
                });
            }

            if budget_skipped_this_turn > 0 {
                consecutive_budget_skipped_turns += 1;
                if consecutive_budget_skipped_turns >= budget_skip_turn_limit {
                    self.session.set_state(SessionState::Failed)?;
                    return Err(anyhow!(
                        "agent repeated budget-skipped tool calls for {} consecutive turns",
                        consecutive_budget_skipped_turns
                    ));
                }
            } else {
                consecutive_budget_skipped_turns = 0;
            }
        }

        self.session.set_state(SessionState::Failed)?;
        Err(anyhow!("agent loop reached maximum tool-call iterations"))
    }

    fn provider_turn_timeout(&self) -> Duration {
        Duration::from_secs(self.config.agent.provider_turn_timeout_seconds.max(1))
    }

    fn emit_progress(&self, event: RuntimeProgress) {
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(event);
        } else {
            eprintln!("{}", event.plain_text());
        }
    }

    async fn execute_tool_call(&mut self, call: &ToolCall) -> Result<String> {
        if !self.registry.has(&call.function.name) {
            return Err(anyhow!(
                "provider requested unknown tool `{}`",
                call.function.name
            ));
        }
        self.emit_progress(RuntimeProgress::ToolStarted {
            tool: call.function.name.clone(),
        });
        self.session
            .append_audit_event("tool_started", json!({ "tool": call.function.name }))?;
        let execution = match self
            .executor
            .execute(&call.function.name, call.function.arguments.clone())
            .await
        {
            Ok(execution) => execution,
            Err(error) if is_approval_error(&error) => {
                self.session.set_state(SessionState::AwaitingApproval)?;
                let output = format!(
                    "tool `{}` is awaiting approval: {error}",
                    call.function.name
                );
                self.emit_progress(RuntimeProgress::ToolCompleted {
                    tool: call.function.name.clone(),
                    ok: false,
                    summary: truncate_progress_detail(&output),
                });
                return Ok(output);
            }
            Err(error) => {
                self.session.append_audit_event(
                    "tool_failed",
                    json!({
                        "tool": call.function.name,
                        "error": error.to_string()
                    }),
                )?;
                let output = format!("tool `{}` failed: {error}", call.function.name);
                self.emit_progress(RuntimeProgress::ToolCompleted {
                    tool: call.function.name.clone(),
                    ok: false,
                    summary: truncate_progress_detail(&output),
                });
                return Ok(output);
            }
        };
        self.update_plan_after_tool(
            &call.function.name,
            execution.raw.get("passed").and_then(|v| v.as_bool()),
        )?;
        self.emit_progress(RuntimeProgress::ToolCompleted {
            tool: call.function.name.clone(),
            ok: true,
            summary: truncate_progress_detail(&execution.content),
        });
        Ok(execution.content)
    }

    fn update_plan_after_tool(&self, tool_name: &str, passed: Option<bool>) -> Result<()> {
        match tool_name {
            "read_file" | "list_files" | "search" => {
                self.session
                    .update_plan_step("context", PlanStepStatus::Completed)?;
            }
            "write_file" | "apply_patch_or_write" => {
                self.session
                    .update_plan_step("implementation", PlanStepStatus::Completed)?;
                if self.plan_step_status("repair")? == Some(PlanStepStatus::InProgress) {
                    self.session
                        .update_plan_step("repair", PlanStepStatus::Completed)?;
                }
            }
            "discover_tests" => {
                self.session
                    .update_plan_step("verification", PlanStepStatus::InProgress)?;
            }
            "run_tests" => {
                if passed == Some(false) {
                    self.session
                        .update_plan_step("verification", PlanStepStatus::Failed)?;
                    self.session
                        .update_plan_step("repair", PlanStepStatus::InProgress)?;
                } else {
                    self.session
                        .update_plan_step("verification", PlanStepStatus::Completed)?;
                    if self.plan_step_status("repair")? == Some(PlanStepStatus::InProgress) {
                        self.session
                            .update_plan_step("repair", PlanStepStatus::Completed)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn plan_step_status(&self, id: &str) -> Result<Option<PlanStepStatus>> {
        Ok(self.session.load_plan()?.and_then(|plan| {
            plan.steps
                .into_iter()
                .find(|step| step.id == id)
                .map(|step| step.status)
        }))
    }
}

fn with_token_estimate(mut content: String, estimated_prompt_tokens: usize) -> String {
    content.push_str(&format!(
        "\n\n[usage estimate] prompt_tokens~{}",
        estimated_prompt_tokens
    ));
    content
}

fn append_usage_report(content: &mut String, usage: &Usage, token_warning_threshold: usize) {
    let mut lines = Vec::new();
    if let Some(total) = usage.total_tokens {
        if total as usize >= token_warning_threshold {
            lines.push(format!(
                "[token warning] total_tokens={} reached configured threshold {}",
                total, token_warning_threshold
            ));
        }
    }
    if usage.prompt_cache_hit_tokens.is_some() || usage.prompt_cache_miss_tokens.is_some() {
        let hit = usage.prompt_cache_hit_tokens.unwrap_or_default();
        let miss = usage.prompt_cache_miss_tokens.unwrap_or_default();
        let total = hit + miss;
        let hit_rate = if total == 0 {
            0.0
        } else {
            hit as f64 / total as f64 * 100.0
        };
        lines.push(format!(
            "[context cache] prompt_cache_hit_tokens={} prompt_cache_miss_tokens={} hit_rate={hit_rate:.1}%",
            hit, miss
        ));
    }
    if !lines.is_empty() {
        content.push_str("\n\n");
        content.push_str(&lines.join("\n"));
    }
}

struct ProviderRequestStats {
    message_count: usize,
    message_bytes: usize,
    tool_count: usize,
    tool_bytes: usize,
    total_bytes: usize,
}

fn provider_request_stats(
    messages: &[ProviderMessage],
    tools: &[crate::providers::ToolSpec],
) -> ProviderRequestStats {
    let message_bytes = serde_json::to_vec(messages)
        .map(|value| value.len())
        .unwrap_or_default();
    let tool_bytes = serde_json::to_vec(tools)
        .map(|value| value.len())
        .unwrap_or_default();
    ProviderRequestStats {
        message_count: messages.len(),
        message_bytes,
        tool_count: tools.len(),
        tool_bytes,
        total_bytes: message_bytes + tool_bytes,
    }
}

fn compact_messages_for_provider(messages: &[ProviderMessage]) -> Vec<ProviderMessage> {
    let limit = std::env::var("DEEP_CLI_MAX_PROVIDER_REQUEST_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= 16_000)
        .unwrap_or(96_000);
    let current = serde_json::to_vec(messages)
        .map(|value| value.len())
        .unwrap_or_default();
    if current <= limit || messages.len() <= 4 {
        return messages.to_vec();
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
        return messages.to_vec();
    }

    let mut compacted = base;
    compacted.push(ProviderMessage {
        role: "user".to_string(),
        content: Some(format!(
            "[deep-cli context compacted: omitted {omitted} earlier completed assistant/tool exchange group(s). The omitted exchanges were older diagnostic reads, shell probes, or test outputs. Re-read specific files or rerun focused commands if needed.]"
        )),
        reasoning_content: None,
        name: None,
        tool_call_id: None,
        tool_calls: None,
    });
    for group in kept_groups {
        compacted.extend(group);
    }
    compacted
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

fn context_tool_limit() -> usize {
    std::env::var("DEEP_CLI_MAX_CONTEXT_TOOL_CALLS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(12)
}

fn verification_tool_limit() -> usize {
    std::env::var("DEEP_CLI_MAX_VERIFICATION_TOOL_CALLS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(12)
}

fn budget_skip_turn_limit() -> usize {
    std::env::var("DEEP_CLI_MAX_BUDGET_SKIPPED_TURNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(3)
}

fn is_context_gathering_tool(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "list_files"
            | "search"
            | "git_status"
            | "git_diff"
            | "git_branch"
            | "open_terminal"
            | "check_environment"
            | "discover_tests"
    )
}

fn is_context_gathering_call(call: &ToolCall) -> bool {
    if is_context_gathering_tool(&call.function.name) {
        return true;
    }
    if call.function.name == "spawn_subagent" {
        return true;
    }
    if call.function.name == "write_file" {
        return !is_project_write_call(call);
    }
    if call.function.name != "run_shell" {
        return false;
    }
    let command = call
        .function
        .arguments
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim();
    if run_shell_writes_project(call) {
        return false;
    }
    !is_progress_shell_command(command)
}

fn is_progress_action_call(call: &ToolCall) -> bool {
    match call.function.name.as_str() {
        "apply_patch_or_write" | "run_tests" | "setup_environment" => true,
        "write_file" => is_project_write_call(call),
        "run_shell" => {
            run_shell_writes_project(call)
                || call
                    .function
                    .arguments
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .map(is_progress_shell_command)
                    .unwrap_or(false)
        }
        _ => false,
    }
}

fn is_project_mutating_call(call: &ToolCall) -> bool {
    match call.function.name.as_str() {
        "apply_patch_or_write" => true,
        "write_file" => is_project_write_call(call),
        "run_shell" => run_shell_writes_project(call),
        _ => false,
    }
}

fn is_verification_call(call: &ToolCall) -> bool {
    match call.function.name.as_str() {
        "run_tests" => true,
        "run_shell" => call
            .function
            .arguments
            .get("command")
            .and_then(serde_json::Value::as_str)
            .map(is_progress_shell_command)
            .unwrap_or(false),
        _ => false,
    }
}

fn is_project_write_call(call: &ToolCall) -> bool {
    let Some(path) = call
        .function
        .arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    let normalized = path.trim().trim_start_matches("./").replace('\\', "/");
    let root_files = [
        "AGENTS.md",
        "Cargo.lock",
        "Cargo.toml",
        "README.md",
        ".deepignore",
        ".gitignore",
    ];
    root_files.contains(&normalized.as_str())
        || normalized.starts_with("src/")
        || normalized.starts_with("tests/")
        || normalized.starts_with("docs/")
        || normalized.starts_with("examples/")
        || normalized.starts_with("benches/")
        || normalized.starts_with(".github/")
}

fn run_shell_writes_files(call: &ToolCall) -> bool {
    call.function
        .arguments
        .get("writes_files")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn run_shell_writes_project(call: &ToolCall) -> bool {
    if run_shell_writes_files(call) {
        return true;
    }
    let command = call
        .function
        .arguments
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    shell_command_writes_project(command)
}

fn shell_command_writes_project(command: &str) -> bool {
    let normalized = command.replace('\\', "/");
    let writes = [
        "apply_patch",
        "sed -i",
        "cat >",
        "cat <<",
        "tee ",
        "rm -f",
        "rm ",
        "mkdir ",
        "mv ",
    ];
    let project_paths = [
        "src/",
        "tests/",
        "docs/",
        "examples/",
        "benches/",
        ".github/",
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        ".deepignore",
        ".gitignore",
        "AGENTS.md",
        "README.md",
    ];
    writes.iter().any(|needle| normalized.contains(needle))
        && project_paths.iter().any(|path| normalized.contains(path))
}

fn shell_command_after_cd(command: &str) -> &str {
    command
        .trim_start_matches("cd ")
        .split_once("&&")
        .map(|(_, rest)| rest.trim())
        .unwrap_or(command)
        .trim()
}

fn is_progress_shell_command(command: &str) -> bool {
    let normalized = shell_command_after_cd(command);
    normalized.contains("cargo fmt")
        || normalized.contains("cargo test")
        || normalized.contains("cargo build")
        || normalized.contains("cargo check")
        || normalized.contains("cargo clippy")
        || normalized.contains("autotest")
        || normalized.contains("apply_patch")
}

fn tool_output_indicates_failure(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.starts_with("tool `")
        && (lower.contains(" failed:")
            || lower.contains(" awaiting approval:")
            || lower.contains(" skipped:"))
        || lower.contains(" failed:")
        || lower.contains("refusing to overwrite")
        || lower.contains("patch check failed")
        || lower.contains("patch apply failed")
}

fn truncate_tool_output_for_prompt(output: &str) -> String {
    let limit = std::env::var("DEEP_CLI_MAX_TOOL_OUTPUT_CHARS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(16_000);
    let char_count = output.chars().count();
    if char_count <= limit {
        return output.to_string();
    }

    let head_limit = limit * 2 / 3;
    let tail_limit = limit - head_limit;
    let head = output.chars().take(head_limit).collect::<String>();
    let tail = output
        .chars()
        .rev()
        .take(tail_limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!(
        "{head}\n\n[deep-cli truncated tool output: original_chars={char_count}, kept_head={head_limit}, kept_tail={tail_limit}. Use narrower read_file ranges, search, or shell filters for more detail.]\n\n{tail}"
    )
}

fn truncate_progress_detail(output: &str) -> String {
    let limit = 2_000usize;
    let char_count = output.chars().count();
    if char_count <= limit {
        return output.to_string();
    }
    let head = output.chars().take(limit).collect::<String>();
    format!("{head}\n\n[deep-cli truncated UI detail: original_chars={char_count}]")
}

fn is_approval_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("requires approval") || message.contains("requires double confirmation")
}

fn is_complex_task(task: &str) -> bool {
    let task = task.to_ascii_lowercase();
    task.contains("修改")
        || task.contains("实现")
        || task.contains("测试")
        || task.contains("fix")
        || task.contains("implement")
        || task.contains("refactor")
        || task.split_whitespace().count() > 12
}

fn is_low_information_input(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let normalized = trimmed.to_ascii_lowercase();
    if normalized.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }
    if normalized.chars().count() <= 2
        && normalized
            .chars()
            .all(|ch| ch.is_ascii_punctuation() || ch.is_ascii_alphanumeric())
    {
        return true;
    }

    matches!(
        normalized.as_str(),
        "ok" | "k" | "y" | "n" | "yes" | "no" | "嗯" | "好" | "继续" | "go" | "next"
    )
}

fn default_plan(task: &str) -> Plan {
    Plan {
        title: format!("Plan for: {task}"),
        updated_at: Utc::now(),
        steps: vec![
            PlanStep {
                id: "context".to_string(),
                description: "Read relevant workspace context and constraints.".to_string(),
                status: PlanStepStatus::Pending,
            },
            PlanStep {
                id: "implementation".to_string(),
                description: "Apply the smallest code changes through permission-checked tools."
                    .to_string(),
                status: PlanStepStatus::Pending,
            },
            PlanStep {
                id: "verification".to_string(),
                description: "Discover and run relevant tests, then repair failures.".to_string(),
                status: PlanStepStatus::Pending,
            },
            PlanStep {
                id: "repair".to_string(),
                description:
                    "Analyze failing validation output and apply targeted fixes if needed."
                        .to_string(),
                status: PlanStepStatus::Pending,
            },
            PlanStep {
                id: "review".to_string(),
                description: "Review diff, summarize changes, validation, and residual risks."
                    .to_string(),
                status: PlanStepStatus::Pending,
            },
        ],
    }
}

fn system_prompt(context: &crate::workspace::WorkspaceContext, config: &AppConfig) -> String {
    let docs = context
        .docs_files
        .iter()
        .map(|file| file.path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let agents = context
        .agents_files
        .iter()
        .map(|file| file.path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    json!({
        "role": "deep-cli agent",
        "language": config.agent.language,
        "workflow": "analyze -> plan -> modify -> test -> repair -> report",
        "rules": [
            format!("Always respond in {} unless the user explicitly asks for another language.", config.agent.language),
            "If the user input is ambiguous or too short to identify a concrete task, ask a concise clarification question before using tools.",
            "All filesystem, shell, git, network, skill, and sub-agent actions must use tools.",
            "For complex tasks, explain the plan before editing.",
            "Use minimal scoped changes and run relevant tests.",
            "For existing files, prefer apply_patch_or_write with a unified diff patch; use write_file only for new files or small complete rewrites.",
            "Do not replace an existing source file with placeholder, omitted, or partial content.",
            "Never expose credentials or secrets in logs or messages."
        ],
        "workspace": context.root,
        "agents_files": agents,
        "docs_files": docs,
        "git_diff_present": context.git_diff_present
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn complex_task_detection_covers_chinese_and_english() {
        assert!(is_complex_task("请实现一个新功能并运行测试"));
        assert!(is_complex_task(
            "implement the provider adapter and run tests"
        ));
        assert!(!is_complex_task("hello"));
    }

    #[test]
    fn low_information_input_is_detected_locally() {
        assert!(is_low_information_input("1"));
        assert!(is_low_information_input("ok"));
        assert!(is_low_information_input("."));
        assert!(is_low_information_input("继续"));
        assert!(!is_low_information_input("/help"));
        assert!(!is_low_information_input("请阅读项目结构"));
    }

    #[test]
    fn default_plan_contains_verification_step() {
        let plan = default_plan("task");
        assert!(plan.steps.iter().any(|step| step.id == "verification"));
        assert!(plan.steps.iter().any(|step| step.id == "repair"));
    }

    #[test]
    fn runtime_can_initialize_without_calling_provider() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".deepcli/credentials")).unwrap();
        fs::write(
            dir.path().join(".deepcli/config.json"),
            serde_json::to_vec_pretty(&AppConfig::default()).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path()
                .join(".deepcli/credentials/deepseek-credentials.json"),
            r#"{"apiKey":"test","model":"deepseek-chat"}"#,
        )
        .unwrap();
        let config = AppConfig::load_effective(dir.path(), None).unwrap();
        let runtime = AgentRuntime::new(
            config,
            RuntimeOptions {
                workspace: dir.path().to_path_buf(),
                provider: None,
                model: None,
                assume_yes: true,
                resume_session: None,
                stream_output: false,
            },
        )
        .unwrap();
        assert!(!runtime.session_id().is_empty());
    }

    #[test]
    fn runtime_marks_repair_step_after_failed_tests() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".deepcli/credentials")).unwrap();
        fs::write(
            dir.path().join(".deepcli/config.json"),
            serde_json::to_vec_pretty(&AppConfig::default()).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path()
                .join(".deepcli/credentials/deepseek-credentials.json"),
            r#"{"apiKey":"test","model":"deepseek-chat"}"#,
        )
        .unwrap();
        let config = AppConfig::load_effective(dir.path(), None).unwrap();
        let runtime = AgentRuntime::new(
            config,
            RuntimeOptions {
                workspace: dir.path().to_path_buf(),
                provider: None,
                model: None,
                assume_yes: true,
                resume_session: None,
                stream_output: false,
            },
        )
        .unwrap();

        runtime
            .session
            .save_plan(&default_plan("fix tests"))
            .unwrap();
        runtime
            .update_plan_after_tool("run_tests", Some(false))
            .unwrap();
        let plan = runtime.session.load_plan().unwrap().unwrap();
        let status = |id: &str| {
            plan.steps
                .iter()
                .find(|step| step.id == id)
                .unwrap()
                .status
                .clone()
        };
        assert_eq!(status("verification"), PlanStepStatus::Failed);
        assert_eq!(status("repair"), PlanStepStatus::InProgress);

        runtime.update_plan_after_tool("write_file", None).unwrap();
        let plan = runtime.session.load_plan().unwrap().unwrap();
        assert_eq!(
            plan.steps
                .iter()
                .find(|step| step.id == "repair")
                .unwrap()
                .status,
            PlanStepStatus::Completed
        );
    }

    #[test]
    fn usage_report_includes_context_cache_stats() {
        let mut content = "done".to_string();
        append_usage_report(
            &mut content,
            &Usage {
                prompt_tokens: Some(10),
                completion_tokens: Some(2),
                total_tokens: Some(12),
                prompt_cache_hit_tokens: Some(7),
                prompt_cache_miss_tokens: Some(3),
            },
            100,
        );
        assert!(content.contains("[context cache]"));
        assert!(content.contains("prompt_cache_hit_tokens=7"));
        assert!(content.contains("hit_rate=70.0%"));
    }

    #[test]
    fn truncates_large_tool_output_for_prompt() {
        let output = "x".repeat(20_000);
        let truncated = truncate_tool_output_for_prompt(&output);
        assert!(truncated.len() < output.len());
        assert!(truncated.contains("truncated tool output"));
    }

    #[test]
    fn compacts_provider_history_at_group_boundaries() {
        let mut messages = vec![
            ProviderMessage {
                role: "system".to_string(),
                content: Some("system".to_string()),
                reasoning_content: None,
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            ProviderMessage {
                role: "user".to_string(),
                content: Some("task".to_string()),
                reasoning_content: None,
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        ];
        for i in 0..20 {
            messages.push(ProviderMessage {
                role: "assistant".to_string(),
                content: None,
                reasoning_content: Some("thinking".repeat(20)),
                name: None,
                tool_call_id: None,
                tool_calls: Some(vec![ToolCall {
                    id: format!("call_{i}"),
                    call_type: "function".to_string(),
                    function: crate::providers::ToolCallFunction {
                        name: "read_file".to_string(),
                        arguments: json!({"path": "src/lib.rs"}),
                    },
                }]),
            });
            messages.push(ProviderMessage {
                role: "tool".to_string(),
                content: Some("x".repeat(8_000)),
                reasoning_content: None,
                name: Some("read_file".to_string()),
                tool_call_id: Some(format!("call_{i}")),
                tool_calls: None,
            });
        }

        let compacted = compact_messages_for_provider(&messages);
        assert!(compacted.len() < messages.len());
        assert_eq!(compacted[0].role, "system");
        assert_eq!(compacted[1].role, "user");
        assert!(compacted[2]
            .content
            .as_deref()
            .unwrap()
            .contains("context compacted"));
        assert_ne!(compacted[3].role, "tool");
    }

    #[test]
    fn classifies_context_gathering_tools() {
        assert!(is_context_gathering_tool("read_file"));
        assert!(is_context_gathering_tool("list_files"));
        assert!(is_context_gathering_tool("search"));
        assert!(is_context_gathering_tool("git_status"));
        assert!(is_context_gathering_tool("open_terminal"));
        assert!(!is_context_gathering_tool("apply_patch_or_write"));
        assert!(!is_context_gathering_tool("run_tests"));

        let tool_call = |name: &str, arguments: serde_json::Value| ToolCall {
            id: format!("call_{name}"),
            call_type: "function".to_string(),
            function: crate::providers::ToolCallFunction {
                name: name.to_string(),
                arguments,
            },
        };
        let shell_call = |command: &str| {
            tool_call(
                "run_shell",
                json!({
                    "command": command
                }),
            )
        };
        let shell_write_call = |command: &str| {
            tool_call(
                "run_shell",
                json!({
                    "command": command,
                    "writes_files": true
                }),
            )
        };
        let subagent_call = ToolCall {
            id: "call_shell".to_string(),
            call_type: "function".to_string(),
            function: crate::providers::ToolCallFunction {
                name: "spawn_subagent".to_string(),
                arguments: json!({ "prompt": "inspect" }),
            },
        };
        let scratch_write = tool_call("write_file", json!({ "path": "test_global.c" }));
        let source_write = tool_call("write_file", json!({ "path": "src/koopa_gen.rs" }));

        assert!(is_context_gathering_call(&shell_call("rg SymbolInfo src")));
        assert!(is_context_gathering_call(&shell_call(
            "cd /tmp/repo && sed -n '1,20p' src/lib.rs"
        )));
        assert!(is_context_gathering_call(&shell_call(
            "docker run image bash -c 'find / -name *.sy'"
        )));
        assert!(is_context_gathering_call(&subagent_call));
        assert!(is_context_gathering_call(&scratch_write));
        assert!(!is_context_gathering_call(&source_write));
        assert!(!is_context_gathering_call(&shell_call("cargo test")));
        assert!(!is_context_gathering_call(&shell_call(
            "docker run autotest"
        )));
        assert!(!is_context_gathering_call(&shell_call(
            "sed -i '' 's/a/b/' src/parser.rs"
        )));
        assert!(is_progress_shell_command("cargo build"));
        assert!(!is_progress_shell_command(
            "python3 - <<'PY'\nprint('inspect')\nPY"
        ));
        assert!(is_verification_call(&tool_call(
            "run_tests",
            json!({"command": "cargo build"})
        )));
        assert!(is_verification_call(&shell_call("cargo build")));
        assert!(!is_verification_call(&shell_call(
            "sed -i '' 's/a/b/' src/parser.rs"
        )));
        assert!(is_project_mutating_call(&shell_call(
            "sed -i '' 's/a/b/' src/parser.rs"
        )));
        assert!(is_progress_action_call(&shell_write_call(
            "python3 - <<'PY'\nprint('patch')\nPY"
        )));
        assert!(is_progress_action_call(&source_write));
        assert!(!is_progress_action_call(&scratch_write));
        assert!(tool_output_indicates_failure(
            "tool `apply_patch_or_write` failed: patch check failed"
        ));
        assert!(tool_output_indicates_failure(
            "refusing to overwrite existing large file src/lib.rs with much shorter content"
        ));
        assert!(!tool_output_indicates_failure(
            "Finished dev profile successfully"
        ));
    }

    #[test]
    fn approval_errors_are_detected_for_runtime_tool_flow() {
        let error = anyhow::anyhow!("operation requires approval: write mode");
        assert!(is_approval_error(&error));
        let other = anyhow::anyhow!("tool failed");
        assert!(!is_approval_error(&other));
    }

    #[tokio::test]
    async fn runtime_returns_tool_failures_to_the_model_loop() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".deepcli/credentials")).unwrap();
        fs::write(
            dir.path().join(".deepcli/config.json"),
            serde_json::to_vec_pretty(&AppConfig::default()).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path()
                .join(".deepcli/credentials/deepseek-credentials.json"),
            r#"{"apiKey":"test","model":"deepseek-chat"}"#,
        )
        .unwrap();
        let config = AppConfig::load_effective(dir.path(), None).unwrap();
        let mut runtime = AgentRuntime::new(
            config,
            RuntimeOptions {
                workspace: dir.path().to_path_buf(),
                provider: None,
                model: None,
                assume_yes: true,
                resume_session: None,
                stream_output: false,
            },
        )
        .unwrap();
        let call = ToolCall {
            id: "call_bad_patch".to_string(),
            call_type: "function".to_string(),
            function: crate::providers::ToolCallFunction {
                name: "apply_patch_or_write".to_string(),
                arguments: json!({"patch": "not a valid patch"}),
            },
        };

        let output = runtime.execute_tool_call(&call).await.unwrap();
        assert!(output.contains("tool `apply_patch_or_write` failed"));
    }
}
