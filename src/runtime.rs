use crate::commands::{
    format_session_list, handle_config, handle_credentials_with_default, handle_model_read_command,
    handle_timeout, list_resumable_sessions, parse_model_set_args, run_cmd_shell,
    set_credentials_api_key, update_project_model_config, CommandContext, CommandRouter,
    SlashCommand,
};
use crate::config::AppConfig;
use crate::context_manager::{
    append_output_recovery_prompt, compact_messages_for_context_retry,
    message_groups_omitted_after_compaction, provider_message_to_transcript_record,
    provider_messages_to_retained_segment, ContextManager,
};
#[cfg(test)]
use crate::context_manager::{
    build_full_compacted_messages, compact_messages_for_provider, full_compact_summary_prompt,
    microcompact_tool_outputs, prepare_messages_for_provider, ContextCompactionOptions,
};
use crate::permissions::PermissionEngine;
use crate::providers::{
    create_provider, ChatRequest, ChatResponse, ProviderClient, ProviderMessage, StreamEvent,
    ToolCall, Usage,
};
use crate::session::{
    ApprovalRequest, ApprovalStatus, AuditEvent, CompactBoundaryRecord, FileHistorySnapshot,
    GoalContract, GoalStatus, Plan, PlanStep, PlanStepStatus, ProviderTranscriptRecord,
    ProviderTranscriptToolCall, Session, SessionMessage, SessionState, SessionStore, SideQuestion,
    SideQuestionStatus, TestRunRecord, ToolCallRecord, ToolCallStatus,
};
use crate::tools::{StructuredToolResult, ToolExecution, ToolExecutor, ToolRegistry};
use crate::workspace::WorkspaceManager;
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures_util::future::join_all;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};
use tokio::time::timeout;

const SESSION_CONTEXT_MESSAGE_LIMIT: usize = 16;
const SESSION_CONTEXT_MESSAGE_CHARS: usize = 1_500;
const SESSION_CONTEXT_TOTAL_CHARS: usize = 16_000;
const AGENTS_INSTRUCTION_CONTENT_CHARS: usize = 16_000;
const PLAN_CRITICAL_FILES_BLOCKER: &str =
    "final plan must include `Critical Files for Implementation` with 3-5 file paths";
const PLAN_USER_QUESTION_BLOCKER: &str =
    "plan mode must call `ask_user_question` with repository-specific questions before finalizing";
const PLAN_MODE_TOOLS: &[&str] = &[
    "read_file",
    "list_files",
    "search",
    "git_status",
    "git_diff",
    "git_branch",
    "discover_tests",
    "ask_user_question",
    "prompt_list",
    "prompt_get",
    "prompt_render",
    "skill_list",
    "skill_run",
];

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
    planning_mode_active: bool,
    planning_initial_side_question_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentRunMode {
    Normal,
    Planning,
}

fn should_run_model_plan(args: &[String]) -> bool {
    !matches!(args.first().map(String::as_str), None | Some("show"))
}

fn model_plan_requirement(args: &[String]) -> Result<String> {
    if args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--json" | "--write-doc" | "--write-requirements" | "--output" | "-o"
        ) || arg.starts_with("--write-doc=")
            || arg.starts_with("--output=")
    }) {
        return Err(anyhow!(
            "`/plan <requirement>` now uses the model-backed planning flow; legacy draft/output options are no longer supported"
        ));
    }
    let requirement = args.join(" ").trim().to_string();
    if requirement.is_empty() {
        return Err(anyhow!("/plan requires a requirement or `show`"));
    }
    Ok(requirement)
}

fn build_model_plan_prompt(requirement: &str) -> String {
    format!(
        r#"You are in DeepCLI plan mode.

Current user requirement:
{requirement}

Plan-mode rules:
- Do not modify files or execute implementation steps.
- Use read-only exploration before answering: inspect relevant files, search for existing patterns, and check git context when useful.
- After initial read-only exploration, call `ask_user_question` with 1-3 focused, code-context-aware questions before finalizing the plan. Include an `options` array with 2-4 concise choices whenever practical; the UI will also allow custom input.
- Ask only questions that depend on this requirement and this repository. Do not use generic fixed planning questions.
- Do not ask the user whether the plan is okay; finish with a concrete plan when enough context is available.
- If more information is needed after queued questions, explain what is blocked.

Allowed planning actions:
- Read/search files, inspect git status/diff/branch, discover likely tests, read prompts/skills, and queue user questions.
- Do not call write/edit/patch/commit/test-run/setup/terminal/subagent tools.

Required final response:
1. Briefly summarize the code context you inspected.
2. List any queued or still-open custom questions.
3. Provide a repository-specific implementation plan.
4. End with `Critical Files for Implementation` and list exactly 3-5 file paths.
"#
    )
}

fn is_plan_mode_tool(name: &str) -> bool {
    PLAN_MODE_TOOLS.contains(&name)
}

fn planning_critical_files_blocker(content: &str) -> Option<String> {
    let count = extract_critical_files_for_implementation(content).len();
    if (3..=5).contains(&count) {
        None
    } else {
        Some(PLAN_CRITICAL_FILES_BLOCKER.to_string())
    }
}

fn extract_critical_files_for_implementation(content: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        let heading = trimmed.trim_start_matches('#').trim();
        if heading.eq_ignore_ascii_case("Critical Files for Implementation") {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with('#') {
            break;
        }
        if !in_section {
            continue;
        }
        if let Some(file) = critical_file_from_line(trimmed) {
            if !files.contains(&file) {
                files.push(file);
            }
        }
    }
    files
}

