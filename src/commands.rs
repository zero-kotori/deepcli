use crate::agents::AgentStore;
use crate::config::AppConfig;
use crate::privacy::looks_sensitive;
use crate::prompts::PromptStore;
use crate::session::{
    ApprovalRequest, ApprovalStatus, PlanStepStatus, SessionStore, SideQuestion, SideQuestionStatus,
};
use crate::skills::SkillStore;
use crate::tools::{ToolExecutor, ToolRegistry};
use crate::workspace::WorkspaceManager;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlashCommand {
    Help,
    Status,
    Context,
    Permissions { args: Vec<String> },
    Config,
    Model { args: Vec<String> },
    Plan,
    Diff { staged: bool },
    Review,
    Test { args: Vec<String> },
    Env { args: Vec<String> },
    Git { args: Vec<String> },
    Prompt { args: Vec<String> },
    Skill { args: Vec<String> },
    Agent { args: Vec<String> },
    Btw { args: Vec<String> },
    Approval { args: Vec<String> },
    Session { args: Vec<String> },
    Resume { id: Option<String> },
    Terminal,
}

pub struct CommandRouter;

pub struct CommandContext<'a> {
    pub workspace: &'a Path,
    pub config: &'a AppConfig,
    pub registry: &'a ToolRegistry,
    pub executor: &'a ToolExecutor,
    pub session_id: Option<String>,
}

impl CommandRouter {
    pub fn parse(input: &str) -> Result<Option<SlashCommand>> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return Ok(None);
        }
        let parts = shell_words::split(trimmed).unwrap_or_else(|_| {
            trimmed
                .split_whitespace()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        });
        let command = parts.first().cloned().unwrap_or_default();
        let args = parts.into_iter().skip(1).collect::<Vec<_>>();
        Ok(Some(match command.as_str() {
            "/help" => SlashCommand::Help,
            "/status" => SlashCommand::Status,
            "/context" => SlashCommand::Context,
            "/permissions" => SlashCommand::Permissions { args },
            "/config" => SlashCommand::Config,
            "/model" => SlashCommand::Model { args },
            "/plan" => SlashCommand::Plan,
            "/diff" => SlashCommand::Diff {
                staged: args.iter().any(|arg| arg == "--staged"),
            },
            "/review" => SlashCommand::Review,
            "/test" => SlashCommand::Test { args },
            "/env" => SlashCommand::Env { args },
            "/git" => SlashCommand::Git { args },
            "/prompt" => SlashCommand::Prompt { args },
            "/skill" => SlashCommand::Skill { args },
            "/agent" => SlashCommand::Agent { args },
            "/btw" => SlashCommand::Btw { args },
            "/approval" => SlashCommand::Approval { args },
            "/session" => SlashCommand::Session { args },
            "/resume" => SlashCommand::Resume {
                id: args.first().cloned(),
            },
            "/terminal" => SlashCommand::Terminal,
            other => bail!("unknown slash command `{other}`"),
        }))
    }

    pub async fn handle(command: SlashCommand, context: CommandContext<'_>) -> Result<String> {
        match command {
            SlashCommand::Help => Ok(Self::help_text()),
            SlashCommand::Status => handle_status(context),
            SlashCommand::Context => handle_context(context.workspace),
            SlashCommand::Permissions { args } => {
                handle_permissions(context.workspace, context.config, args)
            }
            SlashCommand::Config => Ok(serde_json::to_string_pretty(&context.config)?),
            SlashCommand::Model { args } => handle_model(context.workspace, context.config, args),
            SlashCommand::Plan => {
                if let Some(session_id) = context.session_id {
                    let store = SessionStore::new(context.workspace);
                    let session = store.load(&session_id)?;
                    if let Some(plan) = session.load_plan()? {
                        return Ok(serde_json::to_string_pretty(&plan)?);
                    }
                }
                Ok("no active plan".to_string())
            }
            SlashCommand::Diff { staged } => {
                let output = context
                    .executor
                    .execute("git_diff", json!({ "staged": staged }))
                    .await?;
                Ok(output.content)
            }
            SlashCommand::Review => handle_review(context.executor).await,
            SlashCommand::Test { args } => handle_test(context.executor, args).await,
            SlashCommand::Env { args } => handle_env(context.executor, args).await,
            SlashCommand::Git { args } => handle_git(context.executor, args).await,
            SlashCommand::Prompt { args } => handle_prompt(context.workspace, args),
            SlashCommand::Skill { args } => handle_skill(context.workspace, args),
            SlashCommand::Agent { args } => {
                handle_agent(context.workspace, context.executor, args).await
            }
            SlashCommand::Btw { args } => handle_btw(context.workspace, context.session_id, args),
            SlashCommand::Approval { args } => {
                handle_approval(context.workspace, context.session_id, args)
            }
            SlashCommand::Session { args } => {
                handle_session(context.workspace, context.session_id, args)
            }
            SlashCommand::Resume { id } => {
                let store = SessionStore::new(context.workspace);
                if let Some(id) = id {
                    let session = store.load(&id)?;
                    Ok(serde_json::to_string_pretty(&session.metadata)?)
                } else {
                    Ok(serde_json::to_string_pretty(&store.list()?)?)
                }
            }
            SlashCommand::Terminal => {
                let output = context.executor.execute("open_terminal", json!({})).await?;
                Ok(output.content)
            }
        }
    }

    pub fn help_text() -> String {
        [
            "/help",
            "/status",
            "/context",
            "/permissions",
            "/config",
            "/model [show|set <provider> [model]]",
            "/plan",
            "/diff [--staged]",
            "/review",
            "/test [run [command]]",
            "/env check [docker|compiler]|setup [docker|compiler] [--smoke]|test [docker|compiler]",
            "/git status|diff|branch|message|create-branch <name>|commit <message>",
            "/prompt list|get <name>|save <name> <body>",
            "/skill list|generate <name> <description>|run <name>",
            "/agent list|show <id>|spawn <task>",
            "/btw ask <question>|list [--all]|answer <id> <answer>|clear",
            "/approval list [--all]|approve <id>|deny <id>|clear",
            "/session list|show [session_id]",
            "/resume [session_id]",
            "/terminal",
        ]
        .join("\n")
    }

    pub fn command_names() -> Vec<&'static str> {
        vec![
            "/help",
            "/status",
            "/context",
            "/permissions",
            "/config",
            "/model",
            "/plan",
            "/diff",
            "/review",
            "/test",
            "/env",
            "/git",
            "/prompt",
            "/skill",
            "/agent",
            "/btw",
            "/approval",
            "/session",
            "/resume",
            "/terminal",
        ]
    }
}

