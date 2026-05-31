use crate::commands::{
    format_session_list, handle_config, handle_credentials_with_default, handle_model_read_command,
    handle_timeout, list_resumable_sessions, parse_model_set_args, set_credentials_api_key,
    update_project_model_config, CommandContext, CommandRouter, SlashCommand,
};
use crate::config::AppConfig;
use crate::permissions::PermissionEngine;
use crate::providers::{create_provider, ChatRequest, ProviderMessage, ToolCall, Usage};
use crate::session::{
    ApprovalStatus, AuditEvent, Plan, PlanStep, PlanStepStatus, Session, SessionMessage,
    SessionState, SessionStore, SideQuestionStatus, ToolCallRecord, ToolCallStatus,
};
use crate::tools::{ToolExecutor, ToolRegistry};
use crate::workspace::WorkspaceManager;
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use tokio::time::timeout;

const SESSION_CONTEXT_MESSAGE_LIMIT: usize = 16;
const SESSION_CONTEXT_MESSAGE_CHARS: usize = 1_500;
const SESSION_CONTEXT_TOTAL_CHARS: usize = 16_000;

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
                "deepcli: provider stream started".to_string()
            }
            RuntimeProgress::ProviderTurnStarted {
                iteration,
                max_iterations,
                message_count,
                tool_count,
                request_kib,
                compacted,
            } => format!(
                "deepcli: provider turn {iteration}/{max_iterations} (messages={message_count}, tools={tool_count}, request~{request_kib} KiB{})",
                if *compacted { ", compacted" } else { "" }
            ),
            RuntimeProgress::ProviderTurnCompleted {
                elapsed_ms,
                tool_calls,
            } => format!(
                "deepcli: provider response in {:.1}s (tool_calls={tool_calls})",
                *elapsed_ms as f64 / 1000.0
            ),
            RuntimeProgress::ToolStarted { tool } => {
                format!("deepcli: running tool {tool}")
            }
            RuntimeProgress::ToolCompleted { tool, ok, .. } => {
                let status = if *ok { "completed" } else { "failed" };
                format!("deepcli: tool {tool} {status}")
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
                SlashCommand::Resume { id } => {
                    return self.handle_resume_command(id);
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
                },
            )
            .await;
        }
        if is_low_information_input(input) && !self.has_open_user_context()? {
            return self.handle_low_information_input(input);
        }
        self.run_agent_task(input).await
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
        )?;
        match action.as_deref() {
            Some("set" | "import-env") => {
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
        self.session.auto_title_from_user_task(task)?;
        self.executor.set_session(Some(self.session.clone()));
        let session_context = self.build_session_context()?;
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

    fn build_session_context(&self) -> Result<Option<String>> {
        let mut sections = Vec::new();

        if let Some(summary) = self.session.load_summary()? {
            let summary = summary.trim();
            if !summary.is_empty() {
                sections.push(format!(
                    "Last saved summary:\n{}",
                    truncate_chars(summary, SESSION_CONTEXT_MESSAGE_CHARS)
                ));
            }
        }

        if let Some(plan) = self.session.load_plan()? {
            if !plan.steps.is_empty() {
                let steps = plan
                    .steps
                    .iter()
                    .map(|step| format!("- {:?}: {} ({})", step.status, step.description, step.id))
                    .collect::<Vec<_>>()
                    .join("\n");
                sections.push(format!("Current saved plan: {}\n{steps}", plan.title));
            }
        }

        let messages = self
            .session
            .load_recent_messages(SESSION_CONTEXT_MESSAGE_LIMIT)?;
        let recent = messages
            .into_iter()
            .filter(|message| !message.content.trim().is_empty())
            .map(|message| {
                format!(
                    "- {} at {}:\n{}",
                    message.role,
                    message.created_at.to_rfc3339(),
                    indent_multiline(
                        &truncate_chars(&message.content, SESSION_CONTEXT_MESSAGE_CHARS),
                        "  "
                    )
                )
            })
            .collect::<Vec<_>>();
        if !recent.is_empty() {
            sections.push(format!(
                "Recent conversation messages:\n{}",
                recent.join("\n")
            ));
        }

        if sections.is_empty() {
            return Ok(None);
        }

        let context = sections.join("\n\n");
        Ok(Some(truncate_chars(&context, SESSION_CONTEXT_TOTAL_CHARS)))
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

fn compact_messages_for_provider(messages: &[ProviderMessage]) -> Vec<ProviderMessage> {
    let limit = std::env::var("DEEPCLI_MAX_PROVIDER_REQUEST_BYTES")
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
    std::env::var("DEEPCLI_MAX_CONTEXT_TOOL_CALLS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(12)
}

fn verification_tool_limit() -> usize {
    std::env::var("DEEPCLI_MAX_VERIFICATION_TOOL_CALLS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(12)
}

fn budget_skip_turn_limit() -> usize {
    std::env::var("DEEPCLI_MAX_BUDGET_SKIPPED_TURNS")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ProviderConfig};
    use crate::permissions::{DecisionOutcome, PermissionDecision, RiskLevel};
    use crate::session::{TestRunRecord, ToolCallRecord};
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
    fn default_plan_contains_verification_step() {
        let plan = default_plan("task");
        assert!(plan.steps.iter().any(|step| step.id == "verification"));
        assert!(plan.steps.iter().any(|step| step.id == "repair"));
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