fn critical_file_from_line(line: &str) -> Option<String> {
    let item = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| numbered_list_item(line))?
        .trim();
    let candidate = item
        .trim_matches('`')
        .split_whitespace()
        .next()?
        .trim_matches('`')
        .trim_end_matches([':', ',', ';']);
    if candidate.contains('/') || candidate.contains('.') {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn numbered_list_item(line: &str) -> Option<&str> {
    let (prefix, rest) = line.split_once(". ")?;
    if prefix.chars().all(|ch| ch.is_ascii_digit()) {
        Some(rest)
    } else {
        None
    }
}

fn plan_from_planning_document(requirement: &str, document: &str) -> Plan {
    let steps = extract_critical_files_for_implementation(document)
        .into_iter()
        .enumerate()
        .map(|(index, path)| PlanStep {
            id: format!("critical_file_{}", index + 1),
            description: format!("Review or update {path}"),
            status: PlanStepStatus::Pending,
        })
        .collect::<Vec<_>>();
    Plan {
        title: format!("Plan: {}", truncate_chars(requirement, 96)),
        steps,
        updated_at: Utc::now(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionObservation {
    pub state: String,
    pub plan_total: usize,
    pub plan_completed: usize,
    pub plan_in_progress: usize,
    pub plan_failed: usize,
    pub current_step: Option<String>,
    pub latest_test: Option<SessionObservationTest>,
    pub pending_approvals: usize,
    pub open_questions: usize,
    pub tool_calls: usize,
    pub failed_tools: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionObservationTest {
    pub command: String,
    pub passed: bool,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionObservationEnvironment {
    pub tool: String,
    pub target: String,
    pub status: String,
    pub ready: Option<bool>,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionObservationUsage {
    pub provider_turns_started: usize,
    pub provider_turns_completed: usize,
    pub provider_average_elapsed_ms: Option<u128>,
    pub provider_max_elapsed_ms: Option<u128>,
    pub provider_tool_calls: usize,
    pub compacted_turns: usize,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub prompt_cache_hit_tokens: Option<u64>,
    pub prompt_cache_miss_tokens: Option<u64>,
    pub max_request_bytes: Option<usize>,
    pub latest_request_bytes: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMonitor {
    pub observation: SessionObservation,
    pub usage: SessionObservationUsage,
    pub recent_tests: Vec<SessionObservationTest>,
    pub recent_environment: Vec<SessionObservationEnvironment>,
    pub pending_approvals: Vec<SessionObservationApproval>,
    pub open_questions: Vec<SessionObservationQuestion>,
    pub recent_events: Vec<SessionObservationEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionObservationApproval {
    pub id: String,
    pub tool: String,
    pub risk: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionObservationQuestion {
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionObservationEvent {
    pub event_type: String,
    pub created_at: String,
}

pub(crate) fn session_environment_observations_from_tool_calls(
    records: &[ToolCallRecord],
    limit: usize,
) -> Vec<SessionObservationEnvironment> {
    let environment = records
        .iter()
        .filter_map(environment_observation_from_tool_call)
        .collect::<Vec<_>>();
    let skip = environment.len().saturating_sub(limit);
    environment.into_iter().skip(skip).collect()
}

fn environment_observation_from_tool_call(
    record: &ToolCallRecord,
) -> Option<SessionObservationEnvironment> {
    if !matches!(
        record.tool.as_str(),
        "check_environment" | "setup_environment"
    ) {
        return None;
    }
    let target = value_string(&record.output, "target")
        .or_else(|| value_string(&record.input, "target"))
        .unwrap_or_else(|| "auto".to_string());
    let ready = value_bool(&record.output, "ready").or_else(|| {
        record
            .output
            .get("after")
            .and_then(|after| value_bool(after, "ready"))
    });
    let detail = environment_observation_detail(record, ready);
    Some(SessionObservationEnvironment {
        tool: record.tool.clone(),
        target,
        status: environment_observation_status(&record.status, ready).to_string(),
        ready,
        detail,
    })
}

fn environment_observation_status(status: &ToolCallStatus, ready: Option<bool>) -> &'static str {
    match status {
        ToolCallStatus::Succeeded => match ready {
            Some(true) => "ready",
            Some(false) => "needs_setup",
            None => "succeeded",
        },
        ToolCallStatus::Failed => "failed",
        ToolCallStatus::Denied => "denied",
        ToolCallStatus::Running => "running",
        ToolCallStatus::Requested => "requested",
        ToolCallStatus::PolicyChecking => "policy_checking",
        ToolCallStatus::AutoApproved | ToolCallStatus::UserApproved => "approved",
    }
}

fn environment_observation_detail(record: &ToolCallRecord, ready: Option<bool>) -> String {
    if let Some(error) = value_string(&record.output, "error") {
        return format!("error: {error}");
    }
    if let Some(recommended) = value_string(&record.output, "recommended_action").or_else(|| {
        record
            .output
            .get("after")
            .and_then(|after| value_string(after, "recommended_action"))
    }) {
        return format!("recommended: {recommended}");
    }
    if record.tool == "setup_environment" {
        let actions = record
            .output
            .get("actions")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        return format!("actions={actions}");
    }
    ready
        .map(|ready| format!("ready={ready}"))
        .unwrap_or_else(|| format!("{:?}", record.status))
}

fn value_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn value_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

pub(crate) fn session_usage_observation_from_audit_events(
    events: &[AuditEvent],
) -> SessionObservationUsage {
    let mut usage = SessionObservationUsage::default();
    let mut provider_elapsed_ms = 0u128;
    for event in events {
        match event.event_type.as_str() {
            "provider_turn_started" => {
                usage.provider_turns_started += 1;
                if event
                    .payload
                    .pointer("/request/compacted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    usage.compacted_turns += 1;
                }
                if let Some(bytes) = event
                    .payload
                    .pointer("/request/total_bytes")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                {
                    usage.latest_request_bytes = Some(bytes);
                    usage.max_request_bytes =
                        Some(usage.max_request_bytes.unwrap_or_default().max(bytes));
                }
            }
            "provider_turn_completed" => {
                usage.provider_turns_completed += 1;
                let elapsed = event
                    .payload
                    .get("elapsed_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from)
                    .unwrap_or_default();
                provider_elapsed_ms += elapsed;
                usage.provider_max_elapsed_ms = Some(
                    usage
                        .provider_max_elapsed_ms
                        .unwrap_or_default()
                        .max(elapsed),
                );
                usage.provider_tool_calls += event
                    .payload
                    .get("tool_calls")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or_default();
                let response_usage = event.payload.get("usage").unwrap_or(&Value::Null);
                add_optional_usage_value(
                    &mut usage.prompt_tokens,
                    response_usage.get("prompt_tokens").and_then(Value::as_u64),
                );
                add_optional_usage_value(
                    &mut usage.completion_tokens,
                    response_usage
                        .get("completion_tokens")
                        .and_then(Value::as_u64),
                );
                add_optional_usage_value(
                    &mut usage.total_tokens,
                    response_usage.get("total_tokens").and_then(Value::as_u64),
                );
                add_optional_usage_value(
                    &mut usage.prompt_cache_hit_tokens,
                    response_usage
                        .get("prompt_cache_hit_tokens")
                        .and_then(Value::as_u64),
                );
                add_optional_usage_value(
                    &mut usage.prompt_cache_miss_tokens,
                    response_usage
                        .get("prompt_cache_miss_tokens")
                        .and_then(Value::as_u64),
                );
            }
            _ => {}
        }
    }
    if usage.provider_turns_completed > 0 {
        usage.provider_average_elapsed_ms =
            Some(provider_elapsed_ms / usage.provider_turns_completed as u128);
    }
    usage
}

fn add_optional_usage_value(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or_default() + value);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeProgress {
    ProviderStreamStarted,
    AssistantDelta {
        delta: String,
    },
    ProviderTurnStarted {
        iteration: usize,
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
        detail: Option<String>,
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
                "deepcli: provider stream started".to_string()
            }
            RuntimeProgress::AssistantDelta { .. } => "deepcli: assistant streaming".to_string(),
            RuntimeProgress::ProviderTurnStarted {
                iteration,
                message_count,
                tool_count,
                request_kib,
                compacted,
            } => format!(
                "deepcli: provider turn {iteration} (messages={message_count}, tools={tool_count}, request~{request_kib} KiB{})",
                if *compacted { ", compacted" } else { "" }
            ),
            RuntimeProgress::ProviderTurnCompleted {
                elapsed_ms,
                tool_calls,
            } => format!(
                "deepcli: provider response in {:.1}s (tool_calls={tool_calls})",
                *elapsed_ms as f64 / 1000.0
            ),
            RuntimeProgress::ToolStarted { tool, detail } => detail
                .as_ref()
                .filter(|detail| !detail.is_empty())
                .map(|detail| format!("deepcli: running tool {tool}: {detail}"))
                .unwrap_or_else(|| format!("deepcli: running tool {tool}")),
            RuntimeProgress::ToolCompleted { tool, ok, .. } => {
                let status = if *ok { "completed" } else { "failed" };
                format!("deepcli: tool {tool} {status}")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AgentLoopState {
    Initialized,
    PreparingRequest,
    RequestingProvider,
    RecoveringProvider,
    CheckingCompletion,
    DispatchingTools,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AgentLoopTransitionReason {
    StartIteration,
    ContextPrepared,
    ProviderResponded,
    RecoveryAttempted,
    ToolCallsRequested,
    ToolsCompleted,
    CompletionAccepted,
    CompletionBlocked,
    BudgetGuardRecovered,
    ProviderFailed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct AgentLoopTransition {
    iteration: Option<usize>,
    from: AgentLoopState,
    to: AgentLoopState,
    reason: AgentLoopTransitionReason,
    detail: Value,
}

#[derive(Debug, Clone)]
struct AgentLoopTracker {
    state: AgentLoopState,
}

impl AgentLoopTracker {
    fn new() -> Self {
        Self {
            state: AgentLoopState::Initialized,
        }
    }

    fn transition(
        &mut self,
        iteration: Option<usize>,
        to: AgentLoopState,
        reason: AgentLoopTransitionReason,
        detail: Value,
    ) -> AgentLoopTransition {
        let transition = AgentLoopTransition {
            iteration,
            from: self.state,
            to,
            reason,
            detail,
        };
        self.state = to;
        transition
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionAction {
    Accept,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletionDecision {
    action: CompletionAction,
    blockers: Vec<String>,
    follow_up_prompt: Option<String>,
}

impl CompletionDecision {
    fn accept() -> Self {
        Self {
            action: CompletionAction::Accept,
            blockers: Vec::new(),
            follow_up_prompt: None,
        }
    }

    fn continue_with(blockers: Vec<String>) -> Self {
        let blocker_lines = blockers
            .iter()
            .map(|blocker| format!("- {blocker}"))
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            action: CompletionAction::Continue,
            blockers,
            follow_up_prompt: Some(format!(
                "[deepcli completion blocked]\nThe assistant attempted to finish, but completion hooks found unresolved blockers:\n{blocker_lines}\nContinue the agent loop by resolving these blockers with the appropriate tool calls, or explicitly report why user input is required."
            )),
        }
    }

    fn waits_for_user_input(&self) -> bool {
        self.blockers
            .iter()
            .any(|blocker| blocker.starts_with("open user question count:"))
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
            planning_mode_active: false,
            planning_initial_side_question_count: 0,
        })
    }

    pub fn session_id(&self) -> String {
        self.session.id().to_string()
    }

    pub fn session_title(&self) -> Option<&str> {
        self.session.metadata.title.as_deref()
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
            match command {
                SlashCommand::Resume { args } => {
                    return self.handle_resume_command(Self::resume_runtime_id_arg(&args));
                }
                SlashCommand::Rename { args } => {
                    return self.handle_rename_command(args);
                }
                SlashCommand::Model { args } => {
                    return self.handle_model_command(args);
                }
                SlashCommand::Config { args } => {
                    return self.handle_config_command(args);
                }
                SlashCommand::Timeout { args } => {
                    return self.handle_timeout_command(args);
                }
                SlashCommand::Credentials { args } => {
                    return self.handle_credentials_command(args);
                }
                SlashCommand::Cmd { command, attach } => {
                    return self.handle_cmd_command(command, attach).await;
                }
                SlashCommand::Plan { args } if should_run_model_plan(&args) => {
                    return self.handle_model_plan_command(args).await;
                }
                SlashCommand::Quit => {
                    return Ok("bye".to_string());
                }
                _ => {}
            }
            return CommandRouter::handle(
                command,
                CommandContext {
                    workspace: &self.workspace,
                    config: &self.config,
                    registry: &self.registry,
                    executor: &self.executor,
                    session_id: Some(self.session_id()),
                    provider_override: Some(self.provider_name()),
                    allow_interactive_prompts: true,
                },
            )
            .await;
        }
        if is_low_information_input(input) && !self.has_open_user_context()? {
            return self.handle_low_information_input(input);
        }
        self.run_agent_task(input).await
    }

    async fn handle_cmd_command(&mut self, command: String, attach: bool) -> Result<String> {
        self.executor.set_session(Some(self.session.clone()));
        let execution = run_cmd_shell(&self.executor, &command).await?;
        if !attach {
            return Ok(execution.report);
        }

        let response = match self.run_agent_task(&execution.attachment).await {
            Ok(response) => response,
            Err(error) => {
                return Err(anyhow!(
                    "{}\n\nmodel response error:\n{}",
                    execution.report,
                    error
                ));
            }
        };
        Ok(format!(
            "{}\n\nmodel response:\n{}",
            execution.report, response
        ))
    }

    async fn handle_model_plan_command(&mut self, args: Vec<String>) -> Result<String> {
        let requirement = model_plan_requirement(&args)?;
        self.run_planning_task(&requirement).await
    }

    pub fn list_sessions(&self) -> Result<Vec<crate::session::SessionMetadata>> {
        list_resumable_sessions(&self.workspace)
    }

    pub fn recent_session_messages(&self, limit: usize) -> Result<Vec<SessionMessage>> {
        self.session.load_recent_messages(limit)
    }

    pub fn session_messages(&self) -> Result<Vec<SessionMessage>> {
        self.session.load_messages()
    }

    pub fn session_observation(&self) -> Result<SessionObservation> {
        let plan = self.session.load_plan()?;
        let (plan_total, plan_completed, plan_in_progress, plan_failed, current_step) = plan
            .as_ref()
            .map(summarize_plan_observation)
            .unwrap_or((0, 0, 0, 0, None));

        let latest_test = self
            .session
            .load_recent_test_runs(1)?
            .into_iter()
            .last()
            .map(|test| SessionObservationTest {
                command: test.command,
                passed: test.passed,
                exit_code: test.exit_code,
            });

        let pending_approvals = self
            .session
            .load_approval_requests()?
            .iter()
            .filter(|request| request.status == ApprovalStatus::Pending)
            .count();
        let open_questions = self
            .session
            .load_side_questions()?
            .iter()
            .filter(|question| question.status == SideQuestionStatus::Open)
            .count();
        let tools = self.session.load_tool_calls()?;
        let failed_tools = tools
            .iter()
            .filter(|tool| matches!(tool.status, ToolCallStatus::Failed | ToolCallStatus::Denied))
            .count();

        Ok(SessionObservation {
            state: self.state_label(),
            plan_total,
            plan_completed,
            plan_in_progress,
            plan_failed,
            current_step,
            latest_test,
            pending_approvals,
            open_questions,
            tool_calls: tools.len(),
            failed_tools,
        })
    }

    pub fn session_monitor(&self) -> Result<SessionMonitor> {
        let observation = self.session_observation()?;
        let recent_tests = self
            .session
            .load_recent_test_runs(6)?
            .into_iter()
            .map(|test| SessionObservationTest {
                command: test.command,
                passed: test.passed,
                exit_code: test.exit_code,
            })
            .collect();
        let recent_environment =
            session_environment_observations_from_tool_calls(&self.session.load_tool_calls()?, 6);
        let pending_approvals = self
            .session
            .load_approval_requests()?
            .into_iter()
            .filter(|request| request.status == ApprovalStatus::Pending)
            .map(|request| SessionObservationApproval {
                id: request.id.to_string(),
                tool: request.tool,
                risk: format!("{:?}", request.decision.risk),
                reason: request.decision.reason,
            })
            .collect();
        let open_questions = self
            .session
            .load_side_questions()?
            .into_iter()
            .filter(|question| question.status == SideQuestionStatus::Open)
            .map(|question| SessionObservationQuestion {
                id: question.id.to_string(),
                question: question.question,
                options: question.options,
            })
            .collect();
        let events = self.session.load_audit_events()?;
        let usage = session_usage_observation_from_audit_events(&events);
        let skip = events.len().saturating_sub(8);
        let recent_events = events
            .into_iter()
            .skip(skip)
            .map(|event| SessionObservationEvent {
                event_type: event.event_type,
                created_at: event.created_at.format("%H:%M:%S").to_string(),
            })
            .collect();

        Ok(SessionMonitor {
            observation,
            usage,
            recent_tests,
            recent_environment,
            pending_approvals,
            open_questions,
            recent_events,
        })
    }

    pub fn update_current_approval(&mut self, id: &str, approved: bool) -> Result<String> {
        let status = if approved {
            ApprovalStatus::Approved
        } else {
            ApprovalStatus::Denied
        };
        let item = self.session.update_approval_request(id, status)?;
        self.executor.set_session(Some(self.session.clone()));
        let action = if approved { "approved" } else { "denied" };
        Ok(format!(
            "{action} approval request {} for tool {}",
            item.id, item.tool
        ))
    }

    pub fn answer_current_side_question(&mut self, id: &str, answer: &str) -> Result<String> {
        let answer = answer.trim();
        if answer.is_empty() {
            anyhow::bail!("side question answer cannot be empty");
        }
        let item = self.session.answer_side_question(id, answer)?;
        self.executor.set_session(Some(self.session.clone()));
        Ok(format!("answered btw question {}", item.id))
    }

    pub fn resume_session(&mut self, id: &str) -> Result<String> {
        let session = SessionStore::new(&self.workspace).load(id)?;
        let label = session
            .metadata
            .title
            .clone()
            .unwrap_or_else(|| session.metadata.id.to_string());
        self.session = session;
        self.executor.set_session(Some(self.session.clone()));
        Ok(format!("已切换到会话：{label}"))
    }

    pub fn rename_current_session(&mut self, title: &str) -> Result<String> {
        if title.trim().is_empty() {
            return Ok("请提供会话名称，例如：/rename 编译器修复记录".to_string());
        }
        self.session.rename(title)?;
        self.executor.set_session(Some(self.session.clone()));
        Ok(format!("已重命名当前会话为：{}", title.trim()))
    }

    pub fn store_provider_api_key(
        &mut self,
        provider: &str,
        api_key: String,
        force: bool,
    ) -> Result<String> {
        let output = set_credentials_api_key(
            &self.workspace,
            &self.config,
            provider,
            api_key,
            force,
            "hidden prompt",
        )?;
        self.session.append_audit_event(
            "credentials_updated",
            json!({
                "provider": provider,
                "source": "hidden_prompt",
            }),
        )?;
        Ok(output)
    }

    fn handle_resume_command(&mut self, id: Option<String>) -> Result<String> {
        if let Some(id) = id {
            self.resume_session(&id)
        } else {
            Ok(format_session_list(&self.list_sessions()?))
        }
    }

    fn resume_runtime_id_arg(args: &[String]) -> Option<String> {
        args.iter().find(|arg| !arg.starts_with('-')).cloned()
    }

    fn handle_rename_command(&mut self, args: Vec<String>) -> Result<String> {
        let title = args.join(" ");
        self.rename_current_session(&title)
    }

    fn handle_model_command(&mut self, args: Vec<String>) -> Result<String> {
        match args.first().map(String::as_str) {
            None | Some("show" | "list") => handle_model_read_command(
                &self.workspace,
                &self.config,
                &args,
                Some((self.provider_name(), self.model_name())),
            ),
            Some(value) if value.starts_with("--") => handle_model_read_command(
                &self.workspace,
                &self.config,
                &args,
                Some((self.provider_name(), self.model_name())),
            ),
            Some("set") => {
                let (provider, model) = parse_model_set_args(&args)?;
                if !self.config.providers.contains_key(provider) {
                    return Err(anyhow!("provider `{provider}` is not configured"));
                }
                update_project_model_config(&self.workspace, &self.config, provider, model)?;
                self.config.default_provider = provider.to_string();
                if let Some(model) = model {
                    if let Some(provider_config) = self.config.providers.get_mut(provider) {
                        provider_config.acceptance_model = Some(model.to_string());
                    }
                }
                let runtime = self
                    .config
                    .redacted_provider_runtime(&self.workspace, Some(provider))?;
                let active_model = runtime.model.clone();
                self.session
                    .set_provider_model(provider.to_string(), active_model.clone())?;
                self.executor.set_session(Some(self.session.clone()));
                self.session.append_audit_event(
                    "model_updated",
                    json!({
                        "provider": provider,
                        "model": active_model.clone()
                    }),
                )?;
                Ok(format!(
                    "active session provider updated to `{provider}`\nactive session model: {}",
                    active_model.unwrap_or_else(|| "<unset>".to_string())
                ))
            }
            Some(other) => Err(anyhow!("unsupported /model action `{other}`")),
        }
    }

    fn handle_config_command(&mut self, args: Vec<String>) -> Result<String> {
        let changes_project_config = matches!(args.first().map(String::as_str), Some("set"));
        let output = handle_config(&self.workspace, &self.config, args)?;
        if changes_project_config {
            self.config = AppConfig::load_effective(&self.workspace, None)?;
        }
        Ok(output)
    }

    fn handle_timeout_command(&mut self, args: Vec<String>) -> Result<String> {
        let changes_project_config = timeout_args_change_project_config(&args);
        let output = handle_timeout(&self.workspace, &self.config, args)?;
        if changes_project_config {
            self.config = AppConfig::load_effective(&self.workspace, None)?;
            self.session.append_audit_event(
                "timeout_updated",
                json!({
                    "providerTurnTimeoutSeconds": self.config.agent.provider_turn_timeout_seconds,
                }),
            )?;
        }
        Ok(output)
    }

    fn handle_credentials_command(&mut self, args: Vec<String>) -> Result<String> {
        let action = args.first().cloned();
        let provider = args
            .get(1)
            .filter(|value| !value.starts_with('-'))
            .cloned()
            .unwrap_or_else(|| self.provider_name().to_string());
        let output = handle_credentials_with_default(
            &self.workspace,
            &self.config,
            args,
            Some(self.provider_name()),
            true,
        )?;
        match action.as_deref() {
            Some("set") => {
                self.session.append_audit_event(
                    "credentials_updated",
                    json!({
                        "provider": provider,
                        "source": action.unwrap_or_else(|| "unknown".to_string()),
                    }),
                )?;
            }
            Some("remove") => {
                self.session.append_audit_event(
                    "credentials_removed",
                    json!({
                        "provider": provider,
                    }),
                )?;
            }
            _ => {}
        }
        Ok(output)
    }

    fn handle_low_information_input(&mut self, input: &str) -> Result<String> {
        let message = low_information_clarification_message(input);
        self.session.append_message("user", input)?;
        self.session.append_message("assistant", message)?;
        self.session.write_summary(message)?;
        self.session.set_state(SessionState::WaitingUser)?;
        Ok(message.to_string())
    }

    fn has_open_user_context(&self) -> Result<bool> {
        if matches!(
            self.session.metadata.state,
            SessionState::AwaitingApproval | SessionState::Paused | SessionState::WaitingUser
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
        self.run_agent_task_inner(task, task, AgentRunMode::Normal)
            .await
    }

    async fn run_planning_task(&mut self, requirement: &str) -> Result<String> {
        let task = build_model_plan_prompt(requirement);
        self.planning_initial_side_question_count = self.session.load_side_questions()?.len();
        self.planning_mode_active = true;
        let result = self
            .run_agent_task_inner(requirement, &task, AgentRunMode::Planning)
            .await;
        self.planning_mode_active = false;
        result
    }

    async fn run_agent_task_inner(
        &mut self,
        title_task: &str,
        provider_task: &str,
        mode: AgentRunMode,
    ) -> Result<String> {
        self.session.auto_title_from_user_task(title_task)?;
        self.executor.set_session(Some(self.session.clone()));
        let session_context = self.build_session_context()?;
        self.session.set_state(SessionState::ContextLoading)?;
        self.session.append_message("user", provider_task)?;

        let workspace_context = WorkspaceManager::new(&self.workspace)?.collect_context()?;
        if mode == AgentRunMode::Normal
            && self.config.agent.require_plan_for_complex_tasks
            && is_complex_task(title_task)
        {
            self.session.set_state(SessionState::Planning)?;
            self.session.save_plan(&default_plan(title_task))?;
        }

        let provider_runtime = self
            .config
            .provider_runtime(&self.workspace, Some(&self.session.metadata.provider))?;
        let provider = create_provider(provider_runtime)?;
        let mut messages = vec![
            ProviderMessage {
                role: "system".to_string(),
                content: Some(system_prompt(
                    &workspace_context,
                    &self.config,
                    session_context.as_deref(),
                )),
                reasoning_content: None,
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            ProviderMessage {
                role: "user".to_string(),
                content: Some(provider_task.to_string()),
                reasoning_content: None,
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        ];
        let estimated_tokens = provider.count_tokens(&messages);
        let provider_turn_timeout = self.provider_turn_timeout();

        self.session.set_state(SessionState::Executing)?;
        if mode == AgentRunMode::Normal && self.stream_output && !is_complex_task(title_task) {
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
        let mut completion_hook_continuations = 0usize;
        let mut loop_tracker = AgentLoopTracker::new();
        let mut iteration_number = 0usize;
        loop {
            iteration_number = iteration_number.saturating_add(1);
            self.record_agent_loop_transition(
                &mut loop_tracker,
                Some(iteration_number),
                AgentLoopState::PreparingRequest,
                AgentLoopTransitionReason::StartIteration,
                json!({
                    "message_count": messages.len(),
                    "context_tool_calls_before_action": context_tool_calls_before_action,
                    "verification_calls_before_action": verification_calls_before_action,
                }),
            )?;
            let tool_specs = match mode {
                AgentRunMode::Normal => self.registry.tool_specs(),
                AgentRunMode::Planning => self.registry.tool_specs_for_names(PLAN_MODE_TOOLS),
            };
            let context_manager = ContextManager::from_config(&self.config);
            let prepared_context = context_manager
                .prepare(
                    provider.as_ref(),
                    &messages,
                    &tool_specs,
                    provider_turn_timeout,
                )
                .await?;
            let compacted = prepared_context.compacted();
            let context_estimated_tokens = prepared_context.estimated_tokens;
            let context_threshold_tokens = prepared_context.threshold_tokens;
            let microcompacted_tool_results = prepared_context.microcompacted_tool_results;
            let full_compacted = prepared_context.full_compacted;
            let tail_compacted = prepared_context.tail_compacted;
            let compact_boundary = prepared_context.compact_boundary.clone();
            messages = prepared_context.messages;
            let compacted_messages = messages.clone();
            let request_stats = provider_request_stats(&compacted_messages, &tool_specs);
            let full_compact_error = prepared_context.full_compact_error.clone();
            if let Some(boundary) = &compact_boundary {
                self.session.append_compact_boundary(boundary)?;
            }
            self.record_agent_loop_transition(
                &mut loop_tracker,
                Some(iteration_number),
                AgentLoopState::RequestingProvider,
                AgentLoopTransitionReason::ContextPrepared,
                json!({
                    "message_count": request_stats.message_count,
                    "tool_count": request_stats.tool_count,
                    "request_bytes": request_stats.total_bytes,
                    "compacted": compacted,
                    "estimated_tokens": context_estimated_tokens,
                    "threshold_tokens": context_threshold_tokens,
                }),
            )?;
            self.emit_progress(RuntimeProgress::ProviderTurnStarted {
                iteration: iteration_number,
                message_count: request_stats.message_count,
                tool_count: request_stats.tool_count,
                request_kib: request_stats.total_bytes.div_ceil(1024),
                compacted,
            });
            self.session.append_audit_event(
                "provider_turn_started",
                json!({
                    "iteration": iteration_number,
                    "timeout_seconds": provider_turn_timeout.as_secs(),
                    "request": {
                        "message_count": request_stats.message_count,
                        "message_bytes": request_stats.message_bytes,
                        "tool_count": request_stats.tool_count,
                        "tool_bytes": request_stats.tool_bytes,
                        "total_bytes": request_stats.total_bytes,
                        "compacted": compacted,
                        "context": {
                            "estimated_tokens": context_estimated_tokens,
                            "threshold_tokens": context_threshold_tokens,
                            "microcompacted_tool_results": microcompacted_tool_results,
                            "full_compacted": full_compacted,
                            "tail_compacted": tail_compacted,
                            "full_compact_error": full_compact_error,
                            "compact_boundary_id": compact_boundary.as_ref().map(|boundary| boundary.id.to_string())
                        }
                    }
                }),
            )?;
            let started = Instant::now();
            let progress_tx = self.progress_tx.clone();
            let mut on_stream_event = move |event: StreamEvent| {
                if let Some(delta) = event.content_delta.filter(|delta| !delta.is_empty()) {
                    if let Some(tx) = &progress_tx {
                        let _ = tx.send(RuntimeProgress::AssistantDelta { delta });
                    }
                }
            };
            let chat_result = match self
                .chat_with_context_retry_and_streaming_tools(
                    provider.as_ref(),
                    ChatRequest {
                        messages: compacted_messages,
                        tools: tool_specs,
                        json_mode: false,
                    },
                    provider_turn_timeout,
                    &mut on_stream_event,
                    ToolBudgetSnapshot {
                        context_tool_calls_before_action,
                        verification_calls_before_action,
                        context_tool_limit,
                        verification_tool_limit,
                    },
                )
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    let _ = self.record_agent_loop_transition(
                        &mut loop_tracker,
                        Some(iteration_number),
                        AgentLoopState::Failed,
                        AgentLoopTransitionReason::ProviderFailed,
                        json!({ "error": error.to_string() }),
                    );
                    if is_provider_chat_timeout_error(&error) {
                        self.session.set_state(SessionState::Failed)?;
                    }
                    return Err(error);
                }
            };
            if !chat_result.recoveries.is_empty() {
                for recovery in &chat_result.recoveries {
                    self.record_agent_loop_transition(
                        &mut loop_tracker,
                        Some(iteration_number),
                        AgentLoopState::RecoveringProvider,
                        AgentLoopTransitionReason::RecoveryAttempted,
                        json!(recovery),
                    )?;
                }
                messages = chat_result.messages.clone();
            }
            if chat_result.retried_after_context_error {
                self.session.append_audit_event(
                    "provider_context_retry",
                    json!({
                        "reason": "context_length_error",
                        "message_count": chat_result.messages.len()
                    }),
                )?;
            }
            let recoveries = chat_result.recoveries.clone();
            let mut early_tool_results = chat_result.early_tool_results;
            let response = chat_result.response;
            let elapsed = started.elapsed();
            self.emit_progress(RuntimeProgress::ProviderTurnCompleted {
                elapsed_ms: elapsed.as_millis(),
                tool_calls: response.tool_calls.len(),
            });
            self.session.append_audit_event(
                "provider_turn_completed",
                json!({
                    "iteration": iteration_number,
                    "elapsed_ms": elapsed.as_millis(),
                    "tool_calls": response.tool_calls.len(),
                    "usage": response.usage,
                    "recoveries": recoveries
                }),
            )?;

            if response.tool_calls.is_empty() {
                self.record_agent_loop_transition(
                    &mut loop_tracker,
                    Some(iteration_number),
                    AgentLoopState::CheckingCompletion,
                    AgentLoopTransitionReason::ProviderResponded,
                    json!({ "tool_calls": 0 }),
                )?;
                let mut provider_content = response.content.unwrap_or_default();
                append_usage_report(
                    &mut provider_content,
                    &response.usage,
                    self.config.usage.token_warning_threshold,
                );
                let decision = self.evaluate_completion_hooks(
                    &provider_content,
                    iteration_number,
                    completion_hook_continuations,
                )?;
                if decision.action == CompletionAction::Continue {
                    if decision.waits_for_user_input() {
                        let content = self.pause_for_user_questions()?;
                        self.record_agent_loop_transition(
                            &mut loop_tracker,
                            Some(iteration_number),
                            AgentLoopState::Completed,
                            AgentLoopTransitionReason::CompletionBlocked,
                            json!({ "waiting_for_user": true }),
                        )?;
                        return Ok(content);
                    }
                    self.session.append_audit_event(
                        "completion_hook_blocked",
                        json!({
                            "iteration": iteration_number,
                            "blockers": decision.blockers.clone(),
                            "continuation_count": completion_hook_continuations + 1
                        }),
                    )?;
                    messages.push(ProviderMessage {
                        role: "assistant".to_string(),
                        content: Some(provider_content),
                        reasoning_content: response.reasoning_content.clone(),
                        name: None,
                        tool_call_id: None,
                        tool_calls: None,
                    });
                    messages.push(ProviderMessage {
                        role: "user".to_string(),
                        content: decision.follow_up_prompt,
                        reasoning_content: None,
                        name: None,
                        tool_call_id: None,
                        tool_calls: None,
                    });
                    completion_hook_continuations += 1;
                    self.record_agent_loop_transition(
                        &mut loop_tracker,
                        Some(iteration_number),
                        AgentLoopState::PreparingRequest,
                        AgentLoopTransitionReason::CompletionBlocked,
                        json!({ "continuation_count": completion_hook_continuations }),
                    )?;
                    continue;
                }

                if mode == AgentRunMode::Planning {
                    self.save_planning_artifacts(title_task, &provider_content)?;
                }
                let content = with_token_estimate(provider_content, estimated_tokens);
                self.session.append_message("assistant", &content)?;
                self.session
                    .update_plan_step("review", PlanStepStatus::Completed)?;
                self.session.complete_pending_plan_steps()?;
                self.session.set_state(SessionState::Completed)?;
                self.session.write_summary(&content)?;
                self.record_agent_loop_transition(
                    &mut loop_tracker,
                    Some(iteration_number),
                    AgentLoopState::Completed,
                    AgentLoopTransitionReason::CompletionAccepted,
                    json!({ "content_chars": content.chars().count() }),
                )?;
                return Ok(content);
            }

            self.record_agent_loop_transition(
                &mut loop_tracker,
                Some(iteration_number),
                AgentLoopState::DispatchingTools,
                AgentLoopTransitionReason::ToolCallsRequested,
                json!({ "tool_calls": response.tool_calls.len() }),
            )?;
            let assistant_tool_message = ProviderMessage {
                role: "assistant".to_string(),
                content: response.content.clone(),
                reasoning_content: response.reasoning_content.clone(),
                name: None,
                tool_call_id: None,
                tool_calls: Some(response.tool_calls.clone()),
            };
            self.record_provider_message_transcript(&assistant_tool_message)?;
            messages.push(assistant_tool_message);

            let mut budget_skipped_this_turn = 0usize;
            let turn_tool_calls = response.tool_calls;
            for batch in tool_call_batches(&self.registry, &turn_tool_calls) {
                match batch {
                    ToolCallBatch::Parallel(range)
                        if batch_fits_tool_budgets(
                            &turn_tool_calls[range.clone()],
                            context_tool_calls_before_action,
                            verification_calls_before_action,
                            context_tool_limit,
                            verification_tool_limit,
                        ) =>
                    {
                        let calls = &turn_tool_calls[range];
                        let mut outputs_by_id = BTreeMap::new();
                        let pending_calls = calls
                            .iter()
                            .filter(|call| !early_tool_result_matches(&early_tool_results, call))
                            .cloned()
                            .collect::<Vec<_>>();
                        if !pending_calls.is_empty() {
                            let pending_outputs =
                                self.execute_parallel_tool_calls(&pending_calls).await?;
                            for (call, output) in pending_calls.iter().zip(pending_outputs) {
                                outputs_by_id.insert(call.id.clone(), output);
                            }
                        }
                        for call in calls {
                            let tool_output =
                                take_matching_early_tool_output(&mut early_tool_results, call)
                                    .or_else(|| outputs_by_id.remove(&call.id))
                                    .unwrap_or_else(|| {
                                        format!(
                                            "tool `{}` failed: missing tool execution output",
                                            call.function.name
                                        )
                                    });
                            let tool_failed = tool_output_indicates_failure(&tool_output);
                            update_tool_budget_counters(
                                call,
                                tool_failed,
                                &mut context_tool_calls_before_action,
                                &mut verification_calls_before_action,
                            );
                            self.push_tool_provider_message(&mut messages, call, tool_output)?;
                        }
                    }
                    ToolCallBatch::Parallel(range) => {
                        for call in &turn_tool_calls[range] {
                            let tool_output = if let Some(output) =
                                take_matching_early_tool_output(&mut early_tool_results, call)
                            {
                                output
                            } else {
                                self.execute_serial_tool_call_with_budgets(
                                    call,
                                    context_tool_calls_before_action,
                                    verification_calls_before_action,
                                    context_tool_limit,
                                    verification_tool_limit,
                                    &mut budget_skipped_this_turn,
                                )
                                .await?
                            };
                            let tool_failed = tool_output_indicates_failure(&tool_output);
                            update_tool_budget_counters(
                                call,
                                tool_failed,
                                &mut context_tool_calls_before_action,
                                &mut verification_calls_before_action,
                            );
                            self.push_tool_provider_message(&mut messages, call, tool_output)?;
                        }
                    }
                    ToolCallBatch::Serial(index) => {
                        let call = &turn_tool_calls[index];
                        let tool_output = if let Some(output) =
                            take_matching_early_tool_output(&mut early_tool_results, call)
                        {
                            output
                        } else {
                            self.execute_serial_tool_call_with_budgets(
                                call,
                                context_tool_calls_before_action,
                                verification_calls_before_action,
                                context_tool_limit,
                                verification_tool_limit,
                                &mut budget_skipped_this_turn,
                            )
                            .await?
                        };
                        let tool_failed = tool_output_indicates_failure(&tool_output);
                        update_tool_budget_counters(
                            call,
                            tool_failed,
                            &mut context_tool_calls_before_action,
                            &mut verification_calls_before_action,
                        );
                        self.push_tool_provider_message(&mut messages, call, tool_output)?;
                    }
                }
            }
            self.record_agent_loop_transition(
                &mut loop_tracker,
                Some(iteration_number),
                AgentLoopState::PreparingRequest,
                AgentLoopTransitionReason::ToolsCompleted,
                json!({ "tool_calls": turn_tool_calls.len() }),
            )?;

            if mode == AgentRunMode::Planning && self.has_open_side_questions()? {
                let content = self.pause_for_user_questions()?;
                self.record_agent_loop_transition(
                    &mut loop_tracker,
                    Some(iteration_number),
                    AgentLoopState::Completed,
                    AgentLoopTransitionReason::CompletionBlocked,
                    json!({ "waiting_for_user": true }),
                )?;
                return Ok(content);
            }

            if budget_skipped_this_turn > 0 {
                consecutive_budget_skipped_turns += 1;
                if consecutive_budget_skipped_turns >= budget_skip_turn_limit {
                    self.record_agent_loop_transition(
                        &mut loop_tracker,
                        Some(iteration_number),
                        AgentLoopState::RecoveringProvider,
                        AgentLoopTransitionReason::BudgetGuardRecovered,
                        json!({ "turns": consecutive_budget_skipped_turns }),
                    )?;
                    messages.push(ProviderMessage {
                        role: "user".to_string(),
                        content: Some(budget_skip_recovery_prompt(
                            consecutive_budget_skipped_turns,
                        )),
                        reasoning_content: None,
                        name: None,
                        tool_call_id: None,
                        tool_calls: None,
                    });
                    consecutive_budget_skipped_turns = 0;
                }
            } else {
                consecutive_budget_skipped_turns = 0;
            }
        }
    }

    fn save_planning_artifacts(&self, requirement: &str, document: &str) -> Result<()> {
        self.session.write_plan_document(document)?;
        self.session
            .save_plan(&plan_from_planning_document(requirement, document))?;
        self.session.append_audit_event(
            "plan_document_saved",
            json!({
                "path": "plan.md",
                "critical_files": extract_critical_files_for_implementation(document),
            }),
        )
    }

    fn has_open_side_questions(&self) -> Result<bool> {
        Ok(self
            .session
            .load_side_questions()?
            .iter()
            .any(|question| question.status == SideQuestionStatus::Open))
    }

    fn pause_for_user_questions(&mut self) -> Result<String> {
        let open_questions = self
            .session
            .load_side_questions()?
            .into_iter()
            .filter(|question| question.status == SideQuestionStatus::Open)
            .collect::<Vec<_>>();
        let content = if open_questions.len() == 1 {
            format!("等待用户回答 plan 采访问题：{}", open_questions[0].question)
        } else {
            format!("等待用户回答 {} 个 plan 采访问题。", open_questions.len())
        };
        self.session.append_message("assistant", &content)?;
        self.session.set_state(SessionState::WaitingUser)?;
        self.session.write_summary(&content)?;
        Ok(content)
    }

    fn provider_turn_timeout(&self) -> Duration {
        Duration::from_secs(self.config.agent.provider_turn_timeout_seconds.max(1))
    }

    fn build_session_context(&self) -> Result<Option<String>> {
        SessionContextManager::new(&self.session).render()
    }

    fn emit_progress(&self, event: RuntimeProgress) {
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(event);
        } else {
            eprintln!("{}", event.plain_text());
        }
    }

    fn record_agent_loop_transition(
        &self,
        tracker: &mut AgentLoopTracker,
        iteration: Option<usize>,
        to: AgentLoopState,
        reason: AgentLoopTransitionReason,
        detail: Value,
    ) -> Result<()> {
        let transition = tracker.transition(iteration, to, reason, detail);
        self.session
            .append_audit_event("agent_loop_transition", serde_json::to_value(transition)?)?;
        Ok(())
    }

    fn record_provider_message_transcript(&self, message: &ProviderMessage) -> Result<()> {
        let is_assistant_tool_call = message.role == "assistant"
            && message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| !calls.is_empty());
        let is_tool_result = message.role == "tool";
        if !is_assistant_tool_call && !is_tool_result {
            return Ok(());
        }
        self.session
            .append_provider_transcript(&provider_message_to_transcript_record(message))
    }

    fn push_tool_provider_message(
        &self,
        messages: &mut Vec<ProviderMessage>,
        call: &ToolCall,
        output: String,
    ) -> Result<()> {
        let message = tool_provider_message(call, output);
        self.record_provider_message_transcript(&message)?;
        messages.push(message);
        Ok(())
    }

    fn record_tool_execution_recovery_context(
        &self,
        call: &ToolCall,
        execution: &ToolExecution,
    ) -> Result<()> {
        if let Some(snapshot) = file_history_snapshot_from_execution(call, execution) {
            self.session.append_file_history_snapshot(&snapshot)?;
        }
        Ok(())
    }

    fn evaluate_completion_hooks(
        &self,
        content: &str,
        _iteration: usize,
        continuation_count: usize,
    ) -> Result<CompletionDecision> {
        let mut blockers = Vec::new();
        if self.planning_mode_active {
            if let Some(blocker) = planning_critical_files_blocker(content) {
                blockers.push(blocker);
            }
            if self.session.load_side_questions()?.len()
                <= self.planning_initial_side_question_count
            {
                blockers.push(PLAN_USER_QUESTION_BLOCKER.to_string());
            }
        }

        let pending_approvals = self
            .session
            .load_approval_requests()?
            .iter()
            .filter(|request| request.status == ApprovalStatus::Pending)
            .count();
        if pending_approvals > 0 {
            blockers.push(format!(
                "pending approval request count: {pending_approvals}"
            ));
        }

        let open_questions = self
            .session
            .load_side_questions()?
            .iter()
            .filter(|question| question.status == SideQuestionStatus::Open)
            .count();
        if open_questions > 0 {
            blockers.push(format!("open user question count: {open_questions}"));
        }

        let failed_steps = self
            .session
            .load_plan()?
            .map(|plan| {
                plan.steps
                    .into_iter()
                    .filter(|step| step.status == PlanStepStatus::Failed)
                    .map(|step| step.id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !failed_steps.is_empty() {
            blockers.push(format!("failed plan steps: {}", failed_steps.join(", ")));
        }

        let non_overridable_blocker = blockers.iter().any(|blocker| {
            blocker.starts_with("open user question count:")
                || blocker == PLAN_USER_QUESTION_BLOCKER
        });
        if blockers.is_empty()
            || (continuation_count >= completion_hook_continuation_limit()
                && !non_overridable_blocker)
        {
            Ok(CompletionDecision::accept())
        } else {
            Ok(CompletionDecision::continue_with(blockers))
        }
    }

    async fn chat_with_context_retry_and_streaming_tools<F>(
        &mut self,
        provider: &dyn ProviderClient,
        request: ChatRequest,
        provider_turn_timeout: Duration,
        on_stream_event: &mut F,
        tool_budget: ToolBudgetSnapshot,
    ) -> Result<ContextRetryChatResult>
    where
        F: FnMut(StreamEvent) + Send,
    {
        let tools = request.tools.clone();
        let json_mode = request.json_mode;
        let mut messages = request.messages;
        let mut recovery_state = AgentRecoveryState::new();
        let mut recoveries: Vec<AgentRecoveryEvent> = Vec::new();
        let mut early_tool_results = BTreeMap::new();

        loop {
            let current_request = ChatRequest {
                messages: messages.clone(),
                tools: tools.clone(),
                json_mode,
            };
            match self
                .provider_chat_with_streaming_tools(
                    provider,
                    current_request,
                    provider_turn_timeout,
                    on_stream_event,
                    tool_budget,
                    &mut early_tool_results,
                )
                .await
            {
                Ok(response) => {
                    let retried_after_context_error = recoveries
                        .iter()
                        .any(|event| event.kind == AgentRecoveryKind::PromptTooLong);
                    return Ok(ContextRetryChatResult {
                        response,
                        messages,
                        retried_after_context_error,
                        recoveries,
                        early_tool_results,
                    });
                }
                Err(error) => {
                    let Some(kind) = agent_recovery_kind_for_error(&error) else {
                        return Err(error);
                    };
                    let Some(attempt) = recovery_state.register_attempt(kind) else {
                        return Err(error);
                    };
                    let recovered_messages = recover_messages_after_provider_error(kind, &messages);
                    if recovered_messages == messages {
                        return Err(error);
                    }
                    if kind == AgentRecoveryKind::PromptTooLong {
                        self.session
                            .append_compact_boundary(&CompactBoundaryRecord {
                                id: uuid::Uuid::new_v4(),
                                reason: "context_retry_prompt_too_long".to_string(),
                                summary: "The provider rejected the request as too long, so older completed assistant/tool exchange groups were omitted before retrying."
                                    .to_string(),
                                omitted_group_count: message_groups_omitted_after_compaction(
                                    &messages,
                                    &recovered_messages,
                                ),
                                message_count_before: messages.len(),
                                message_count_after: recovered_messages.len(),
                                retained_segment: provider_messages_to_retained_segment(
                                    &recovered_messages,
                                ),
                                created_at: Utc::now(),
                            })?;
                    }
                    messages = recovered_messages;
                    recoveries.push(AgentRecoveryEvent {
                        kind,
                        attempt,
                        message_count: messages.len(),
                    });
                }
            }
        }
    }

    async fn provider_chat_with_streaming_tools<F>(
        &mut self,
        provider: &dyn ProviderClient,
        request: ChatRequest,
        provider_turn_timeout: Duration,
        on_stream_event: &mut F,
        tool_budget: ToolBudgetSnapshot,
        early_tool_results: &mut BTreeMap<String, EarlyToolResult>,
    ) -> Result<ChatResponse>
    where
        F: FnMut(StreamEvent) + Send,
    {
        let (stream_tx, stream_rx) = mpsc::channel::<StreamEvent>();
        let mut callback = |event: StreamEvent| {
            on_stream_event(event.clone());
            if event.tool_call_completed.is_some() {
                let _ = stream_tx.send(event);
            }
        };
        let provider_future = timeout(
            provider_turn_timeout,
            provider.chat_with_stream_events(request, Some(&mut callback)),
        );
        tokio::pin!(provider_future);
        let mut early_counts = StreamingToolBudgetCounters::default();

        loop {
            while let Ok(event) = stream_rx.try_recv() {
                self.handle_streaming_tool_event(
                    event,
                    early_tool_results,
                    tool_budget,
                    &mut early_counts,
                )
                .await?;
            }

            tokio::select! {
                result = &mut provider_future => {
                    while let Ok(event) = stream_rx.try_recv() {
                        self.handle_streaming_tool_event(
                            event,
                            early_tool_results,
                            tool_budget,
                            &mut early_counts,
                        ).await?;
                    }
                    return match result {
                        Ok(response) => response,
                        Err(_) => Err(anyhow!(
                            "provider chat timed out after {} seconds",
                            provider_turn_timeout.as_secs()
                        )),
                    };
                }
                _ = tokio::time::sleep(Duration::from_millis(1)) => {}
            }
        }
    }

    async fn handle_streaming_tool_event(
        &mut self,
        event: StreamEvent,
        early_tool_results: &mut BTreeMap<String, EarlyToolResult>,
        tool_budget: ToolBudgetSnapshot,
        early_counts: &mut StreamingToolBudgetCounters,
    ) -> Result<()> {
        let Some(call) = event.tool_call_completed else {
            return Ok(());
        };
        if early_tool_results.contains_key(&call.id)
            || !is_parallel_safe_call(&self.registry, &call)
        {
            return Ok(());
        }
        if is_context_gathering_call(&call)
            && tool_budget.context_tool_calls_before_action + early_counts.context_calls
                >= tool_budget.context_tool_limit
        {
            return Ok(());
        }
        if is_verification_call(&call)
            && tool_budget.verification_calls_before_action + early_counts.verification_calls
                >= tool_budget.verification_tool_limit
        {
            return Ok(());
        }

        self.session.append_audit_event(
            "streaming_tool_execution_started",
            json!({
                "tool": call.function.name,
                "tool_call_id": call.id
            }),
        )?;
        let output = self.execute_tool_call(&call).await?;
        let tool_failed = tool_output_indicates_failure(&output);
        update_tool_budget_counters(
            &call,
            tool_failed,
            &mut early_counts.context_calls,
            &mut early_counts.verification_calls,
        );
        self.session.append_audit_event(
            "streaming_tool_execution_completed",
            json!({
                "tool": call.function.name,
                "tool_call_id": call.id,
                "failed": tool_failed
            }),
        )?;
        early_tool_results.insert(call.id.clone(), EarlyToolResult { call, output });
        Ok(())
    }

    async fn execute_tool_call(&mut self, call: &ToolCall) -> Result<String> {
        if !self.registry.has(&call.function.name) {
            return Err(anyhow!(
                "provider requested unknown tool `{}`",
                call.function.name
            ));
        }
        if self.planning_mode_active && !is_plan_mode_tool(&call.function.name) {
            let output = format!(
                "tool `{}` skipped: not available in DeepCLI plan mode. Use read-only exploration tools or ask_user_question.",
                call.function.name
            );
            self.session.append_audit_event(
                "tool_skipped_plan_mode",
                json!({ "tool": call.function.name, "reason": "not_available_in_plan_mode" }),
            )?;
            self.emit_progress(RuntimeProgress::ToolCompleted {
                tool: call.function.name.clone(),
                ok: false,
                summary: truncate_progress_detail(&output),
            });
            return Ok(output);
        }
        let progress_detail = tool_call_progress_detail(call);
        self.emit_progress(RuntimeProgress::ToolStarted {
            tool: call.function.name.clone(),
            detail: progress_detail.clone(),
        });
        self.session.append_audit_event(
            "tool_started",
            json!({ "tool": call.function.name, "detail": progress_detail }),
        )?;
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
        self.record_tool_execution_recovery_context(call, &execution)?;
        self.emit_progress(RuntimeProgress::ToolCompleted {
            tool: call.function.name.clone(),
            ok: true,
            summary: truncate_progress_detail(&execution.content),
        });
        Ok(execution.prompt_content())
    }

    async fn execute_serial_tool_call_with_budgets(
        &mut self,
        call: &ToolCall,
        context_tool_calls_before_action: usize,
        verification_calls_before_action: usize,
        context_tool_limit: usize,
        verification_tool_limit: usize,
        budget_skipped_this_turn: &mut usize,
    ) -> Result<String> {
        if call.function.name == "run_tests" {
            self.session.set_state(SessionState::Testing)?;
        }
        let tool_output = if is_context_gathering_call(call)
            && context_tool_calls_before_action >= context_tool_limit
        {
            *budget_skipped_this_turn += 1;
            format!(
                "tool `{}` skipped: context-gathering budget exceeded after {} context-only tool calls without a patch or verification action. Stop gathering context and either apply a focused patch, run a focused verification command, or report the concrete blocker.",
                call.function.name, context_tool_limit
            )
        } else if is_verification_call(call)
            && verification_calls_before_action >= verification_tool_limit
        {
            *budget_skipped_this_turn += 1;
            format!(
                "tool `{}` skipped: verification budget exceeded after {} verification-only tool calls without a project write. Stop running more tests, apply a focused patch to the current failure, or report the concrete blocker.",
                call.function.name, verification_tool_limit
            )
        } else {
            self.execute_tool_call(call).await?
        };
        if call.function.name == "run_tests" {
            self.session.set_state(SessionState::Executing)?;
        }
        Ok(tool_output)
    }

    async fn execute_parallel_tool_calls(&mut self, calls: &[ToolCall]) -> Result<Vec<String>> {
        if self.planning_mode_active {
            let mut outputs = Vec::with_capacity(calls.len());
            for call in calls {
                outputs.push(self.execute_tool_call(call).await?);
            }
            return Ok(outputs);
        }
        for call in calls {
            if !self.registry.has(&call.function.name) {
                return Err(anyhow!(
                    "provider requested unknown tool `{}`",
                    call.function.name
                ));
            }
            let progress_detail = tool_call_progress_detail(call);
            self.emit_progress(RuntimeProgress::ToolStarted {
                tool: call.function.name.clone(),
                detail: progress_detail.clone(),
            });
            self.session.append_audit_event(
                "tool_started",
                json!({ "tool": call.function.name, "detail": progress_detail, "parallel": true }),
            )?;
        }

        let futures = calls.iter().map(|call| {
            self.executor
                .execute(&call.function.name, call.function.arguments.clone())
        });
        let results = join_all(futures).await;
        let mut outputs = Vec::with_capacity(results.len());
        for (call, result) in calls.iter().zip(results) {
            match result {
                Ok(execution) => {
                    self.update_plan_after_tool(
                        &call.function.name,
                        execution.raw.get("passed").and_then(|v| v.as_bool()),
                    )?;
                    self.record_tool_execution_recovery_context(call, &execution)?;
                    self.emit_progress(RuntimeProgress::ToolCompleted {
                        tool: call.function.name.clone(),
                        ok: true,
                        summary: truncate_progress_detail(&execution.content),
                    });
                    outputs.push(execution.prompt_content());
                }
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
                    outputs.push(output);
                }
                Err(error) => {
                    self.session.append_audit_event(
                        "tool_failed",
                        json!({
                            "tool": call.function.name,
                            "error": error.to_string(),
                            "parallel": true
                        }),
                    )?;
                    let output = format!("tool `{}` failed: {error}", call.function.name);
                    self.emit_progress(RuntimeProgress::ToolCompleted {
                        tool: call.function.name.clone(),
                        ok: false,
                        summary: truncate_progress_detail(&output),
                    });
                    outputs.push(output);
                }
            }
        }
        Ok(outputs)
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

struct SessionContextManager<'a> {
    session: &'a Session,
}

impl<'a> SessionContextManager<'a> {
    fn new(session: &'a Session) -> Self {
        Self { session }
    }

    fn render(&self) -> Result<Option<String>> {
        let boundary = self.session.load_latest_compact_boundary()?;
        let mut sections = Vec::new();

        if let Some(section) = self.render_summary(boundary.as_ref())? {
            sections.push(section);
        }
        if let Some(section) = self.render_goal()? {
            sections.push(section);
        }
        if let Some(section) = self.render_plan()? {
            sections.push(section);
        }
        if let Some(section) = self.render_side_questions()? {
            sections.push(section);
        }
        if let Some(section) = self.render_pending_approvals()? {
            sections.push(section);
        }
        if let Some(section) = self.render_compact_boundary(boundary.as_ref()) {
            sections.push(section);
        }
        if let Some(section) = self.render_provider_transcript(boundary.as_ref())? {
            sections.push(section);
        }
        if let Some(section) = self.render_recent_tool_findings()? {
            sections.push(section);
        }
        if let Some(section) = self.render_file_history()? {
            sections.push(section);
        }
        if let Some(section) = self.render_latest_tests()? {
            sections.push(section);
        }
        if let Some(section) = self.render_diff_summary()? {
            sections.push(section);
        }
        if let Some(section) = self.render_recent_conversation(boundary.as_ref())? {
            sections.push(section);
        }

        if sections.is_empty() {
            return Ok(None);
        }
        Ok(Some(truncate_chars(
            &sections.join("\n\n"),
            SESSION_CONTEXT_TOTAL_CHARS,
        )))
    }

    fn render_summary(&self, boundary: Option<&CompactBoundaryRecord>) -> Result<Option<String>> {
        if let Some(boundary) = boundary.filter(|boundary| !boundary.summary.trim().is_empty()) {
            return Ok(Some(format!(
                "Summary:\nLast saved summary:\n{}",
                truncate_chars(&boundary.summary, SESSION_CONTEXT_MESSAGE_CHARS)
            )));
        }
        let Some(summary) = self.session.load_summary()? else {
            return Ok(None);
        };
        let summary = summary.trim();
        if summary.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!(
            "Summary:\nLast saved summary:\n{}",
            truncate_chars(summary, SESSION_CONTEXT_MESSAGE_CHARS)
        )))
    }

    fn render_goal(&self) -> Result<Option<String>> {
        let Some(goal) = self.session.load_goal()? else {
            return Ok(None);
        };
        if goal.status != GoalStatus::Active {
            return Ok(None);
        }
        Ok(Some(render_goal_context(&goal)))
    }

    fn render_plan(&self) -> Result<Option<String>> {
        let Some(plan) = self.session.load_plan()? else {
            return Ok(None);
        };
        if plan.steps.is_empty() {
            return Ok(None);
        }
        let steps = plan
            .steps
            .iter()
            .map(|step| format!("- {:?}: {} ({})", step.status, step.description, step.id))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Some(format!(
            "Plan:\nCurrent saved plan: {}\n{steps}",
            plan.title
        )))
    }

    fn render_side_questions(&self) -> Result<Option<String>> {
        let questions = self
            .session
            .load_side_questions()?
            .into_iter()
            .filter(|question| question.status != SideQuestionStatus::Cleared)
            .collect::<Vec<_>>();
        if questions.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!(
            "Side questions:\n{}",
            questions
                .iter()
                .map(format_side_question_context)
                .collect::<Vec<_>>()
                .join("\n")
        )))
    }

    fn render_pending_approvals(&self) -> Result<Option<String>> {
        let approvals = self
            .session
            .load_approval_requests()?
            .into_iter()
            .filter(|approval| approval.status == ApprovalStatus::Pending)
            .collect::<Vec<_>>();
        if approvals.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!(
            "Pending approvals:\n{}",
            approvals
                .iter()
                .map(format_approval_context)
                .collect::<Vec<_>>()
                .join("\n")
        )))
    }

    fn render_compact_boundary(&self, boundary: Option<&CompactBoundaryRecord>) -> Option<String> {
        let boundary = boundary?;
        let retained = boundary
            .retained_segment
            .iter()
            .take(8)
            .map(format_provider_transcript_record)
            .collect::<Vec<_>>();
        Some(format!(
            "Compact boundary:\nreason={}\nomitted_groups={}\nmessages_before={}\nmessages_after={}\ncreated_at={}\nretained_segment:\n{}",
            boundary.reason,
            boundary.omitted_group_count,
            boundary.message_count_before,
            boundary.message_count_after,
            boundary.created_at.to_rfc3339(),
            if retained.is_empty() {
                "- <none>".to_string()
            } else {
                retained.join("\n")
            }
        ))
    }

    fn render_provider_transcript(
        &self,
        boundary: Option<&CompactBoundaryRecord>,
    ) -> Result<Option<String>> {
        let mut records = boundary
            .map(|boundary| boundary.retained_segment.clone())
            .unwrap_or_default();
        records.extend(
            self.session
                .load_provider_transcript()?
                .into_iter()
                .filter(|record| {
                    boundary.is_none_or(|boundary| record.created_at > boundary.created_at)
                }),
        );
        if records.is_empty() {
            return Ok(None);
        }
        let recovered = recovered_provider_transcript(records);
        let skip = recovered.len().saturating_sub(24);
        Ok(Some(format!(
            "Provider transcript:\n{}",
            recovered
                .into_iter()
                .skip(skip)
                .map(|record| format_provider_transcript_record(&record))
                .collect::<Vec<_>>()
                .join("\n")
        )))
    }

    fn render_recent_tool_findings(&self) -> Result<Option<String>> {
        let records = self.session.load_recent_tool_calls(10)?;
        let findings = records
            .iter()
            .filter(|record| {
                matches!(
                    record.status,
                    ToolCallStatus::Succeeded | ToolCallStatus::Failed | ToolCallStatus::Denied
                )
            })
            .map(format_tool_finding_context)
            .collect::<Vec<_>>();
        if findings.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!(
            "Recent tool findings:\n{}",
            findings.join("\n")
        )))
    }

    fn render_file_history(&self) -> Result<Option<String>> {
        let snapshots = self.session.load_recent_file_history_snapshots(12)?;
        if snapshots.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!(
            "File history snapshots:\n{}",
            snapshots
                .iter()
                .map(format_file_history_context)
                .collect::<Vec<_>>()
                .join("\n")
        )))
    }

    fn render_latest_tests(&self) -> Result<Option<String>> {
        let tests = self.session.load_recent_test_runs(6)?;
        if tests.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!(
            "Latest tests:\n{}",
            tests
                .iter()
                .map(format_test_context)
                .collect::<Vec<_>>()
                .join("\n")
        )))
    }

    fn render_diff_summary(&self) -> Result<Option<String>> {
        let diffs = self.session.load_recent_diffs(6)?;
        if diffs.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!(
            "Diff summary:\n{}",
            diffs
                .iter()
                .map(|diff| {
                    let additions = diff
                        .content
                        .lines()
                        .filter(|line| line.starts_with('+'))
                        .count();
                    let deletions = diff
                        .content
                        .lines()
                        .filter(|line| line.starts_with('-'))
                        .count();
                    format!(
                        "- {} at {} (+{} -{})",
                        diff.name,
                        diff.modified_at.to_rfc3339(),
                        additions,
                        deletions
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        )))
    }

    fn render_recent_conversation(
        &self,
        boundary: Option<&CompactBoundaryRecord>,
    ) -> Result<Option<String>> {
        let mut messages = if let Some(boundary) = boundary {
            self.session
                .load_messages()?
                .into_iter()
                .filter(|message| message.created_at > boundary.created_at)
                .collect::<Vec<_>>()
        } else {
            self.session
                .load_recent_messages(SESSION_CONTEXT_MESSAGE_LIMIT)?
        };
        let skip = messages.len().saturating_sub(SESSION_CONTEXT_MESSAGE_LIMIT);
        messages = messages.into_iter().skip(skip).collect();
        let recent = messages
            .into_iter()
            .filter(|message| !message.content.trim().is_empty())
            .map(format_session_message_context)
            .collect::<Vec<_>>();
        if recent.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!(
            "Recent conversation messages:\n{}",
            recent.join("\n")
        )))
    }
}

fn render_goal_context(goal: &GoalContract) -> String {
    let sources = goal
        .source_requirements
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    let stop_conditions = goal
        .stop_conditions
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    let acceptance = goal
        .acceptance_commands
        .iter()
        .map(|item| format!("- `{item}`"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Goal:\nActive goal contract:\nObjective: {}\nRequirement sources:\n{}\nStop conditions:\n{}\nAcceptance commands:\n{}\nYou must not claim this goal is complete or stop the implementation loop until the objective is achieved, all explicit requirements are verified, all acceptance commands pass, and residual risks are reported.",
        truncate_chars(&goal.objective, SESSION_CONTEXT_MESSAGE_CHARS),
        if sources.is_empty() { "- <none>".to_string() } else { sources },
        if stop_conditions.is_empty() { "- <none>".to_string() } else { stop_conditions },
        if acceptance.is_empty() { "- <none>".to_string() } else { acceptance }
    )
}

fn format_side_question_context(question: &SideQuestion) -> String {
    let mut line = format!(
        "- {} [{:?}] at {}: {}",
        question.id,
        question.status,
        question.created_at.to_rfc3339(),
        truncate_chars(&question.question, 600)
    );
    if !question.options.is_empty() {
        line.push_str(&format!("\n  options: {}", question.options.join(" | ")));
    }
    if let Some(answer) = question.answer.as_deref() {
        line.push_str(&format!("\n  answer: {}", truncate_chars(answer, 600)));
    }
    line
}

fn format_approval_context(approval: &ApprovalRequest) -> String {
    format!(
        "- {} tool={} risk={:?}: {}",
        approval.id,
        approval.tool,
        approval.decision.risk,
        truncate_chars(&approval.decision.reason, 600)
    )
}

fn format_session_message_context(message: SessionMessage) -> String {
    format!(
        "- {} at {}:\n{}",
        message.role,
        message.created_at.to_rfc3339(),
        indent_multiline(
            &truncate_chars(&message.content, SESSION_CONTEXT_MESSAGE_CHARS),
            "  "
        )
    )
}

fn format_provider_transcript_record(record: &ProviderTranscriptRecord) -> String {
    match record.role.as_str() {
        "assistant" if !record.tool_calls.is_empty() => {
            let calls = record
                .tool_calls
                .iter()
                .map(|call| {
                    format!(
                        "{} id={} args={}",
                        call.name,
                        call.id,
                        compact_json(&call.arguments, 600)
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "- assistant tool_calls at {}: {calls}",
                record.created_at.to_rfc3339()
            )
        }
        "tool" => format!(
            "- {}tool_result {} id={} at {}:\n{}",
            if record.synthetic { "synthetic " } else { "" },
            record.name.as_deref().unwrap_or("<unknown>"),
            record.tool_call_id.as_deref().unwrap_or("<unknown>"),
            record.created_at.to_rfc3339(),
            indent_multiline(
                &truncate_chars(record.content.as_deref().unwrap_or_default(), 1_200),
                "  "
            )
        ),
        _ => format!(
            "- {} at {}:\n{}",
            record.role,
            record.created_at.to_rfc3339(),
            indent_multiline(
                &truncate_chars(record.content.as_deref().unwrap_or_default(), 800),
                "  "
            )
        ),
    }
}

fn recovered_provider_transcript(
    records: Vec<ProviderTranscriptRecord>,
) -> Vec<ProviderTranscriptRecord> {
    let mut recovered = Vec::new();
    let mut pending = BTreeMap::<String, ProviderTranscriptToolCall>::new();
    for record in records {
        if record.role == "assistant" {
            for call in &record.tool_calls {
                pending.insert(call.id.clone(), call.clone());
            }
        } else if record.role == "tool" {
            if let Some(id) = &record.tool_call_id {
                pending.remove(id);
            }
        }
        recovered.push(record);
    }
    for (id, call) in pending {
        recovered.push(ProviderTranscriptRecord {
            role: "tool".to_string(),
            content: Some(format!(
                "synthetic tool_result: tool call `{}` ({}) was interrupted before deepcli persisted a result during the previous run.",
                call.name, id
            )),
            reasoning_content: None,
            name: Some(call.name),
            tool_call_id: Some(id),
            tool_calls: Vec::new(),
            synthetic: true,
            created_at: Utc::now(),
        });
    }
    recovered
}

fn format_tool_finding_context(record: &ToolCallRecord) -> String {
    format!(
        "- {} {:?} at {} input={} output={}",
        record.tool,
        record.status,
        record.created_at.to_rfc3339(),
        compact_json(&record.input, 500),
        compact_json(&record.output, 700)
    )
}

fn format_file_history_context(snapshot: &FileHistorySnapshot) -> String {
    format!(
        "- {} {} at {}: {} data={}",
        snapshot.tool,
        snapshot.target,
        snapshot.created_at.to_rfc3339(),
        truncate_chars(&snapshot.summary, 500),
        compact_json(&snapshot.data, 700)
    )
}

fn format_test_context(test: &TestRunRecord) -> String {
    format!(
        "- `{}` passed={} exit={:?} at {} stdout={} stderr={}",
        test.command,
        test.passed,
        test.exit_code,
        test.created_at.to_rfc3339(),
        truncate_chars(&test.stdout, 500),
        truncate_chars(&test.stderr, 500)
    )
}

fn compact_json(value: &Value, limit: usize) -> String {
    serde_json::to_string(value)
        .map(|value| truncate_chars(&value, limit))
        .unwrap_or_else(|_| "<unserializable>".to_string())
}

fn file_history_snapshot_from_execution(
    call: &ToolCall,
    execution: &ToolExecution,
) -> Option<FileHistorySnapshot> {
    match call.function.name.as_str() {
        "read_file" => Some(file_history_snapshot(
            "read_file",
            file_history_target(call, &execution.raw, "path"),
            execution.structured.summary.clone(),
            read_file_history_data(&execution.structured),
        )),
        "list_files" => Some(file_history_snapshot(
            "list_files",
            list_files_history_target(call, &execution.raw),
            execution.structured.summary.clone(),
            list_files_history_data(&execution.structured),
        )),
        "search" => Some(file_history_snapshot(
            "search",
            search_history_target(call, &execution.raw),
            execution.structured.summary.clone(),
            search_history_data(&execution.structured),
        )),
        _ => None,
    }
}

fn file_history_snapshot(
    tool: &str,
    target: String,
    summary: String,
    data: Value,
) -> FileHistorySnapshot {
    FileHistorySnapshot {
        tool: tool.to_string(),
        target,
        summary: truncate_chars(&summary, 800),
        data,
        created_at: Utc::now(),
    }
}

fn file_history_target(call: &ToolCall, raw: &Value, key: &str) -> String {
    raw.get(key)
        .and_then(Value::as_str)
        .or_else(|| call.function.arguments.get(key).and_then(Value::as_str))
        .unwrap_or("<unknown>")
        .to_string()
}

fn list_files_history_target(call: &ToolCall, raw: &Value) -> String {
    let path = file_history_target(call, raw, "path");
    let glob = raw
        .get("glob")
        .and_then(Value::as_str)
        .or_else(|| call.function.arguments.get("glob").and_then(Value::as_str));
    glob.map(|glob| format!("{path} glob={glob}"))
        .unwrap_or(path)
}

fn search_history_target(call: &ToolCall, raw: &Value) -> String {
    let query = call
        .function
        .arguments
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let glob = raw
        .get("glob")
        .and_then(Value::as_str)
        .or_else(|| call.function.arguments.get("glob").and_then(Value::as_str));
    glob.map(|glob| format!("query={query} glob={glob}"))
        .unwrap_or_else(|| format!("query={query}"))
}

fn read_file_history_data(result: &StructuredToolResult) -> Value {
    let path = result
        .data
        .get("path")
        .cloned()
        .unwrap_or(Value::String("<unknown>".to_string()));
    let content_chars = result
        .data
        .get("content")
        .and_then(Value::as_str)
        .map(|content| content.chars().count())
        .unwrap_or_default();
    json!({
        "path": path,
        "content_chars": content_chars,
        "truncated": result.truncated,
    })
}

fn list_files_history_data(result: &StructuredToolResult) -> Value {
    json!({
        "path": result.data.get("path").cloned().unwrap_or(Value::Null),
        "glob": result.data.get("glob").cloned().unwrap_or(Value::Null),
        "count": result.data.get("count").cloned().unwrap_or(Value::Null),
        "truncated": result.data.get("truncated").cloned().unwrap_or(Value::Bool(result.truncated)),
        "sample": result.data
            .get("files")
            .and_then(Value::as_array)
            .map(|files| files.iter().take(12).cloned().collect::<Vec<_>>())
            .unwrap_or_default(),
    })
}

fn search_history_data(result: &StructuredToolResult) -> Value {
    json!({
        "count": result.data.get("count").cloned().unwrap_or(Value::Null),
        "searched_files": result.data.get("searched_files").cloned().unwrap_or(Value::Null),
        "glob": result.data.get("glob").cloned().unwrap_or(Value::Null),
        "truncated": result.data.get("truncated").cloned().unwrap_or(Value::Bool(result.truncated)),
        "matches": result.data
            .get("matches")
            .and_then(Value::as_array)
            .map(|matches| {
                matches
                    .iter()
                    .take(8)
                    .map(|item| {
                        json!({
                            "path": item.get("path").cloned().unwrap_or(Value::Null),
                            "line": item.get("line").cloned().unwrap_or(Value::Null),
                            "text": item
                                .get("text")
                                .and_then(Value::as_str)
                                .map(|text| truncate_chars(text, 240))
                                .unwrap_or_default(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    })
}

fn timeout_args_change_project_config(args: &[String]) -> bool {
    match args.first().map(String::as_str) {
        Some("set" | "reset") => true,
        Some("show") | None => false,
        Some(value) => !value.starts_with('-'),
    }
}

fn summarize_plan_observation(plan: &Plan) -> (usize, usize, usize, usize, Option<String>) {
    let total = plan.steps.len();
    let completed = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Completed)
        .count();
    let in_progress = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::InProgress)
        .count();
    let failed = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Failed)
        .count();
    let current_step = plan
        .steps
        .iter()
        .find(|step| step.status == PlanStepStatus::InProgress)
        .or_else(|| {
            plan.steps
                .iter()
                .find(|step| step.status == PlanStepStatus::Failed)
        })
        .or_else(|| {
            plan.steps
                .iter()
                .find(|step| step.status == PlanStepStatus::Pending)
        })
        .map(|step| step.description.clone());

    (total, completed, in_progress, failed, current_step)
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

#[derive(Debug, Clone)]
struct ContextRetryChatResult {
    response: ChatResponse,
    messages: Vec<ProviderMessage>,
    retried_after_context_error: bool,
    recoveries: Vec<AgentRecoveryEvent>,
    early_tool_results: BTreeMap<String, EarlyToolResult>,
}

#[derive(Debug, Clone)]
struct EarlyToolResult {
    call: ToolCall,
    output: String,
}

#[derive(Debug, Clone, Copy)]
struct ToolBudgetSnapshot {
    context_tool_calls_before_action: usize,
    verification_calls_before_action: usize,
    context_tool_limit: usize,
    verification_tool_limit: usize,
}

#[derive(Debug, Clone, Default)]
struct StreamingToolBudgetCounters {
    context_calls: usize,
    verification_calls: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AgentRecoveryKind {
    PromptTooLong,
    MaxOutputTokens,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AgentRecoveryEvent {
    kind: AgentRecoveryKind,
    attempt: usize,
    message_count: usize,
}

#[derive(Debug, Clone)]
struct AgentRecoveryState {
    prompt_too_long_attempts: usize,
    max_output_tokens_attempts: usize,
    prompt_too_long_limit: usize,
    max_output_tokens_limit: usize,
}

impl AgentRecoveryState {
    fn new() -> Self {
        Self {
            prompt_too_long_attempts: 0,
            max_output_tokens_attempts: 0,
            prompt_too_long_limit: env_usize("DEEPCLI_PROMPT_TOO_LONG_RECOVERY_LIMIT", 1, 1),
            max_output_tokens_limit: env_usize("DEEPCLI_MAX_OUTPUT_RECOVERY_LIMIT", 1, 3),
        }
    }

    fn register_attempt(&mut self, kind: AgentRecoveryKind) -> Option<usize> {
        match kind {
            AgentRecoveryKind::PromptTooLong => {
                if self.prompt_too_long_attempts >= self.prompt_too_long_limit {
                    return None;
                }
                self.prompt_too_long_attempts += 1;
                Some(self.prompt_too_long_attempts)
            }
            AgentRecoveryKind::MaxOutputTokens => {
                if self.max_output_tokens_attempts >= self.max_output_tokens_limit {
                    return None;
                }
                self.max_output_tokens_attempts += 1;
                Some(self.max_output_tokens_attempts)
            }
        }
    }
}

#[cfg(test)]
async fn chat_with_context_retry<F>(
    provider: &dyn ProviderClient,
    request: ChatRequest,
    provider_turn_timeout: Duration,
    on_stream_event: &mut F,
) -> Result<ContextRetryChatResult>
where
    F: FnMut(StreamEvent) + Send,
{
    let tools = request.tools.clone();
    let json_mode = request.json_mode;
    let mut messages = request.messages;
    let mut recovery_state = AgentRecoveryState::new();
    let mut recoveries: Vec<AgentRecoveryEvent> = Vec::new();

    loop {
        let current_request = ChatRequest {
            messages: messages.clone(),
            tools: tools.clone(),
            json_mode,
        };
        match provider_chat_with_timeout(
            provider,
            current_request,
            provider_turn_timeout,
            on_stream_event,
        )
        .await
        {
            Ok(response) => {
                let retried_after_context_error = recoveries
                    .iter()
                    .any(|event| event.kind == AgentRecoveryKind::PromptTooLong);
                return Ok(ContextRetryChatResult {
                    response,
                    messages,
                    retried_after_context_error,
                    recoveries,
                    early_tool_results: BTreeMap::new(),
                });
            }
            Err(error) => {
                let Some(kind) = agent_recovery_kind_for_error(&error) else {
                    return Err(error);
                };
                let Some(attempt) = recovery_state.register_attempt(kind) else {
                    return Err(error);
                };
                let recovered_messages = recover_messages_after_provider_error(kind, &messages);
                if recovered_messages == messages {
                    return Err(error);
                }
                messages = recovered_messages;
                recoveries.push(AgentRecoveryEvent {
                    kind,
                    attempt,
                    message_count: messages.len(),
                });
            }
        }
    }
}

#[cfg(test)]
async fn provider_chat_with_timeout<F>(
    provider: &dyn ProviderClient,
    request: ChatRequest,
    provider_turn_timeout: Duration,
    on_stream_event: &mut F,
) -> Result<ChatResponse>
where
    F: FnMut(StreamEvent) + Send,
{
    let callback: &mut (dyn FnMut(StreamEvent) + Send) = on_stream_event;
    match timeout(
        provider_turn_timeout,
        provider.chat_with_stream_events(request, Some(callback)),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(anyhow!(
            "provider chat timed out after {} seconds",
            provider_turn_timeout.as_secs()
        )),
    }
}

fn recover_messages_after_provider_error(
    kind: AgentRecoveryKind,
    messages: &[ProviderMessage],
) -> Vec<ProviderMessage> {
    match kind {
        AgentRecoveryKind::PromptTooLong => compact_messages_for_context_retry(messages),
        AgentRecoveryKind::MaxOutputTokens => append_output_recovery_prompt(messages),
    }
}

fn agent_recovery_kind_for_error(error: &anyhow::Error) -> Option<AgentRecoveryKind> {
    if is_context_length_error(error) {
        return Some(AgentRecoveryKind::PromptTooLong);
    }
    if is_max_output_tokens_error(error) {
        return Some(AgentRecoveryKind::MaxOutputTokens);
    }
    None
}

fn is_context_length_error(error: &anyhow::Error) -> bool {
    let text = format!("{error:?}").to_ascii_lowercase();
    text.contains("context_length_exceeded")
        || text.contains("maximum context")
        || text.contains("prompt too long")
        || text.contains("input too long")
        || text.contains("too many tokens")
        || text.contains("request too large")
        || (text.contains("context") && text.contains("length") && text.contains("exceed"))
}

fn is_max_output_tokens_error(error: &anyhow::Error) -> bool {
    let text = format!("{error:?}").to_ascii_lowercase();
    text.contains("max_output_tokens")
        || text.contains("maximum output tokens")
        || text.contains("output limit")
        || text.contains("output too long")
        || text.contains("finish_reason")
            && text.contains("length")
            && (text.contains("output") || text.contains("completion"))
}

fn is_provider_chat_timeout_error(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .starts_with("provider chat timed out after ")
}

fn env_usize(name: &str, min: usize, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= min)
        .unwrap_or(default)
}

fn context_tool_limit() -> usize {
    std::env::var("DEEPCLI_MAX_CONTEXT_TOOL_CALLS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(usize::MAX)
}

fn verification_tool_limit() -> usize {
    std::env::var("DEEPCLI_MAX_VERIFICATION_TOOL_CALLS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(usize::MAX)
}

fn budget_skip_turn_limit() -> usize {
    std::env::var("DEEPCLI_MAX_BUDGET_SKIPPED_TURNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(3)
}

fn budget_skip_recovery_prompt(turns: usize) -> String {
    format!(
        "你连续 {turns} 轮请求的工具调用被 deepcli 的工具预算护栏跳过。不要停止，也不要把这个作为最终答案。请自行诊断为什么反复请求了被跳过的工具，基于当前已有上下文调整策略，优先继续完成任务；如果确实缺少关键信息，请提出一个具体、最小的替代读取或向用户说明唯一阻塞点。"
    )
}

fn completion_hook_continuation_limit() -> usize {
    std::env::var("DEEPCLI_MAX_COMPLETION_HOOK_CONTINUATIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(3)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ToolCallBatch {
    Parallel(Range<usize>),
    Serial(usize),
}

fn tool_call_batches(registry: &ToolRegistry, calls: &[ToolCall]) -> Vec<ToolCallBatch> {
    let mut batches = Vec::new();
    let mut index = 0usize;
    while index < calls.len() {
        if is_parallel_safe_call(registry, &calls[index]) {
            let start = index;
            index += 1;
            while index < calls.len() && is_parallel_safe_call(registry, &calls[index]) {
                index += 1;
            }
            batches.push(ToolCallBatch::Parallel(start..index));
        } else {
            batches.push(ToolCallBatch::Serial(index));
            index += 1;
        }
    }
    batches
}

fn is_parallel_safe_call(registry: &ToolRegistry, call: &ToolCall) -> bool {
    registry
        .tool(&call.function.name)
        .is_some_and(|tool| tool.can_run_parallel())
}

fn batch_fits_tool_budgets(
    calls: &[ToolCall],
    context_tool_calls_before_action: usize,
    verification_calls_before_action: usize,
    context_tool_limit: usize,
    verification_tool_limit: usize,
) -> bool {
    let context_calls = calls
        .iter()
        .filter(|call| is_context_gathering_call(call))
        .count();
    let verification_calls = calls
        .iter()
        .filter(|call| is_verification_call(call))
        .count();
    context_tool_calls_before_action + context_calls <= context_tool_limit
        && verification_calls_before_action + verification_calls <= verification_tool_limit
}

fn update_tool_budget_counters(
    call: &ToolCall,
    tool_failed: bool,
    context_tool_calls_before_action: &mut usize,
    verification_calls_before_action: &mut usize,
) {
    if is_context_gathering_call(call) {
        *context_tool_calls_before_action += 1;
    } else if is_progress_action_call(call) && !tool_failed {
        *context_tool_calls_before_action = 0;
    }
    if is_project_mutating_call(call) && !tool_failed {
        *verification_calls_before_action = 0;
    } else if is_verification_call(call) {
        *verification_calls_before_action += 1;
    }
}

fn early_tool_result_matches(
    early_tool_results: &BTreeMap<String, EarlyToolResult>,
    call: &ToolCall,
) -> bool {
    early_tool_results
        .get(&call.id)
        .is_some_and(|result| result.call == *call)
}

fn take_matching_early_tool_output(
    early_tool_results: &mut BTreeMap<String, EarlyToolResult>,
    call: &ToolCall,
) -> Option<String> {
    if !early_tool_result_matches(early_tool_results, call) {
        return None;
    }
    early_tool_results
        .remove(&call.id)
        .map(|result| result.output)
}

fn tool_provider_message(call: &ToolCall, output: String) -> ProviderMessage {
    ProviderMessage {
        role: "tool".to_string(),
        content: Some(truncate_tool_output_for_prompt(&output)),
        reasoning_content: None,
        name: Some(call.function.name.clone()),
        tool_call_id: Some(call.id.clone()),
        tool_calls: None,
    }
}

fn is_context_gathering_tool(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "list_files"
            | "search"
            | "web_search"
            | "web_fetch"
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
    let limit = std::env::var("DEEPCLI_MAX_TOOL_OUTPUT_CHARS")
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
        "{head}\n\n[deepcli truncated tool output: original_chars={char_count}, kept_head={head_limit}, kept_tail={tail_limit}. Use narrower read_file ranges, search, or shell filters for more detail.]\n\n{tail}"
    )
}

fn truncate_progress_detail(output: &str) -> String {
    let limit = 2_000usize;
    let char_count = output.chars().count();
    if char_count <= limit {
        return output.to_string();
    }
    let head = output.chars().take(limit).collect::<String>();
    format!("{head}\n\n[deepcli truncated UI detail: original_chars={char_count}]")
}

fn is_approval_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("requires approval") || message.contains("requires double confirmation")
}

fn tool_call_progress_detail(call: &ToolCall) -> Option<String> {
    if !matches!(call.function.name.as_str(), "run_shell" | "run_tests") {
        return None;
    }
    call.function
        .arguments
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(truncate_progress_detail)
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

fn low_information_clarification_message(input: &str) -> &'static str {
    if matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "继续" | "go" | "next"
    ) {
        return "我还不能判断要继续哪一项。请补一句具体目标，例如“继续修复失败测试”、“继续实现上次计划”，或用 `/status`、`/session history --limit 5` 查看当前上下文。";
    }
    "我不确定你想执行什么。请说明要我分析、修改、测试、继续上次任务，或使用 `/help` 查看命令。你也可以用 `/status` 查看当前会话状态。"
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

fn truncate_chars(value: &str, limit: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= limit {
        return value.to_string();
    }
    let mut truncated = value.chars().take(limit).collect::<String>();
    truncated.push_str(&format!(
        "\n[deepcli truncated session context: original_chars={char_count}, kept_chars={limit}]"
    ));
    truncated
}

fn indent_multiline(value: &str, indent: &str) -> String {
    value
        .lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn system_prompt(
    context: &crate::workspace::WorkspaceContext,
    config: &AppConfig,
    session_context: Option<&str>,
) -> String {
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
    let agents_instructions = agents_instructions_for_prompt(context);
    let mut prompt = json!({
        "role": "deepcli agent",
        "language": config.agent.language,
        "workflow": "analyze -> plan -> modify -> test -> repair -> report",
        "rules": [
            format!("Always respond in {} unless the user explicitly asks for another language.", config.agent.language),
            "If the user input is ambiguous or too short to identify a concrete task, ask a concise clarification question before using tools.",
            "All filesystem, shell, git, network, skill, and sub-agent actions must use tools.",
            "For complex tasks, explain the plan before editing.",
            "Use prompt_list/prompt_get/prompt_render and skill_list/skill_run when reusable project prompts or Skills may fit the task.",
            "Use minimal scoped changes and run relevant tests.",
            "For existing files, prefer apply_patch_or_write with a unified diff patch; use write_file only for new files or small complete rewrites.",
            "Do not replace an existing source file with placeholder, omitted, or partial content.",
            "Never expose credentials or secrets in logs or messages."
        ],
        "workspace": context.root,
        "agents_files": agents,
        "agents_instructions": agents_instructions,
        "docs_files": docs,
        "git_diff_present": context.git_diff_present
    });
    if let Some(session_context) = session_context.filter(|value| !value.trim().is_empty()) {
        prompt["resumed_session_context"] = json!({
            "instruction": "Use this as prior conversation state for a resumed session. Do not treat it as the current user request.",
            "content": session_context
        });
    }
    prompt.to_string()
}

fn agents_instructions_for_prompt(
    context: &crate::workspace::WorkspaceContext,
) -> Vec<serde_json::Value> {
    context
        .agents_files
        .iter()
        .filter_map(|file| {
            let content = std::fs::read_to_string(&file.path).ok()?;
            Some(json!({
                "path": file.path.display().to_string(),
                "content": truncate_chars(&content, AGENTS_INSTRUCTION_CONTENT_CHARS),
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ProviderConfig};
    use crate::permissions::{DecisionOutcome, PermissionDecision, RiskLevel};
    use crate::providers::{ChatResponse, ProviderCapability, ProviderMetadata};
    use crate::session::{TestRunRecord, ToolCallRecord, ToolCallStatus};
    use std::fs;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    fn low_information_clarification_gives_actionable_context_commands() {
        let generic = low_information_clarification_message("1");
        assert!(generic.contains("/help"));
        assert!(generic.contains("/status"));

        let continuation = low_information_clarification_message("继续");
        assert!(continuation.contains("继续修复失败测试"));
        assert!(continuation.contains("/session history --limit 5"));
    }

    #[test]
    fn waiting_user_state_counts_as_open_context_for_short_replies() {
        let dir = tempdir().unwrap();
        let mut runtime = AgentRuntime::new(
            AppConfig::default(),
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

        assert!(!runtime.has_open_user_context().unwrap());
        runtime
            .session
            .set_state(SessionState::WaitingUser)
            .unwrap();
        assert!(runtime.has_open_user_context().unwrap());
    }

    #[test]
    fn resumed_session_context_includes_answered_side_questions() {
        let dir = tempdir().unwrap();
        let runtime = AgentRuntime::new(
            AppConfig::default(),
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
        let question = runtime
            .session
            .enqueue_side_question_with_options(
                "Which route should the plan use?",
                vec!["Validate first".to_string(), "Implement Task 6".to_string()],
            )
            .unwrap();
        runtime
            .session
            .answer_side_question(&question.id.to_string(), "Implement Task 6")
            .unwrap();

        let context = runtime.build_session_context().unwrap().unwrap();

        assert!(context.contains("Side questions"));
        assert!(context.contains("Which route should the plan use?"));
        assert!(context.contains("options: Validate first | Implement Task 6"));
        assert!(context.contains("answer: Implement Task 6"));
    }

    #[test]
    fn default_plan_contains_verification_step() {
        let plan = default_plan("task");
        assert!(plan.steps.iter().any(|step| step.id == "verification"));
        assert!(plan.steps.iter().any(|step| step.id == "repair"));
    }

    #[test]
    fn plan_mode_tool_specs_are_read_only_and_question_capable() {
        let registry = ToolRegistry::mvp();
        let tool_names = registry
            .tool_specs_for_names(PLAN_MODE_TOOLS)
            .into_iter()
            .map(|spec| spec.function.name)
            .collect::<Vec<_>>();

        assert!(tool_names.contains(&"read_file".to_string()));
        assert!(tool_names.contains(&"search".to_string()));
        assert!(tool_names.contains(&"ask_user_question".to_string()));
        assert!(!tool_names.contains(&"write_file".to_string()));
        assert!(!tool_names.contains(&"apply_patch_or_write".to_string()));
        assert!(!tool_names.contains(&"run_shell".to_string()));
        assert!(!tool_names.contains(&"run_tests".to_string()));
        assert!(!tool_names.contains(&"git_commit".to_string()));
    }

    #[test]
    fn planning_completion_requires_three_to_five_critical_files() {
        let missing = "## Plan\n\nImplement the feature.";
        assert_eq!(
            planning_critical_files_blocker(missing).as_deref(),
            Some("final plan must include `Critical Files for Implementation` with 3-5 file paths")
        );

        let valid = "## Plan\n\n### Critical Files for Implementation\n- src/runtime.rs\n- src/session.rs\n- src/commands/plan.rs\n";
        assert_eq!(planning_critical_files_blocker(valid), None);
        let plan = plan_from_planning_document("实现 plan 文档", valid);
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].description, "Review or update src/runtime.rs");
        assert!(plan.title.contains("实现 plan 文档"));
    }

    #[test]
    fn planning_completion_requires_model_queued_user_question() {
        let dir = tempdir().unwrap();
        let mut runtime = AgentRuntime::new(
            AppConfig::default(),
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
        runtime.planning_mode_active = true;
        let valid_plan = "## Plan\n\n### Critical Files for Implementation\n- src/runtime.rs\n- src/session.rs\n- src/commands/plan.rs\n";

        let decision = runtime.evaluate_completion_hooks(valid_plan, 1, 0).unwrap();

        assert_eq!(decision.action, CompletionAction::Continue);
        assert!(decision
            .follow_up_prompt
            .unwrap()
            .contains("ask_user_question"));
    }

    #[test]
    fn planning_question_requirement_is_not_ignored_at_continuation_limit() {
        let dir = tempdir().unwrap();
        let mut runtime = AgentRuntime::new(
            AppConfig::default(),
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
        runtime.planning_mode_active = true;
        let valid_plan = "## Plan\n\n### Critical Files for Implementation\n- src/runtime.rs\n- src/session.rs\n- src/commands/plan.rs\n";

        let decision = runtime
            .evaluate_completion_hooks(valid_plan, 1, completion_hook_continuation_limit())
            .unwrap();

        assert_eq!(decision.action, CompletionAction::Continue);
        assert!(decision
            .follow_up_prompt
            .unwrap()
            .contains("ask_user_question"));
    }

    #[test]
    fn planning_completion_accepts_after_answered_user_question() {
        let dir = tempdir().unwrap();
        let mut runtime = AgentRuntime::new(
            AppConfig::default(),
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
        runtime.planning_mode_active = true;
        let question = runtime
            .session
            .enqueue_side_question("Which module should the plan prioritize?")
            .unwrap();
        runtime
            .session
            .answer_side_question(&question.id.to_string(), "runtime")
            .unwrap();
        let valid_plan = "## Plan\n\n### Critical Files for Implementation\n- src/runtime.rs\n- src/session.rs\n- src/commands/plan.rs\n";

        let decision = runtime.evaluate_completion_hooks(valid_plan, 1, 0).unwrap();

        assert_eq!(decision.action, CompletionAction::Accept);
    }

    #[test]
    fn session_observation_summarizes_plan_tests_and_blockers() {
        let dir = tempdir().unwrap();
        let runtime = AgentRuntime::new(
            AppConfig::default(),
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
            .save_plan(&Plan {
                title: "test plan".to_string(),
                updated_at: Utc::now(),
                steps: vec![
                    PlanStep {
                        id: "one".to_string(),
                        description: "read context".to_string(),
                        status: PlanStepStatus::Completed,
                    },
                    PlanStep {
                        id: "two".to_string(),
                        description: "run tests".to_string(),
                        status: PlanStepStatus::InProgress,
                    },
                    PlanStep {
                        id: "three".to_string(),
                        description: "repair".to_string(),
                        status: PlanStepStatus::Failed,
                    },
                ],
            })
            .unwrap();
        runtime
            .session
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(101),
                stdout: String::new(),
                stderr: "failed".to_string(),
                passed: false,
                created_at: Utc::now(),
            })
            .unwrap();
        runtime
            .session
            .enqueue_side_question("which model should I use?")
            .unwrap();
        runtime
            .session
            .enqueue_approval_request(
                "write_file",
                PermissionDecision {
                    outcome: DecisionOutcome::RequiresUserApproval,
                    risk: RiskLevel::Medium,
                    reason: "write requires approval".to_string(),
                },
            )
            .unwrap();
        runtime
            .session
            .append_tool_call(&ToolCallRecord {
                tool: "run_tests".to_string(),
                input: json!({}),
                output: json!({ "stderr": "failed" }),
                decision: None,
                status: ToolCallStatus::Failed,
                created_at: Utc::now(),
            })
            .unwrap();

        let observation = runtime.session_observation().unwrap();
        assert_eq!(observation.plan_total, 3);
        assert_eq!(observation.plan_completed, 1);
        assert_eq!(observation.plan_in_progress, 1);
        assert_eq!(observation.plan_failed, 1);
        assert_eq!(observation.current_step.as_deref(), Some("run tests"));
        assert_eq!(observation.latest_test.unwrap().command, "cargo test");
        assert_eq!(observation.pending_approvals, 1);
        assert_eq!(observation.open_questions, 1);
        assert_eq!(observation.tool_calls, 1);
        assert_eq!(observation.failed_tools, 1);
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
    fn runtime_builds_resumed_session_context_from_persisted_files() {
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
            config.clone(),
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
            .append_message("user", "之前要求：修复 compiler 项目")
            .unwrap();
        runtime
            .session
            .append_message("assistant", "已经完成一次测试")
            .unwrap();
        runtime
            .session
            .write_summary("上次做到 resume 问题排查")
            .unwrap();
        runtime
            .session
            .save_plan(&default_plan("继续修复"))
            .unwrap();

        let context = runtime.build_session_context().unwrap().unwrap();
        assert!(context.contains("Last saved summary"));
        assert!(context.contains("上次做到 resume 问题排查"));
        assert!(context.contains("之前要求：修复 compiler 项目"));
        assert!(context.contains("Current saved plan"));

        let workspace_context = crate::workspace::WorkspaceContext {
            root: dir.path().to_path_buf(),
            agents_files: Vec::new(),
            docs_files: Vec::new(),
            readme_files: Vec::new(),
            git_diff_present: false,
        };
        let prompt = system_prompt(&workspace_context, &config, Some(&context));
        let value: serde_json::Value = serde_json::from_str(&prompt).unwrap();
        assert_eq!(
            value["resumed_session_context"]["content"]
                .as_str()
                .unwrap(),
            context
        );
        assert!(value["rules"].as_array().unwrap().iter().any(|rule| rule
            .as_str()
            .is_some_and(|rule| rule.contains("prompt_list/prompt_get"))));
    }

    #[test]
    fn system_prompt_includes_agents_file_contents() {
        let dir = tempdir().unwrap();
        let agents_path = dir.path().join("AGENTS.md");
        fs::write(
            &agents_path,
            "# Agent Rules\n\n- Default user-facing language is Chinese.\n",
        )
        .unwrap();
        let config = AppConfig::default();
        let workspace_context = crate::workspace::WorkspaceContext {
            root: dir.path().to_path_buf(),
            agents_files: vec![crate::workspace::summarize_file(&agents_path).unwrap()],
            docs_files: Vec::new(),
            readme_files: Vec::new(),
            git_diff_present: false,
        };

        let prompt = system_prompt(&workspace_context, &config, None);
        let value: serde_json::Value = serde_json::from_str(&prompt).unwrap();

        let instructions = value["agents_instructions"].as_array().unwrap();
        assert_eq!(instructions.len(), 1);
        assert_eq!(instructions[0]["path"], agents_path.display().to_string());
        assert!(instructions[0]["content"]
            .as_str()
            .unwrap()
            .contains("Default user-facing language is Chinese"));
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

    #[tokio::test]
    async fn runtime_renames_and_resumes_sessions_without_provider_call() {
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
        let store = SessionStore::new(dir.path());
        let other = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("other".to_string()),
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

        let rename = runtime.handle_input("/rename first task").await.unwrap();
        assert!(rename.contains("first task"));
        assert_eq!(runtime.session_title(), Some("first task"));

        let resume = runtime
            .handle_input(&format!("/resume {}", other.id()))
            .await
            .unwrap();
        assert!(resume.contains(&other.id().to_string()));
        assert_eq!(runtime.session_id(), other.id().to_string());
    }

    #[tokio::test]
    async fn runtime_auto_titles_session_before_provider_work() {
        let dir = tempdir().unwrap();
        let mut config = AppConfig {
            default_provider: "stub".to_string(),
            ..AppConfig::default()
        };
        config.providers.insert(
            "stub".to_string(),
            ProviderConfig {
                provider_type: "stub".to_string(),
                credentials_file: ".deepcli/credentials/stub.json".into(),
                acceptance_model: Some("stub-model".to_string()),
                capabilities: Vec::new(),
            },
        );

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

        let error = runtime
            .handle_input("请修复 compiler 项目并运行测试")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("provider type `stub` is not implemented"));
        assert_eq!(
            runtime.session_title(),
            Some("请修复 compiler 项目并运行测试")
        );

        let loaded = SessionStore::new(dir.path())
            .load(&runtime.session_id())
            .unwrap();
        assert_eq!(
            loaded.metadata.title.as_deref(),
            Some("请修复 compiler 项目并运行测试")
        );
    }

    #[tokio::test]
    async fn runtime_cmd_runs_shell_locally_without_provider_call() {
        let dir = tempdir().unwrap();
        let mut runtime = AgentRuntime::new(
            AppConfig::default(),
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

        let output = runtime.handle_input("/cmd pwd").await.unwrap();

        assert!(output.contains("command: pwd"));
        assert!(output.contains("exit code: 0"));
        assert!(output.contains(&dir.path().display().to_string()));
        assert!(runtime.session.load_messages().unwrap().is_empty());
        assert!(
            runtime
                .session
                .load_tool_calls()
                .unwrap()
                .iter()
                .any(|record| record.tool == "run_shell"
                    && record.status == ToolCallStatus::Succeeded)
        );
    }

    #[tokio::test]
    async fn runtime_plan_requirement_uses_provider_planning_prompt() {
        let dir = tempdir().unwrap();
        let mut config = AppConfig {
            default_provider: "stub".to_string(),
            ..AppConfig::default()
        };
        config.providers.insert(
            "stub".to_string(),
            ProviderConfig {
                provider_type: "stub".to_string(),
                credentials_file: ".deepcli/credentials/stub.json".into(),
                acceptance_model: Some("stub-model".to_string()),
                capabilities: Vec::new(),
            },
        );
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

        let error = runtime
            .handle_input("/plan 支持根据代码上下文生成澄清问题")
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("provider type `stub` is not implemented"));
        let messages = runtime.session.load_messages().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert!(messages[0]
            .content
            .contains("You are in DeepCLI plan mode."));
        assert!(messages[0]
            .content
            .contains("支持根据代码上下文生成澄清问题"));
        assert!(messages[0].content.contains("ask_user_question"));
        assert!(messages[0]
            .content
            .contains("Do not modify files or execute implementation steps."));
    }

    #[tokio::test]
    async fn runtime_cmd_attach_runs_shell_then_sends_output_to_provider_context() {
        let dir = tempdir().unwrap();
        let mut config = AppConfig {
            default_provider: "stub".to_string(),
            ..AppConfig::default()
        };
        config.providers.insert(
            "stub".to_string(),
            ProviderConfig {
                provider_type: "stub".to_string(),
                credentials_file: ".deepcli/credentials/stub.json".into(),
                acceptance_model: Some("stub-model".to_string()),
                capabilities: Vec::new(),
            },
        );
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

        let error = runtime
            .handle_input("/cmd --attach pwd")
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("provider type `stub` is not implemented"));
        assert!(error.contains("command: pwd"));
        assert!(error.contains("exit code: 0"));
        let messages = runtime.session.load_messages().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert!(messages[0]
            .content
            .contains("Local shell command output attached for model context."));
        assert!(messages[0].content.contains("command:\n```bash\npwd\n```"));
        assert!(messages[0].content.contains("exit code: 0"));
        assert!(messages[0]
            .content
            .contains(&dir.path().display().to_string()));
        assert!(
            runtime
                .session
                .load_tool_calls()
                .unwrap()
                .iter()
                .any(|record| record.tool == "run_shell"
                    && record.status == ToolCallStatus::Succeeded)
        );
    }

    #[tokio::test]
    async fn runtime_model_set_updates_active_session_and_project_config() {
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

        let output = runtime
            .handle_input("/model set kimi kimi-for-coding")
            .await
            .unwrap();
        assert!(output.contains("active session provider updated to `kimi`"));
        assert_eq!(runtime.provider_name(), "kimi");
        assert_eq!(runtime.model_name(), Some("kimi-for-coding"));

        let loaded = SessionStore::new(dir.path())
            .load(&runtime.session_id())
            .unwrap();
        assert_eq!(loaded.metadata.provider, "kimi");
        assert_eq!(loaded.metadata.model.as_deref(), Some("kimi-for-coding"));

        let raw = fs::read_to_string(dir.path().join(".deepcli/config.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["defaultProvider"], "kimi");
        assert_eq!(
            value["providers"]["kimi"]["acceptanceModel"],
            "kimi-for-coding"
        );

        let show = runtime.handle_input("/model show").await.unwrap();
        assert!(show.contains("active session provider: kimi"));
        assert!(show.contains("active session model: kimi-for-coding"));
    }

    #[tokio::test]
    async fn runtime_config_set_reloads_active_configuration() {
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

        let output = runtime
            .handle_input("/config set agent.providerTurnTimeoutSeconds 45")
            .await
            .unwrap();
        assert!(output.contains("agent.providerTurnTimeoutSeconds = 45"));
        assert_eq!(runtime.config.agent.provider_turn_timeout_seconds, 45);

        let show = runtime
            .handle_input("/config get agent.providerTurnTimeoutSeconds")
            .await
            .unwrap();
        assert_eq!(show, "45");
    }

    #[tokio::test]
    async fn runtime_credentials_update_records_redacted_audit() {
        let dir = tempdir().unwrap();
        let config = AppConfig::default();
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

        let output = runtime
            .store_provider_api_key("deepseek", "runtime-secret".to_string(), false)
            .unwrap();
        assert!(output.contains("apiKey redacted"));
        assert!(!output.contains("runtime-secret"));

        let raw = fs::read_to_string(
            dir.path()
                .join(".deepcli/credentials/deepseek-credentials.json"),
        )
        .unwrap();
        assert!(raw.contains("runtime-secret"));

        let events = runtime.session.load_audit_events().unwrap();
        let raw_events = serde_json::to_string(&events).unwrap();
        assert!(raw_events.contains("credentials_updated"));
        assert!(raw_events.contains("hidden_prompt"));
        assert!(!raw_events.contains("runtime-secret"));
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
    fn tool_call_progress_detail_uses_command_for_shell_and_tests() {
        let tool_call = |name: &str, arguments: serde_json::Value| ToolCall {
            id: format!("call_{name}"),
            call_type: "function".to_string(),
            function: crate::providers::ToolCallFunction {
                name: name.to_string(),
                arguments,
            },
        };

        assert_eq!(
            tool_call_progress_detail(&tool_call(
                "run_tests",
                json!({"command": "cargo test 2>&1"})
            ))
            .as_deref(),
            Some("cargo test 2>&1")
        );
        assert_eq!(
            tool_call_progress_detail(&tool_call("run_shell", json!({"command": "pwd && ls -la"})))
                .as_deref(),
            Some("pwd && ls -la")
        );
        assert_eq!(
            tool_call_progress_detail(&tool_call("read_file", json!({"path": "src/runtime.rs"}))),
            None
        );
    }

    #[test]
    fn agent_loop_tracker_records_structured_transitions() {
        let mut tracker = AgentLoopTracker::new();

        let transition = tracker.transition(
            Some(1),
            AgentLoopState::PreparingRequest,
            AgentLoopTransitionReason::StartIteration,
            json!({"message_count": 2}),
        );

        assert_eq!(transition.iteration, Some(1));
        assert_eq!(transition.from, AgentLoopState::Initialized);
        assert_eq!(transition.to, AgentLoopState::PreparingRequest);
        assert_eq!(transition.reason, AgentLoopTransitionReason::StartIteration);
        assert_eq!(transition.detail["message_count"], 2);

        let value = serde_json::to_value(&transition).unwrap();
        assert_eq!(value["from"], "initialized");
        assert_eq!(value["to"], "preparing_request");
        assert_eq!(value["reason"], "start_iteration");
    }

    #[test]
    fn completion_hooks_accept_clean_final_response() {
        let dir = tempdir().unwrap();
        let runtime = AgentRuntime::new(
            AppConfig::default(),
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

        let decision = runtime
            .evaluate_completion_hooks("final answer", 1, 0)
            .unwrap();

        assert_eq!(decision.action, CompletionAction::Accept);
        assert!(decision.follow_up_prompt.is_none());
    }

    #[test]
    fn completion_hooks_block_open_user_questions() {
        let dir = tempdir().unwrap();
        let runtime = AgentRuntime::new(
            AppConfig::default(),
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
            .enqueue_side_question("Which target should I verify?")
            .unwrap();

        let decision = runtime
            .evaluate_completion_hooks("final answer", 1, 0)
            .unwrap();

        assert_eq!(decision.action, CompletionAction::Continue);
        let prompt = decision.follow_up_prompt.unwrap();
        assert!(prompt.contains("completion blocked"));
        assert!(prompt.contains("open user question"));
    }

    #[test]
    fn completion_hooks_do_not_accept_open_user_questions_at_continuation_limit() {
        let dir = tempdir().unwrap();
        let runtime = AgentRuntime::new(
            AppConfig::default(),
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
            .enqueue_side_question("Which plan path should I use?")
            .unwrap();

        let decision = runtime
            .evaluate_completion_hooks("final answer", 1, completion_hook_continuation_limit())
            .unwrap();

        assert_eq!(decision.action, CompletionAction::Continue);
        assert!(decision
            .follow_up_prompt
            .unwrap()
            .contains("open user question"));
    }

    fn test_provider_message(role: &str, content: &str) -> ProviderMessage {
        ProviderMessage {
            role: role.to_string(),
            content: Some(content.to_string()),
            reasoning_content: None,
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    fn test_tool_message(id: &str, name: &str, content: &str) -> ProviderMessage {
        ProviderMessage {
            role: "tool".to_string(),
            content: Some(content.to_string()),
            reasoning_content: None,
            name: Some(name.to_string()),
            tool_call_id: Some(id.to_string()),
            tool_calls: None,
        }
    }

    struct LowTokenProvider;

    #[async_trait::async_trait]
    impl ProviderClient for LowTokenProvider {
        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: Some("ok".to_string()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }

        async fn chat_with_stream_events(
            &self,
            _request: ChatRequest,
            _on_event: Option<crate::providers::StreamEventCallback<'_>>,
        ) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: Some("ok".to_string()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }

        async fn stream(&self, _request: ChatRequest) -> Result<Vec<StreamEvent>> {
            Ok(Vec::new())
        }

        fn count_tokens(&self, _messages: &[ProviderMessage]) -> usize {
            100
        }

        fn supports(&self, _capability: ProviderCapability) -> bool {
            false
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "low-token".to_string(),
                provider_type: "test".to_string(),
                model: None,
                capabilities: Vec::new(),
            }
        }
    }

    #[tokio::test]
    async fn prepare_messages_skips_microcompact_when_context_is_not_near_budget() {
        let mut messages = vec![
            test_provider_message("system", "system"),
            test_provider_message("user", "task"),
        ];
        for index in 0..10 {
            messages.push(test_tool_message(
                &format!("call_{index}"),
                "read_file",
                &"a".repeat(5_000),
            ));
        }
        let mut config = AppConfig::default();
        config.agent.max_context_tokens = 1_000_000;
        config.agent.reserved_output_tokens = 384_000;

        let prepared = prepare_messages_for_provider(
            &LowTokenProvider,
            &messages,
            &[],
            &config,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

        assert!(!prepared.compacted());
        assert_eq!(prepared.microcompacted_tool_results, 0);
        assert_eq!(prepared.messages, messages);
    }

    #[test]
    fn recovery_context_manager_uses_boundary_transcript_and_structured_sections() {
        let dir = tempdir().unwrap();
        let runtime = AgentRuntime::new(
            AppConfig::default(),
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
            .append_message("user", "before compact boundary")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let boundary_created_at = Utc::now();
        runtime
            .session
            .append_compact_boundary(&crate::session::CompactBoundaryRecord {
                id: uuid::Uuid::new_v4(),
                reason: "full_compact".to_string(),
                summary: "summary from compact boundary".to_string(),
                omitted_group_count: 4,
                message_count_before: 14,
                message_count_after: 7,
                retained_segment: vec![crate::session::ProviderTranscriptRecord {
                    role: "assistant".to_string(),
                    content: Some("retained assistant note".to_string()),
                    reasoning_content: None,
                    name: None,
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                    synthetic: false,
                    created_at: boundary_created_at,
                }],
                created_at: boundary_created_at,
            })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        runtime
            .session
            .append_message("assistant", "after compact boundary")
            .unwrap();
        runtime.session.write_summary("older summary file").unwrap();
        runtime
            .session
            .save_plan(&Plan {
                title: "resume plan".to_string(),
                updated_at: Utc::now(),
                steps: vec![PlanStep {
                    id: "context".to_string(),
                    description: "recover state".to_string(),
                    status: PlanStepStatus::InProgress,
                }],
            })
            .unwrap();
        runtime
            .session
            .save_goal(&crate::session::GoalContract {
                objective: "finish recovery context".to_string(),
                source_requirements: vec!["user request".to_string()],
                stop_conditions: vec!["tests pass".to_string()],
                acceptance_commands: vec!["cargo test --lib".to_string()],
                status: GoalStatus::Active,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .unwrap();
        runtime
            .session
            .enqueue_side_question("which branch should resume use?")
            .unwrap();
        runtime
            .session
            .enqueue_approval_request(
                "write_file",
                PermissionDecision {
                    outcome: DecisionOutcome::RequiresUserApproval,
                    risk: RiskLevel::Medium,
                    reason: "needs write approval".to_string(),
                },
            )
            .unwrap();
        runtime
            .session
            .append_provider_transcript(&crate::session::ProviderTranscriptRecord {
                role: "assistant".to_string(),
                content: None,
                reasoning_content: None,
                name: None,
                tool_call_id: None,
                tool_calls: vec![crate::session::ProviderTranscriptToolCall {
                    id: "call_missing".to_string(),
                    call_type: "function".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({"path": "src/runtime.rs"}),
                }],
                synthetic: false,
                created_at: Utc::now(),
            })
            .unwrap();
        runtime
            .session
            .append_file_history_snapshot(&crate::session::FileHistorySnapshot {
                tool: "read_file".to_string(),
                target: "src/runtime.rs".to_string(),
                summary: "runtime contains build_session_context".to_string(),
                data: json!({"path": "src/runtime.rs"}),
                created_at: Utc::now(),
            })
            .unwrap();
        runtime
            .session
            .append_test_run(&TestRunRecord {
                command: "cargo test --lib".to_string(),
                exit_code: Some(101),
                stdout: String::new(),
                stderr: "failed".to_string(),
                passed: false,
                created_at: Utc::now(),
            })
            .unwrap();
        runtime
            .session
            .save_diff("src/runtime.rs", "-old\n+new\n")
            .unwrap();

        let context = runtime.build_session_context().unwrap().unwrap();

        assert!(context.contains("Summary"));
        assert!(context.contains("summary from compact boundary"));
        assert!(context.contains("Compact boundary"));
        assert!(context.contains("omitted_groups=4"));
        assert!(context.contains("Provider transcript"));
        assert!(context.contains("synthetic tool_result"));
        assert!(context.contains("File history snapshots"));
        assert!(context.contains("runtime contains build_session_context"));
        assert!(context.contains("Latest tests"));
        assert!(context.contains("cargo test --lib"));
        assert!(context.contains("Diff summary"));
        assert!(context.contains("Side questions"));
        assert!(context.contains("Pending approvals"));
        assert!(context.contains("after compact boundary"));
        assert!(!context.contains("before compact boundary"));
    }

    #[test]
    fn recovered_provider_transcript_closes_missing_tool_results() {
        let now = Utc::now();
        let transcript = vec![crate::session::ProviderTranscriptRecord {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
            name: None,
            tool_call_id: None,
            tool_calls: vec![crate::session::ProviderTranscriptToolCall {
                id: "call_missing".to_string(),
                call_type: "function".to_string(),
                name: "search".to_string(),
                arguments: json!({"query": "build_session_context"}),
            }],
            synthetic: false,
            created_at: now,
        }];

        let recovered = recovered_provider_transcript(transcript);

        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[1].role, "tool");
        assert_eq!(recovered[1].tool_call_id.as_deref(), Some("call_missing"));
        assert!(recovered[1].synthetic);
        assert!(recovered[1]
            .content
            .as_deref()
            .unwrap()
            .contains("interrupted before deepcli persisted a result"));
    }

    #[test]
    fn microcompact_tool_outputs_preserves_high_value_old_tool_results() {
        let failed_test_output = json!({
            "tool": "run_tests",
            "ok": false,
            "kind": "command_output",
            "summary": "tests failed",
            "content": "failed ".repeat(100),
            "data": {"passed": false},
            "truncated": false
        })
        .to_string();
        let file_diff_output = json!({
            "tool": "apply_patch_or_write",
            "ok": true,
            "kind": "file_diff",
            "summary": "patch applied",
            "content": "diff ".repeat(100),
            "data": {},
            "truncated": false
        })
        .to_string();
        let ordinary_read_output = json!({
            "tool": "read_file",
            "ok": true,
            "kind": "file_content",
            "summary": "large file",
            "content": "source ".repeat(100),
            "data": {},
            "truncated": false
        })
        .to_string();
        let messages = vec![
            test_provider_message("system", "system"),
            test_provider_message("user", "task"),
            test_tool_message("failed", "run_tests", &failed_test_output),
            test_tool_message("diff", "apply_patch_or_write", &file_diff_output),
            test_tool_message("read", "read_file", &ordinary_read_output),
        ];
        let options = ContextCompactionOptions {
            max_context_tokens: 1_000_000,
            reserved_output_tokens: 384_000,
            microcompact_keep_recent_tool_results: 0,
            microcompact_tool_output_chars: 120,
            full_compact_keep_recent_groups: 4,
        };

        let outcome = microcompact_tool_outputs(&messages, &options);

        assert_eq!(outcome.compacted_tool_results, 1);
        assert_eq!(
            outcome.messages[2].content.as_deref(),
            Some(failed_test_output.as_str())
        );
        assert_eq!(
            outcome.messages[3].content.as_deref(),
            Some(file_diff_output.as_str())
        );
        assert!(outcome.messages[4]
            .content
            .as_deref()
            .unwrap()
            .contains("deepcli compacted tool output"));
    }

    #[test]
    fn microcompact_tool_outputs_preserves_recent_results_and_compacts_older_large_outputs() {
        let messages = vec![
            test_provider_message("system", "system"),
            test_provider_message("user", "task"),
            test_tool_message("old", "read_file", &"a".repeat(500)),
            test_tool_message("middle", "search", &"b".repeat(500)),
            test_tool_message("recent", "run_tests", &"c".repeat(500)),
        ];
        let options = ContextCompactionOptions {
            max_context_tokens: 1_000_000,
            reserved_output_tokens: 384_000,
            microcompact_keep_recent_tool_results: 1,
            microcompact_tool_output_chars: 120,
            full_compact_keep_recent_groups: 4,
        };

        let outcome = microcompact_tool_outputs(&messages, &options);

        assert_eq!(outcome.compacted_tool_results, 2);
        assert!(outcome.messages[2]
            .content
            .as_deref()
            .unwrap()
            .contains("deepcli compacted tool output"));
        assert!(outcome.messages[3]
            .content
            .as_deref()
            .unwrap()
            .contains("original_chars=500"));
        let recent_output = "c".repeat(500);
        assert_eq!(
            outcome.messages[4].content.as_deref(),
            Some(recent_output.as_str())
        );
    }

    #[test]
    fn full_compacted_messages_keep_summary_and_recent_groups_without_orphan_tools() {
        let mut messages = vec![
            test_provider_message("system", "system"),
            test_provider_message("user", "task"),
        ];
        for i in 0..5 {
            messages.push(ProviderMessage {
                role: "assistant".to_string(),
                content: Some(format!("assistant {i}")),
                reasoning_content: None,
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
            messages.push(test_tool_message(
                &format!("call_{i}"),
                "read_file",
                &format!("tool result {i}"),
            ));
        }

        let compacted = build_full_compacted_messages(&messages, "summary text", 2);

        assert!(compacted.len() < messages.len());
        assert_eq!(compacted[0].role, "system");
        assert_eq!(compacted[1].role, "user");
        assert!(compacted[2]
            .content
            .as_deref()
            .unwrap()
            .contains("summary text"));
        assert_ne!(compacted[3].role, "tool");
        assert!(compacted
            .iter()
            .any(|message| message.content.as_deref() == Some("assistant 4")));
    }

    #[test]
    fn full_compact_summary_prompt_has_stable_recovery_sections() {
        let prompt = full_compact_summary_prompt();

        assert!(prompt.contains("User goal"));
        assert!(prompt.contains("Changed files"));
        assert!(prompt.contains("Tool findings"));
        assert!(prompt.contains("Pending work"));
        assert!(prompt.contains("Next step"));
    }

    struct FailingSummaryProvider;

    #[async_trait::async_trait]
    impl ProviderClient for FailingSummaryProvider {
        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            Err(anyhow!("summary provider unavailable"))
        }

        async fn chat_with_stream_events(
            &self,
            _request: ChatRequest,
            _on_event: Option<crate::providers::StreamEventCallback<'_>>,
        ) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: Some("ok".to_string()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }

        async fn stream(&self, _request: ChatRequest) -> Result<Vec<StreamEvent>> {
            Ok(Vec::new())
        }

        fn count_tokens(&self, _messages: &[ProviderMessage]) -> usize {
            1_000_000
        }

        fn supports(&self, _capability: ProviderCapability) -> bool {
            false
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "failing-summary".to_string(),
                provider_type: "test".to_string(),
                model: None,
                capabilities: Vec::new(),
            }
        }
    }

    struct SuccessfulSummaryProvider;

    #[async_trait::async_trait]
    impl ProviderClient for SuccessfulSummaryProvider {
        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: Some("provider generated compact summary".to_string()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }

        async fn chat_with_stream_events(
            &self,
            _request: ChatRequest,
            _on_event: Option<crate::providers::StreamEventCallback<'_>>,
        ) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: Some("ok".to_string()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }

        async fn stream(&self, _request: ChatRequest) -> Result<Vec<StreamEvent>> {
            Ok(Vec::new())
        }

        fn count_tokens(&self, _messages: &[ProviderMessage]) -> usize {
            1_000_000
        }

        fn supports(&self, _capability: ProviderCapability) -> bool {
            false
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "successful-summary".to_string(),
                provider_type: "test".to_string(),
                model: None,
                capabilities: Vec::new(),
            }
        }
    }

    #[tokio::test]
    async fn prepare_messages_falls_back_when_full_compact_summary_fails() {
        let mut messages = vec![
            test_provider_message("system", "system"),
            test_provider_message("user", "task"),
        ];
        for index in 0..20 {
            messages.push(test_provider_message(
                "assistant",
                &format!("assistant {index}"),
            ));
            messages.push(test_tool_message(
                &format!("call_{index}"),
                "read_file",
                &"large output ".repeat(1_000),
            ));
        }
        let mut config = AppConfig::default();
        config.agent.max_context_tokens = 10_000;
        config.agent.reserved_output_tokens = 1_000;

        let prepared = prepare_messages_for_provider(
            &FailingSummaryProvider,
            &messages,
            &[],
            &config,
            Duration::from_secs(1),
        )
        .await
        .expect("summary failure should degrade to tail compaction");

        assert!(prepared.tail_compacted);
        assert!(prepared.messages.len() < messages.len());
    }

    #[tokio::test]
    async fn prepare_messages_returns_compact_boundary_snapshot() {
        let mut messages = vec![
            test_provider_message("system", "system"),
            test_provider_message("user", "task"),
        ];
        for index in 0..8 {
            messages.push(ProviderMessage {
                role: "assistant".to_string(),
                content: Some(format!("assistant {index}")),
                reasoning_content: None,
                name: None,
                tool_call_id: None,
                tool_calls: Some(vec![ToolCall {
                    id: format!("call_{index}"),
                    call_type: "function".to_string(),
                    function: crate::providers::ToolCallFunction {
                        name: "read_file".to_string(),
                        arguments: json!({"path": "src/lib.rs"}),
                    },
                }]),
            });
            messages.push(test_tool_message(
                &format!("call_{index}"),
                "read_file",
                &"large output ".repeat(1_000),
            ));
        }
        let mut config = AppConfig::default();
        config.agent.max_context_tokens = 10_000;
        config.agent.reserved_output_tokens = 1_000;

        let prepared = prepare_messages_for_provider(
            &SuccessfulSummaryProvider,
            &messages,
            &[],
            &config,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

        let boundary = prepared
            .compact_boundary
            .expect("full compaction should create a compact boundary snapshot");
        assert!(boundary.reason.contains("full_compact"));
        assert_eq!(boundary.summary, "provider generated compact summary");
        assert!(boundary.omitted_group_count > 0);
        assert_eq!(boundary.message_count_before, messages.len());
        assert_eq!(boundary.message_count_after, prepared.messages.len());
        assert!(!boundary.retained_segment.is_empty());
    }

    #[derive(Clone, Default)]
    struct ContextRetryProvider {
        calls: Arc<AtomicUsize>,
        message_counts: Arc<Mutex<Vec<usize>>>,
    }

    #[async_trait::async_trait]
    impl ProviderClient for ContextRetryProvider {
        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
            self.chat_with_stream_events(request, None).await
        }

        async fn chat_with_stream_events(
            &self,
            request: ChatRequest,
            _on_event: Option<crate::providers::StreamEventCallback<'_>>,
        ) -> Result<ChatResponse> {
            self.message_counts
                .lock()
                .unwrap()
                .push(request.messages.len());
            let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
            if call_index == 0 {
                return Err(anyhow!(
                    "context_length_exceeded: maximum context length exceeded"
                ));
            }
            Ok(ChatResponse {
                content: Some("after retry".to_string()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }

        async fn stream(&self, _request: ChatRequest) -> Result<Vec<StreamEvent>> {
            Ok(Vec::new())
        }

        fn count_tokens(&self, messages: &[ProviderMessage]) -> usize {
            messages
                .iter()
                .filter_map(|message| message.content.as_ref())
                .map(|content| content.len() / 4)
                .sum()
        }

        fn supports(&self, _capability: ProviderCapability) -> bool {
            false
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "context-retry".to_string(),
                provider_type: "test".to_string(),
                model: None,
                capabilities: Vec::new(),
            }
        }
    }

    #[tokio::test]
    async fn chat_with_context_retry_compacts_and_retries_once() {
        let mut messages = vec![
            test_provider_message("system", "system"),
            test_provider_message("user", "task"),
        ];
        for index in 0..10 {
            messages.push(test_provider_message(
                "assistant",
                &format!("assistant {index}"),
            ));
            messages.push(test_tool_message(
                &format!("call_{index}"),
                "read_file",
                &format!("tool result {index}"),
            ));
        }
        let provider = ContextRetryProvider::default();
        let request = ChatRequest {
            messages: messages.clone(),
            tools: Vec::new(),
            json_mode: false,
        };
        let mut noop = |_event: StreamEvent| {};

        let result = chat_with_context_retry(&provider, request, Duration::from_secs(1), &mut noop)
            .await
            .unwrap();

        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
        assert!(result.retried_after_context_error);
        assert!(result.messages.len() < messages.len());
        assert_eq!(result.response.content.as_deref(), Some("after retry"));
        let counts = provider.message_counts.lock().unwrap().clone();
        assert_eq!(counts.len(), 2);
        assert!(counts[1] < counts[0]);
        assert_eq!(result.recoveries.len(), 1);
        assert_eq!(result.recoveries[0].kind, AgentRecoveryKind::PromptTooLong);
    }

    #[derive(Clone, Default)]
    struct OutputLimitRecoveryProvider {
        calls: Arc<AtomicUsize>,
        last_messages: Arc<Mutex<Vec<ProviderMessage>>>,
    }

    #[async_trait::async_trait]
    impl ProviderClient for OutputLimitRecoveryProvider {
        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
            self.chat_with_stream_events(request, None).await
        }

        async fn chat_with_stream_events(
            &self,
            request: ChatRequest,
            _on_event: Option<crate::providers::StreamEventCallback<'_>>,
        ) -> Result<ChatResponse> {
            *self.last_messages.lock().unwrap() = request.messages.clone();
            let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
            if call_index == 0 {
                return Err(anyhow!(
                    "max_output_tokens exceeded before the provider could finish"
                ));
            }
            Ok(ChatResponse {
                content: Some("after output recovery".to_string()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }

        async fn stream(&self, _request: ChatRequest) -> Result<Vec<StreamEvent>> {
            Ok(Vec::new())
        }

        fn count_tokens(&self, _messages: &[ProviderMessage]) -> usize {
            1
        }

        fn supports(&self, _capability: ProviderCapability) -> bool {
            false
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "output-limit-recovery".to_string(),
                provider_type: "test".to_string(),
                model: None,
                capabilities: Vec::new(),
            }
        }
    }

    #[tokio::test]
    async fn chat_recovery_retries_output_limit_with_bounded_nudge() {
        let provider = OutputLimitRecoveryProvider::default();
        let request = ChatRequest {
            messages: vec![
                test_provider_message("system", "system"),
                test_provider_message("user", "task"),
            ],
            tools: Vec::new(),
            json_mode: false,
        };
        let mut noop = |_event: StreamEvent| {};

        let result = chat_with_context_retry(&provider, request, Duration::from_secs(1), &mut noop)
            .await
            .unwrap();

        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
        assert_eq!(result.recoveries.len(), 1);
        assert_eq!(
            result.recoveries[0].kind,
            AgentRecoveryKind::MaxOutputTokens
        );
        assert_eq!(
            result.response.content.as_deref(),
            Some("after output recovery")
        );
        let last_messages = provider.last_messages.lock().unwrap().clone();
        assert!(last_messages
            .last()
            .and_then(|message| message.content.as_deref())
            .is_some_and(|content| content.contains("deepcli output recovery")));
    }

    #[derive(Clone)]
    struct StreamingToolCompletionProvider {
        call: ToolCall,
    }

    #[async_trait::async_trait]
    impl ProviderClient for StreamingToolCompletionProvider {
        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
            self.chat_with_stream_events(request, None).await
        }

        async fn chat_with_stream_events(
            &self,
            _request: ChatRequest,
            mut on_event: Option<crate::providers::StreamEventCallback<'_>>,
        ) -> Result<ChatResponse> {
            if let Some(callback) = on_event.as_mut() {
                callback(StreamEvent {
                    content_delta: None,
                    reasoning_delta: None,
                    tool_call_delta: None,
                    tool_call_completed: Some(self.call.clone()),
                    done: false,
                });
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(ChatResponse {
                content: None,
                reasoning_content: None,
                tool_calls: vec![self.call.clone()],
                usage: Usage::default(),
            })
        }

        async fn stream(&self, _request: ChatRequest) -> Result<Vec<StreamEvent>> {
            Ok(Vec::new())
        }

        fn count_tokens(&self, _messages: &[ProviderMessage]) -> usize {
            1
        }

        fn supports(&self, _capability: ProviderCapability) -> bool {
            true
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "streaming-tool-completion".to_string(),
                provider_type: "test".to_string(),
                model: None,
                capabilities: Vec::new(),
            }
        }
    }

    #[tokio::test]
    async fn streaming_tool_completion_executes_read_only_tool_before_turn_finishes() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        let mut runtime = AgentRuntime::new(
            AppConfig::default(),
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
            id: "call_read".to_string(),
            call_type: "function".to_string(),
            function: crate::providers::ToolCallFunction {
                name: "read_file".to_string(),
                arguments: json!({"path": "Cargo.toml"}),
            },
        };
        let provider = StreamingToolCompletionProvider { call: call.clone() };
        let request = ChatRequest {
            messages: vec![test_provider_message("user", "read cargo")],
            tools: runtime.registry.tool_specs(),
            json_mode: false,
        };
        let mut noop = |_event: StreamEvent| {};

        let result = runtime
            .chat_with_context_retry_and_streaming_tools(
                &provider,
                request,
                Duration::from_secs(1),
                &mut noop,
                ToolBudgetSnapshot {
                    context_tool_calls_before_action: 0,
                    verification_calls_before_action: 0,
                    context_tool_limit: 12,
                    verification_tool_limit: 12,
                },
            )
            .await
            .unwrap();

        let early = result
            .early_tool_results
            .get("call_read")
            .expect("read_file should execute during provider stream");
        assert_eq!(early.call, call);
        assert!(early.output.contains("[package]"));
        let succeeded = runtime
            .session
            .load_tool_calls()
            .unwrap()
            .into_iter()
            .filter(|record| record.status == ToolCallStatus::Succeeded)
            .count();
        assert_eq!(succeeded, 1);
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
        assert!(is_context_gathering_tool("web_fetch"));
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
    fn tool_call_batches_group_consecutive_parallel_safe_tools() {
        let call = |name: &str| ToolCall {
            id: format!("call_{name}"),
            call_type: "function".to_string(),
            function: crate::providers::ToolCallFunction {
                name: name.to_string(),
                arguments: json!({}),
            },
        };
        let registry = ToolRegistry::mvp();
        let calls = vec![
            call("read_file"),
            call("list_files"),
            call("todo_write"),
            call("web_fetch"),
            call("search"),
            call("write_file"),
        ];

        let batches = tool_call_batches(&registry, &calls);

        assert_eq!(
            batches,
            vec![
                ToolCallBatch::Parallel(0..2),
                ToolCallBatch::Serial(2),
                ToolCallBatch::Parallel(3..5),
                ToolCallBatch::Serial(5),
            ]
        );
    }

    #[test]
    fn budget_skipped_exhaustion_creates_recovery_prompt_for_model() {
        let prompt = budget_skip_recovery_prompt(3);

        assert!(prompt.contains("连续 3 轮"));
        assert!(prompt.contains("不要停止"));
        assert!(prompt.contains("自行诊断"));
        assert!(prompt.contains("继续完成任务"));
    }

    #[test]
    fn tool_budget_guards_are_disabled_by_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("DEEPCLI_MAX_CONTEXT_TOOL_CALLS");
        std::env::remove_var("DEEPCLI_MAX_VERIFICATION_TOOL_CALLS");

        assert_eq!(context_tool_limit(), usize::MAX);
        assert_eq!(verification_tool_limit(), usize::MAX);
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