fn handle_context(workspace: &Path) -> Result<String> {
    let manager = WorkspaceManager::new(workspace)?;
    let context = manager.collect_context()?;
    let format_files = |files: &[crate::workspace::FileSummary]| {
        if files.is_empty() {
            "<none>".to_string()
        } else {
            files
                .iter()
                .map(|file| {
                    file.path
                        .strip_prefix(workspace)
                        .unwrap_or(&file.path)
                        .display()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    Ok(format!(
        "workspace: {}\nagents: {}\nreadme: {}\ndocs: {}\ngit diff present: {}",
        context.root.display(),
        format_files(&context.agents_files),
        format_files(&context.readme_files),
        format_files(&context.docs_files),
        context.git_diff_present
    ))
}

fn handle_status(context: CommandContext<'_>) -> Result<String> {
    let session_label = context
        .session_id
        .clone()
        .unwrap_or_else(|| "<none>".to_string());
    let mut lines = vec![
        format!("workspace: {}", context.workspace.display()),
        format!("session: {session_label}"),
        format!(
            "registered tools: {}",
            context.registry.declarations().len()
        ),
        format!(
            "token warning threshold: {}",
            context.config.usage.token_warning_threshold
        ),
        format!(
            "provider turn timeout: {}s",
            context.config.agent.provider_turn_timeout_seconds
        ),
    ];

    if let Some(session_id) = context.session_id {
        let store = SessionStore::new(context.workspace);
        if let Ok(session) = store.load(&session_id) {
            let summary = session.activity_summary()?;
            lines.push(format!("state: {:?}", session.metadata.state));
            lines.push(format!("provider: {}", session.metadata.provider));
            lines.push(format!(
                "model: {}",
                session
                    .metadata
                    .model
                    .clone()
                    .unwrap_or_else(|| "<unset>".to_string())
            ));
            lines.push(format!(
                "activity: messages={} tools={} tests={} diffs={} backups={} side_questions={} approvals={} summary={}",
                summary.message_count,
                summary.tool_call_count,
                summary.test_run_count,
                summary.diff_count,
                summary.backup_count,
                summary.side_question_count,
                summary.approval_request_count,
                summary.has_summary
            ));
            if let Some(plan) = session.load_plan()? {
                let completed = plan
                    .steps
                    .iter()
                    .filter(|step| step.status == PlanStepStatus::Completed)
                    .count();
                lines.push(format!("plan: {completed}/{} completed", plan.steps.len()));
            }
        }
    }

    Ok(lines.join("\n"))
}

fn handle_permissions(workspace: &Path, config: &AppConfig, args: Vec<String>) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("show") => Ok(serde_json::to_string_pretty(&config.permissions)?),
        Some("set-mode") => {
            let mode = required_arg(&args, 1, "permission mode")?;
            update_project_permission_mode(workspace, mode)?;
            Ok(format!(
                "permissions.defaultMode updated to `{mode}` in .deepcli/config.json"
            ))
        }
        Some(other) => bail!("unsupported /permissions action `{other}`"),
    }
}

fn handle_model(workspace: &Path, config: &AppConfig, args: Vec<String>) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("show") => {
            let runtime = config.redacted_provider_runtime(workspace, None)?;
            Ok(format!(
                "provider: {}\ntype: {}\nmodel: {}\ncapabilities: {}",
                runtime.name,
                runtime.provider_type,
                runtime.model.unwrap_or_else(|| "<unset>".to_string()),
                runtime.capabilities.join(", ")
            ))
        }
        Some("set") => {
            let provider = required_arg(&args, 1, "provider name")?;
            if !config.providers.contains_key(provider) {
                bail!("provider `{provider}` is not configured");
            }
            let model = args.get(2).map(String::as_str);
            update_project_model_config(workspace, provider, model)?;
            if let Some(model) = model {
                Ok(format!(
                    "defaultProvider updated to `{provider}`, acceptanceModel updated to `{model}`"
                ))
            } else {
                Ok(format!("defaultProvider updated to `{provider}`"))
            }
        }
        Some(other) => bail!("unsupported /model action `{other}`"),
    }
}

fn update_project_model_config(
    workspace: &Path,
    provider: &str,
    model: Option<&str>,
) -> Result<()> {
    let path = workspace.join(".deepcli").join("config.json");
    let raw = fs::read_to_string(&path)?;
    let mut value: Value = serde_json::from_str(&raw)?;
    value["defaultProvider"] = Value::String(provider.to_string());
    if let Some(model) = model {
        let providers = value
            .get_mut("providers")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow::anyhow!("project config providers must be an object"))?;
        let provider_value = providers.get_mut(provider).ok_or_else(|| {
            anyhow::anyhow!("provider `{provider}` is missing from project config")
        })?;
        provider_value["acceptanceModel"] = Value::String(model.to_string());
    }
    fs::write(&path, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}

fn handle_session(workspace: &Path, current: Option<String>, args: Vec<String>) -> Result<String> {
    let store = SessionStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => Ok(serde_json::to_string_pretty(&store.list()?)?),
        Some("show") => {
            let id = args.get(1).cloned().or(current).ok_or_else(|| {
                anyhow::anyhow!("missing session id and no active session is available")
            })?;
            let session = store.load(&id)?;
            let summary = session.activity_summary()?;
            Ok(format!(
                "{}\n{}",
                serde_json::to_string_pretty(&session.metadata)?,
                serde_json::to_string_pretty(&summary)?
            ))
        }
        Some(other) => bail!("unsupported /session action `{other}`"),
    }
}

fn handle_approval(workspace: &Path, current: Option<String>, args: Vec<String>) -> Result<String> {
    let id = current.ok_or_else(|| anyhow::anyhow!("no active session is available"))?;
    let store = SessionStore::new(workspace);
    let session = store.load(&id)?;
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let include_all = args.iter().any(|arg| arg == "--all");
            Ok(format_approval_requests(
                &session.load_approval_requests()?,
                include_all,
            ))
        }
        Some("approve") => {
            let approval_id = required_arg(&args, 1, "approval request id")?;
            let item = session.update_approval_request(approval_id, ApprovalStatus::Approved)?;
            Ok(format!("approved request {}", short_id(&item.id)))
        }
        Some("deny") => {
            let approval_id = required_arg(&args, 1, "approval request id")?;
            let item = session.update_approval_request(approval_id, ApprovalStatus::Denied)?;
            Ok(format!("denied request {}", short_id(&item.id)))
        }
        Some("clear") => {
            let cleared = session.clear_pending_approval_requests()?;
            Ok(format!("cleared {cleared} pending approval request(s)"))
        }
        Some(other) => bail!("unsupported /approval action `{other}`"),
    }
}

fn format_approval_requests(items: &[ApprovalRequest], include_all: bool) -> String {
    let rows = items
        .iter()
        .filter(|item| include_all || item.status == ApprovalStatus::Pending)
        .map(|item| {
            format!(
                "{} [{}] tool={} risk={:?} outcome={:?} reason={}",
                short_id(&item.id),
                approval_status_label(&item.status),
                item.tool,
                item.decision.risk,
                item.decision.outcome,
                item.decision.reason
            )
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        "no approval requests".to_string()
    } else {
        rows.join("\n")
    }
}

fn approval_status_label(status: &ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Cleared => "cleared",
    }
}

fn handle_btw(workspace: &Path, current: Option<String>, args: Vec<String>) -> Result<String> {
    let id = current.ok_or_else(|| anyhow::anyhow!("no active session is available"))?;
    let store = SessionStore::new(workspace);
    let session = store.load(&id)?;
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let include_all = args.iter().any(|arg| arg == "--all");
            Ok(format_side_questions(
                &session.load_side_questions()?,
                include_all,
            ))
        }
        Some("ask") => {
            let question = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if question.trim().is_empty() {
                bail!("/btw ask requires a question");
            }
            let item = session.enqueue_side_question(question.trim())?;
            Ok(format!(
                "queued by-the-way question {}: {}",
                short_id(&item.id),
                item.question
            ))
        }
        Some("answer") => {
            let question_id = required_arg(&args, 1, "side question id")?;
            let answer = args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
            if answer.trim().is_empty() {
                bail!("/btw answer requires an answer");
            }
            let item = session.answer_side_question(question_id, answer.trim())?;
            Ok(format!(
                "answered by-the-way question {}",
                short_id(&item.id)
            ))
        }
        Some("clear") => {
            let cleared = session.clear_side_questions()?;
            Ok(format!("cleared {cleared} open by-the-way question(s)"))
        }
        Some(other) => bail!("unsupported /btw action `{other}`"),
    }
}

fn format_side_questions(items: &[SideQuestion], include_all: bool) -> String {
    let rows = items
        .iter()
        .filter(|item| include_all || item.status == SideQuestionStatus::Open)
        .map(|item| {
            let mut line = format!(
                "{} [{}] {}",
                short_id(&item.id),
                side_question_status_label(&item.status),
                item.question
            );
            if let Some(answer) = &item.answer {
                line.push_str(&format!("\n  answer: {answer}"));
            }
            line
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        "no by-the-way questions".to_string()
    } else {
        rows.join("\n")
    }
}

fn side_question_status_label(status: &SideQuestionStatus) -> &'static str {
    match status {
        SideQuestionStatus::Open => "open",
        SideQuestionStatus::Answered => "answered",
        SideQuestionStatus::Cleared => "cleared",
    }
}

fn short_id(id: &uuid::Uuid) -> String {
    id.to_string()[..8].to_string()
}

async fn handle_agent(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let store = AgentStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => Ok(serde_json::to_string_pretty(&store.list()?)?),
        Some("show") => {
            let id = required_arg(&args, 1, "sub-agent id")?;
            let id = uuid::Uuid::parse_str(id)?;
            Ok(serde_json::to_string_pretty(&store.load(id)?)?)
        }
        Some("spawn") => {
            let task = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if task.trim().is_empty() {
                bail!("/agent spawn requires a task");
            }
            Ok(executor
                .execute("spawn_subagent", json!({"task": task, "depth": 1}))
                .await?
                .content)
        }
        Some(other) => bail!("unsupported /agent action `{other}`"),
    }
}

async fn handle_test(executor: &ToolExecutor, args: Vec<String>) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("discover") => {
            let output = executor.execute("discover_tests", json!({})).await?;
            if output.content.trim().is_empty() {
                Ok("no test command discovered".to_string())
            } else {
                Ok(output.content)
            }
        }
        Some("run") => {
            let command = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            let args = if command.trim().is_empty() {
                json!({})
            } else {
                json!({ "command": command })
            };
            let output = executor.execute("run_tests", args).await?;
            Ok(output.content)
        }
        Some(other) => bail!("unsupported /test action `{other}`"),
    }
}

async fn handle_env(executor: &ToolExecutor, args: Vec<String>) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("check") => {
            let target = args.get(1).map(String::as_str).unwrap_or("auto");
            let output = executor
                .execute("check_environment", json!({ "target": target }))
                .await?;
            Ok(output.content)
        }
        Some("setup") | Some("install") => {
            let target = args
                .iter()
                .skip(1)
                .find(|arg| !arg.starts_with("--"))
                .map(String::as_str)
                .unwrap_or("docker");
            let smoke_test = args.iter().any(|arg| arg == "--smoke");
            let output = executor
                .execute(
                    "setup_environment",
                    json!({
                        "target": target,
                        "approved": true,
                        "install_missing": true,
                        "smoke_test": smoke_test
                    }),
                )
                .await?;
            Ok(output.content)
        }
        Some("test") => {
            let target = args.get(1).map(String::as_str).unwrap_or("docker");
            if target == "compiler" {
                let command = executor
                    .discover_tests()?
                    .into_iter()
                    .find(|command| {
                        command.requires_docker
                            && command.command.contains("maxxing/compiler-dev")
                            && command.command.contains("autotest -koopa -s lv1")
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("no compiler Docker autotest command discovered")
                    })?;
                let output = executor
                    .execute("run_tests", json!({ "command": command.command }))
                    .await?;
                Ok(output.content)
            } else {
                let output = executor
                    .execute(
                        "setup_environment",
                        json!({
                            "target": target,
                            "approved": true,
                            "install_missing": false,
                            "smoke_test": true
                        }),
                    )
                    .await?;
                Ok(output.content)
            }
        }
        Some(other) => bail!("unsupported /env action `{other}`"),
    }
}

async fn handle_review(executor: &ToolExecutor) -> Result<String> {
    let status = executor.execute("git_status", json!({})).await?.content;
    let diff = executor.execute("git_diff", json!({})).await?.content;
    Ok(review_worktree(&status, &diff))
}

fn update_project_permission_mode(workspace: &Path, mode: &str) -> Result<()> {
    if !matches!(mode, "read" | "write" | "full_control" | "sandbox") {
        bail!("unsupported permission mode `{mode}`");
    }
    let path = workspace.join(".deepcli").join("config.json");
    let raw = fs::read_to_string(&path)?;
    let mut value: Value = serde_json::from_str(&raw)?;
    value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("project config root must be an object"))?
        .entry("permissions")
        .or_insert_with(|| json!({}));
    value["permissions"]["defaultMode"] = Value::String(mode.to_string());
    fs::write(&path, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}

fn review_diff(diff: &str) -> String {
    if diff.trim().is_empty() {
        return "auto-reviewer: no local diff to review".to_string();
    }

    let mut high = Vec::new();
    let mut medium = Vec::new();
    let mut low = Vec::new();
    let added_lines = diff.lines().filter(|line| line.starts_with('+')).count();
    let removed_lines = diff.lines().filter(|line| line.starts_with('-')).count();

    for line in diff.lines() {
        if line.starts_with('+') && looks_sensitive(line) {
            high.push("added line appears to contain sensitive material");
        }
        if line.contains("rm -rf") || line.contains("git reset --hard") {
            high.push("diff contains a dangerous command pattern");
        }
        if line.starts_with("diff --git") && line.contains(".deepcli/credentials") {
            high.push("diff touches local credentials path");
        }
        if line.starts_with('+') && (line.contains("unwrap()") || line.contains("expect(")) {
            medium.push("added Rust panic-prone call; confirm it is acceptable");
        }
    }

    if added_lines + removed_lines > 500 {
        medium.push("large diff; consider splitting review scope");
    }
    if high.is_empty() && medium.is_empty() {
        low.push("no obvious high-risk pattern found");
    }

    let mut report = vec![
        "auto-reviewer report".to_string(),
        format!("changed lines: +{added_lines} -{removed_lines}"),
    ];
    append_findings(&mut report, "high", &high);
    append_findings(&mut report, "medium", &medium);
    append_findings(&mut report, "low", &low);
    report.join("\n")
}

fn review_worktree(status: &str, diff: &str) -> String {
    let mut report = review_diff(diff);
    let untracked = status
        .lines()
        .filter_map(|line| line.strip_prefix("?? "))
        .collect::<Vec<_>>();
    if !untracked.is_empty() {
        report.push_str("\nworktree:");
        report.push_str(&format!("\n- untracked files: {}", untracked.len()));
        for path in untracked.iter().take(8) {
            report.push_str(&format!("\n  - {path}"));
        }
        if untracked.len() > 8 {
            report.push_str("\n  - ...");
        }
    }
    report
}

fn append_findings(report: &mut Vec<String>, label: &str, findings: &[&str]) {
    if findings.is_empty() {
        return;
    }
    report.push(format!("{label}:"));
    for finding in findings {
        report.push(format!("- {finding}"));
    }
}

async fn handle_git(executor: &ToolExecutor, args: Vec<String>) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("status") => Ok(executor.execute("git_status", json!({})).await?.content),
        Some("diff") => Ok(executor.execute("git_diff", json!({})).await?.content),
        Some("branch") => Ok(executor.execute("git_branch", json!({})).await?.content),
        Some("message") => Ok(executor
            .execute("git_commit_message", json!({}))
            .await?
            .content),
        Some("create-branch") => {
            let name = required_arg(&args, 1, "branch name")?;
            Ok(executor
                .execute("git_create_branch", json!({"name": name, "approved": true}))
                .await?
                .content)
        }
        Some("commit") => {
            let message = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if message.trim().is_empty() {
                bail!("/git commit requires a message");
            }
            Ok(executor
                .execute("git_commit", json!({"message": message, "approved": true}))
                .await?
                .content)
        }
        Some(other) => bail!("unsupported /git action `{other}`"),
    }
}

fn handle_prompt(workspace: &Path, args: Vec<String>) -> Result<String> {
    let store = PromptStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => Ok(store
            .list()?
            .into_iter()
            .map(|prompt| format!("{} - {}", prompt.name, prompt.description))
            .collect::<Vec<_>>()
            .join("\n")),
        Some("get") => {
            let name = required_arg(&args, 1, "prompt name")?;
            Ok(store.get(name)?.body)
        }
        Some("save") => {
            let name = required_arg(&args, 1, "prompt name")?;
            let body = args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
            if body.trim().is_empty() {
                bail!("/prompt save requires a body");
            }
            let path = store.save(name, &body)?;
            Ok(path.display().to_string())
        }
        Some(other) => bail!("unsupported /prompt action `{other}`"),
    }
}

fn handle_skill(workspace: &Path, args: Vec<String>) -> Result<String> {
    let store = SkillStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => Ok(store
            .discover()?
            .into_iter()
            .map(|skill| format!("{} - {}", skill.name, skill.description))
            .collect::<Vec<_>>()
            .join("\n")),
        Some("generate") => {
            let name = required_arg(&args, 1, "skill name")?;
            let description = args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
            if description.trim().is_empty() {
                bail!("/skill generate requires a description");
            }
            Ok(store
                .generate(name, &description)?
                .instruction_path
                .display()
                .to_string())
        }
        Some("run") => {
            let name = required_arg(&args, 1, "skill name")?;
            Ok(store.load(name)?.instructions)
        }
        Some(other) => bail!("unsupported /skill action `{other}`"),
    }
}

fn required_arg<'a>(args: &'a [String], index: usize, name: &str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing {name}"))
}

#[allow(dead_code)]
fn _workspace_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parses_core_slash_commands() {
        assert_eq!(CommandRouter::parse("hello").unwrap(), None);
        assert_eq!(
            CommandRouter::parse("/help").unwrap(),
            Some(SlashCommand::Help)
        );
        assert_eq!(
            CommandRouter::parse("/diff --staged").unwrap(),
            Some(SlashCommand::Diff { staged: true })
        );
        assert_eq!(
            CommandRouter::parse("/resume abc").unwrap(),
            Some(SlashCommand::Resume {
                id: Some("abc".to_string())
            })
        );
        assert_eq!(
            CommandRouter::parse("/permissions set-mode write").unwrap(),
            Some(SlashCommand::Permissions {
                args: vec!["set-mode".to_string(), "write".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/test run cargo test").unwrap(),
            Some(SlashCommand::Test {
                args: vec!["run".to_string(), "cargo".to_string(), "test".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/env setup compiler --smoke").unwrap(),
            Some(SlashCommand::Env {
                args: vec![
                    "setup".to_string(),
                    "compiler".to_string(),
                    "--smoke".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/context").unwrap(),
            Some(SlashCommand::Context)
        );
        assert_eq!(
            CommandRouter::parse("/model set deepseek deepseek-v4-pro").unwrap(),
            Some(SlashCommand::Model {
                args: vec![
                    "set".to_string(),
                    "deepseek".to_string(),
                    "deepseek-v4-pro".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/session show abc").unwrap(),
            Some(SlashCommand::Session {
                args: vec!["show".to_string(), "abc".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/agent list").unwrap(),
            Some(SlashCommand::Agent {
                args: vec!["list".to_string()]
            })
        );
        assert_eq!(
            CommandRouter::parse("/btw ask quick question").unwrap(),
            Some(SlashCommand::Btw {
                args: vec![
                    "ask".to_string(),
                    "quick".to_string(),
                    "question".to_string()
                ]
            })
        );
        assert_eq!(
            CommandRouter::parse("/approval approve abc").unwrap(),
            Some(SlashCommand::Approval {
                args: vec!["approve".to_string(), "abc".to_string()]
            })
        );
    }

    #[test]
    fn help_contains_mvp_commands() {
        let help = CommandRouter::help_text();
        for command in CommandRouter::command_names() {
            assert!(help.contains(command), "{command} missing from help");
        }
    }

    #[test]
    fn review_diff_flags_sensitive_additions() {
        let report = review_diff("+api_key = secret\n");
        assert!(report.contains("high:"));
        assert!(report.contains("sensitive"));
    }

    #[test]
    fn review_worktree_reports_untracked_files() {
        let report = review_worktree("?? src/main.rs\n?? Cargo.toml\n", "");
        assert!(report.contains("untracked files: 2"));
        assert!(report.contains("src/main.rs"));
    }

    #[test]
    fn formats_side_questions_by_default_and_all() {
        let now = chrono::Utc::now();
        let open = SideQuestion {
            id: uuid::Uuid::new_v4(),
            question: "open item".to_string(),
            answer: None,
            status: SideQuestionStatus::Open,
            created_at: now,
            updated_at: now,
        };
        let answered = SideQuestion {
            id: uuid::Uuid::new_v4(),
            question: "answered item".to_string(),
            answer: Some("done".to_string()),
            status: SideQuestionStatus::Answered,
            created_at: now,
            updated_at: now,
        };
        let default = format_side_questions(&[open.clone(), answered.clone()], false);
        assert!(default.contains("open item"));
        assert!(!default.contains("answered item"));

        let all = format_side_questions(&[open, answered], true);
        assert!(all.contains("answered item"));
        assert!(all.contains("answer: done"));
    }

    #[test]
    fn formats_approval_requests_by_default_and_all() {
        let now = chrono::Utc::now();
        let pending = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            tool: "write_file".to_string(),
            decision: crate::permissions::PermissionDecision {
                outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                risk: crate::permissions::RiskLevel::High,
                reason: "write requires approval".to_string(),
            },
            status: ApprovalStatus::Pending,
            created_at: now,
            updated_at: now,
        };
        let approved = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            tool: "git_commit".to_string(),
            decision: crate::permissions::PermissionDecision {
                outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                risk: crate::permissions::RiskLevel::High,
                reason: "git write requires approval".to_string(),
            },
            status: ApprovalStatus::Approved,
            created_at: now,
            updated_at: now,
        };
        let default = format_approval_requests(&[pending.clone(), approved.clone()], false);
        assert!(default.contains("write_file"));
        assert!(!default.contains("git_commit"));

        let all = format_approval_requests(&[pending, approved], true);
        assert!(all.contains("git_commit"));
        assert!(all.contains("[approved]"));
    }

    #[test]
    fn updates_project_model_config() {
        let dir = tempdir().unwrap();
        let deepcli = dir.path().join(".deepcli");
        fs::create_dir_all(&deepcli).unwrap();
        fs::write(
            deepcli.join("config.json"),
            r#"{
              "version": 1,
              "defaultProvider": "deepseek",
              "providers": {
                "deepseek": {
                  "type": "deepseek",
                  "credentialsFile": ".deepcli/credentials/deepseek-credentials.json",
                  "acceptanceModel": "old"
                }
              }
            }"#,
        )
        .unwrap();
        update_project_model_config(dir.path(), "deepseek", Some("deepseek-v4-pro")).unwrap();
        let raw = fs::read_to_string(deepcli.join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["defaultProvider"], "deepseek");
        assert_eq!(
            value["providers"]["deepseek"]["acceptanceModel"],
            "deepseek-v4-pro"
        );
    }
}
