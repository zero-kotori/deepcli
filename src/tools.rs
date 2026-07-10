use crate::agents::{AgentStore, SubagentStatus, SubagentTask};
#[cfg(test)]
use crate::permissions::RiskLevel;
use crate::permissions::{
    DecisionOutcome, PermissionDecision, PermissionEngine, ToolRequest, ToolSurface,
};
use crate::privacy::{looks_sensitive, redact_sensitive_value};
use crate::prompts::{render_prompt_body, Prompt, PromptRenderContext, PromptStore};
use crate::session::{
    Plan, PlanStep, PlanStepStatus, Session, TestRunRecord, ToolCallRecord, ToolCallStatus,
};
use crate::skills::{SkillMetadata, SkillStore};
use crate::workspace::{DeepIgnore, WorkspaceManager};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use globset::{Glob, GlobSet};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) mod authorization;
mod declarations;
mod environment;
mod file;
mod git;
mod process;
mod schema;
mod test_discovery;
mod validation;
mod web;

use authorization::{invocation_digest, invocation_summary, validate_test_command};
pub use declarations::{ToolDeclaration, ToolObject, ToolPermissionContext, ToolRegistry};
use environment::{
    check_environment_in, environment_target_arg, format_environment_report,
    format_environment_setup, setup_environment_in,
};
#[cfg(test)]
use environment::{compiler_image_pull_command, docker_available, environment_ready};
pub use environment::{EnvironmentCheck, EnvironmentReport, EnvironmentSetupResult};
#[cfg(test)]
use file::normalize_unified_diff_hunk_counts;
pub use file::resolve_workspace_path;
use file::{
    normalize_patch_input, patch_target_paths, reject_large_destructive_rewrite,
    reject_large_existing_rewrite, reject_placeholder_overwrite, slice_text_by_line, unified_diff,
    validate_patch_paths,
};
use git::{generate_commit_message, validate_branch_name};
use process::{
    command_stdout_or_empty, default_shell_timeout_seconds, output_text, terminal_open_command,
};
pub use process::{
    run_command, run_command_blocking, run_command_with_stdin, run_command_with_timeout,
    CommandOutput,
};
use test_discovery::format_discovered_test_command;
pub use test_discovery::{discover_tests_in, DiscoveredTestCommand};
use validation::validate_tool_arguments;
use web::{
    bounded_web_fetch_chars, format_web_fetch_text, format_web_search_result,
    read_bounded_response_body, safe_web_get,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolExecution {
    pub tool: String,
    pub content: String,
    pub raw: Value,
    pub structured: StructuredToolResult,
    pub decision: PermissionDecision,
    #[serde(default = "default_tool_success")]
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredToolResult {
    pub kind: String,
    pub summary: String,
    pub data: Value,
    pub truncated: bool,
}

fn default_tool_success() -> bool {
    true
}

impl ToolExecution {
    fn new(
        tool: impl Into<String>,
        content: impl Into<String>,
        raw: Value,
        decision: PermissionDecision,
    ) -> Self {
        let tool = tool.into();
        let content = content.into();
        let summary = first_line(&content).to_string();
        Self {
            tool,
            content,
            structured: StructuredToolResult {
                kind: "text".to_string(),
                summary,
                data: raw.clone(),
                truncated: false,
            },
            raw,
            decision,
            success: true,
        }
    }

    fn with_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    fn with_structured(
        mut self,
        kind: impl Into<String>,
        summary: impl Into<String>,
        data: Value,
        truncated: bool,
    ) -> Self {
        self.structured = StructuredToolResult {
            kind: kind.into(),
            summary: summary.into(),
            data,
            truncated,
        };
        self
    }

    pub fn prompt_content(&self) -> String {
        serde_json::to_string(&json!({
            "tool": self.tool,
            "ok": self.success,
            "kind": self.structured.kind,
            "summary": self.structured.summary,
            "content": self.content,
            "data": self.structured.data,
            "truncated": self.structured.truncated,
        }))
        .unwrap_or_else(|_| self.content.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SubagentBackgroundStart {
    status: String,
    pid: Option<u32>,
    reason: Option<String>,
    command: Vec<String>,
}

#[derive(Debug, Clone)]
struct SubagentCapability {
    allowed_tools: Option<BTreeSet<String>>,
    read_scope: Option<Vec<PathBuf>>,
    write_scope: Option<Vec<PathBuf>>,
}

impl SubagentCapability {
    fn from_task(workspace: &Path, task: &SubagentTask) -> Result<Self> {
        let allowed_tools = (!task.allowed_tools.is_empty())
            .then(|| task.allowed_tools.iter().cloned().collect::<BTreeSet<_>>());
        let read_scope = resolve_capability_scope(workspace, &task.read_scope)?;
        let write_scope = resolve_capability_scope(workspace, &task.write_scope)?;
        Ok(Self {
            allowed_tools,
            read_scope,
            write_scope,
        })
    }

    fn has_path_restrictions(&self) -> bool {
        self.read_scope.is_some() || self.write_scope.is_some()
    }
}

pub struct ToolExecutor {
    workspace: PathBuf,
    permissions: PermissionEngine,
    session: Option<Session>,
    max_subagent_depth: u8,
    current_subagent_depth: u8,
    assume_yes: bool,
    subagent_capability: Option<SubagentCapability>,
}

impl ToolExecutor {
    pub fn new(
        workspace: impl AsRef<Path>,
        permissions: PermissionEngine,
        session: Option<Session>,
        max_subagent_depth: u8,
    ) -> Self {
        let workspace = workspace.as_ref();
        let workspace = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        Self {
            workspace,
            permissions,
            session,
            max_subagent_depth,
            current_subagent_depth: 0,
            assume_yes: false,
            subagent_capability: None,
        }
    }

    pub fn with_assume_yes(mut self, assume_yes: bool) -> Self {
        self.assume_yes = assume_yes;
        self
    }

    pub fn set_session(&mut self, session: Option<Session>) {
        self.session = session;
    }

    pub(crate) fn restrict_to_subagent(&mut self, task: &SubagentTask) -> Result<()> {
        if task.depth == 0 || task.depth > self.max_subagent_depth {
            bail!(
                "sub-agent depth {} is outside the configured range 1..={}",
                task.depth,
                self.max_subagent_depth
            );
        }
        validate_allowed_subagent_tools(&task.allowed_tools)?;
        let capability = SubagentCapability::from_task(&self.workspace, task)?;
        self.current_subagent_depth = task.depth;
        self.subagent_capability = Some(capability);
        Ok(())
    }

    pub(crate) fn subagent_read_scope(&self) -> Option<&[PathBuf]> {
        self.subagent_capability
            .as_ref()
            .and_then(|capability| capability.read_scope.as_deref())
    }

    pub async fn execute_user_action(&self, name: &str, args: Value) -> Result<ToolExecution> {
        let executor = Self {
            workspace: self.workspace.clone(),
            permissions: self.permissions.clone(),
            session: self.session.clone(),
            max_subagent_depth: self.max_subagent_depth,
            current_subagent_depth: self.current_subagent_depth,
            assume_yes: true,
            subagent_capability: self.subagent_capability.clone(),
        };
        executor.execute(name, args).await
    }

    pub fn execute_open_terminal_now(&self) -> Result<ToolExecution> {
        self.execute_open_terminal_app_now("Terminal")
    }

    pub fn execute_open_terminal_app_now(&self, app: &str) -> Result<ToolExecution> {
        let name = "open_terminal";
        let original_args = json!({ "app": app });
        if let Some(session) = &self.session {
            self.append_tool_lifecycle(
                session,
                name,
                &original_args,
                Value::Null,
                None,
                ToolCallStatus::Requested,
            )?;
            self.append_tool_lifecycle(
                session,
                name,
                &original_args,
                Value::Null,
                None,
                ToolCallStatus::PolicyChecking,
            )?;
            self.append_tool_lifecycle(
                session,
                name,
                &original_args,
                Value::Null,
                None,
                ToolCallStatus::Running,
            )?;
        }

        let result = self.open_terminal_app_now(app);
        if let Some(session) = &self.session {
            let (status, output) = match &result {
                Ok(execution) if execution.success => {
                    (ToolCallStatus::Succeeded, execution.raw.clone())
                }
                Ok(execution) => (ToolCallStatus::Failed, execution.raw.clone()),
                Err(error) => (ToolCallStatus::Failed, json!({"error": error.to_string()})),
            };
            let decision = result
                .as_ref()
                .ok()
                .map(|execution| execution.decision.clone());
            if let Some(decision) = &decision {
                if let Some(approval_status) = approval_status_for(decision.outcome) {
                    self.append_tool_lifecycle(
                        session,
                        name,
                        &original_args,
                        Value::Null,
                        Some(decision.clone()),
                        approval_status,
                    )?;
                }
            }
            self.append_tool_lifecycle(session, name, &original_args, output, decision, status)?;
        }

        result
    }

    pub async fn execute(&self, name: &str, args: Value) -> Result<ToolExecution> {
        let original_args = args.clone();
        if let Some(session) = &self.session {
            self.append_tool_lifecycle(
                session,
                name,
                &original_args,
                Value::Null,
                None,
                ToolCallStatus::Requested,
            )?;
            self.append_tool_lifecycle(
                session,
                name,
                &original_args,
                Value::Null,
                None,
                ToolCallStatus::PolicyChecking,
            )?;
            self.append_tool_lifecycle(
                session,
                name,
                &original_args,
                Value::Null,
                None,
                ToolCallStatus::Running,
            )?;
        }
        let validation = self.ensure_subagent_tool_allowed(name).and_then(|_| {
            if let Some(tool) = ToolRegistry::mvp().tool(name) {
                tool.validate_arguments(&args)
            } else {
                validate_tool_arguments(name, &args)
            }
        });
        let result = if let Err(error) = validation {
            Err(error)
        } else {
            match name {
                "read_file" => self.read_file(args).await,
                "list_files" => self.list_files(args).await,
                "search" => self.search(args).await,
                "write_file" => self.write_file(name, args).await,
                "apply_patch_or_write" => {
                    if args.get("patch").is_some() {
                        self.apply_patch(args).await
                    } else if args.get("old").is_some() && args.get("new").is_some() {
                        self.replace_in_file(name, args).await
                    } else {
                        self.write_file(name, args).await
                    }
                }
                "run_shell" => self.run_shell(args).await,
                "git_status" => self.git_status().await,
                "git_diff" => self.git_diff(args).await,
                "git_branch" => self.git_branch().await,
                "git_create_branch" => self.git_create_branch(args).await,
                "git_commit_message" => self.git_commit_message().await,
                "git_commit" => self.git_commit(args).await,
                "discover_tests" => self.discover_tests_tool().await,
                "run_tests" => self.run_tests(args).await,
                "check_environment" => self.check_environment(args).await,
                "setup_environment" => self.setup_environment(args).await,
                "todo_write" => self.todo_write(args).await,
                "ask_user_question" => self.ask_user_question(args).await,
                "web_search" => self.web_search(args).await,
                "web_fetch" => self.web_fetch(args).await,
                "open_terminal" => self.open_terminal().await,
                "prompt_list" => self.prompt_list().await,
                "prompt_get" => self.prompt_get(args).await,
                "prompt_render" => self.prompt_render(args).await,
                "skill_list" => self.skill_list().await,
                "skill_generate" => self.skill_generate(args).await,
                "skill_run" => self.skill_run(args).await,
                "spawn_subagent" => self.spawn_subagent(args).await,
                other => Err(anyhow!("unknown tool `{other}`")),
            }
        };

        if let Some(session) = &self.session {
            let (status, output) = match &result {
                Ok(execution) if execution.success => {
                    (ToolCallStatus::Succeeded, execution.raw.clone())
                }
                Ok(execution) => (ToolCallStatus::Failed, execution.raw.clone()),
                Err(error) => (ToolCallStatus::Failed, json!({"error": error.to_string()})),
            };
            let decision = result
                .as_ref()
                .ok()
                .map(|execution| execution.decision.clone());
            if let Some(decision) = &decision {
                if let Some(approval_status) = approval_status_for(decision.outcome) {
                    self.append_tool_lifecycle(
                        session,
                        name,
                        &original_args,
                        Value::Null,
                        Some(decision.clone()),
                        approval_status,
                    )?;
                }
            }
            self.append_tool_lifecycle(session, name, &original_args, output, decision, status)?;
        }

        result
    }

    fn append_tool_lifecycle(
        &self,
        session: &Session,
        name: &str,
        input: &Value,
        output: Value,
        decision: Option<PermissionDecision>,
        status: ToolCallStatus,
    ) -> Result<()> {
        session.append_tool_call(&ToolCallRecord {
            tool: name.to_string(),
            input: redact_sensitive_value(input),
            output: redact_sensitive_value(&output),
            decision,
            status,
            created_at: Utc::now(),
        })
    }

    pub fn discover_tests(&self) -> Result<Vec<DiscoveredTestCommand>> {
        discover_tests_in(&self.workspace)
    }

    async fn read_file(&self, args: Value) -> Result<ToolExecution> {
        let path = self.resolve_required_path(&args)?;
        let start_line = args.get("start_line").and_then(Value::as_u64).unwrap_or(1) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let decision = self.evaluate_filesystem("read_file", &path, false)?;
        self.ensure_allowed("read_file", &decision, &args)?;
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let content = slice_text_by_line(&content, start_line.max(1), limit);
        let raw = json!({"path": path, "content": content});
        Ok(
            ToolExecution::new("read_file", content.clone(), raw.clone(), decision)
                .with_structured("file_content", first_line(&content), raw, false),
        )
    }

    async fn list_files(&self, args: Value) -> Result<ToolExecution> {
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(256) as usize;
        let base_path = self
            .resolve_optional_path(&args)?
            .unwrap_or(self.workspace.clone());
        let decision = self.evaluate_filesystem("list_files", &base_path, false)?;
        self.ensure_allowed("list_files", &decision, &args)?;
        let manager = WorkspaceManager::new(&self.workspace)?;
        let glob = optional_glob(&args)?;
        let walk_limit = limit
            .saturating_mul(20)
            .max(limit.saturating_add(1))
            .min(4096);
        let files = manager
            .walk_files_from(&base_path, walk_limit)?
            .into_iter()
            .filter(|entry| path_matches_glob(entry.path(), &self.workspace, glob.as_ref()))
            .map(|entry| {
                entry
                    .path()
                    .strip_prefix(&self.workspace)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .to_string()
            })
            .take(limit.saturating_add(1))
            .collect::<Vec<_>>();
        let truncated = files.len() > limit;
        let files = files.into_iter().take(limit).collect::<Vec<_>>();
        let count = files.len();
        let content = files.join("\n");
        let raw = json!({
            "files": files.clone(),
            "count": count,
            "truncated": truncated,
            "path": relative_path_string(&self.workspace, &base_path),
            "glob": args.get("glob").and_then(Value::as_str)
        });
        Ok(
            ToolExecution::new("list_files", content.clone(), raw.clone(), decision)
                .with_structured("file_list", format!("{count} file(s)"), raw, truncated),
        )
    }

    async fn search(&self, args: Value) -> Result<ToolExecution> {
        let query = required_str(&args, "query")?;
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
        let base_path = self
            .resolve_optional_path(&args)?
            .unwrap_or(self.workspace.clone());
        let decision = self.evaluate_filesystem("search", &base_path, false)?;
        self.ensure_allowed("search", &decision, &args)?;
        let manager = WorkspaceManager::new(&self.workspace)?;
        let glob = optional_glob(&args)?;
        let case_sensitive = args
            .get("case_sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let context_lines = args
            .get("context_lines")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let max_file_bytes = args
            .get("max_file_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(1_000_000);
        let needle = if case_sensitive {
            query.to_string()
        } else {
            query.to_ascii_lowercase()
        };
        let mut matches = Vec::new();
        let mut searched_files = 0usize;
        for entry in manager.walk_files_from(&base_path, 4096)? {
            if !path_matches_glob(entry.path(), &self.workspace, glob.as_ref()) {
                continue;
            }
            if entry.metadata().map(|meta| meta.len()).unwrap_or(0) > max_file_bytes {
                continue;
            }
            let Ok(content) = fs::read_to_string(entry.path()) else {
                continue;
            };
            searched_files += 1;
            let lines = content.lines().collect::<Vec<_>>();
            for (line_number, line) in lines.iter().enumerate() {
                let haystack = if case_sensitive {
                    (*line).to_string()
                } else {
                    line.to_ascii_lowercase()
                };
                if haystack.contains(&needle) {
                    let path = entry
                        .path()
                        .strip_prefix(&self.workspace)
                        .unwrap_or(entry.path())
                        .to_string_lossy()
                        .to_string();
                    let before_start = line_number.saturating_sub(context_lines);
                    let before = lines[before_start..line_number]
                        .iter()
                        .map(|line| (*line).to_string())
                        .collect::<Vec<_>>();
                    let after_end = (line_number + 1 + context_lines).min(lines.len());
                    let after = lines[line_number + 1..after_end]
                        .iter()
                        .map(|line| (*line).to_string())
                        .collect::<Vec<_>>();
                    matches.push(json!({
                        "path": path,
                        "line": line_number + 1,
                        "text": *line,
                        "before": before,
                        "after": after
                    }));
                    if matches.len() >= limit {
                        break;
                    }
                }
            }
            if matches.len() >= limit {
                break;
            }
        }
        let content = format_search_matches(&matches)?;
        let count = matches.len();
        let truncated = count >= limit;
        let raw = json!({
            "matches": matches.clone(),
            "count": count,
            "searched_files": searched_files,
            "truncated": truncated,
            "case_sensitive": case_sensitive,
            "glob": args.get("glob").and_then(Value::as_str)
        });
        Ok(
            ToolExecution::new("search", content, raw.clone(), decision).with_structured(
                "search_matches",
                format!("{count} match(es)"),
                raw,
                truncated,
            ),
        )
    }

    async fn write_file(&self, name: &str, args: Value) -> Result<ToolExecution> {
        let path = self.resolve_required_path(&args)?;
        let content = required_str(&args, "content")?;
        let decision = self.evaluate_filesystem(name, &path, true)?;
        let authorization_args = authorization_args_with_resolved_path(&args, &path);
        self.ensure_allowed(name, &decision, &authorization_args)?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let before = fs::read_to_string(&path).unwrap_or_default();
        reject_placeholder_overwrite(&path, &before, content)?;
        reject_large_destructive_rewrite(&path, &before, content)?;
        reject_large_existing_rewrite(&path, &before, content)?;
        let rel = path
            .strip_prefix(&self.workspace)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        if !self.workspace.join(".git").exists() {
            if let Some(session) = &self.session {
                session.save_backup(&rel, &before)?;
            }
        }
        fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
        let diff = unified_diff(&before, content, &path);
        if let Some(session) = &self.session {
            session.save_diff(&rel, &diff)?;
        }

        let raw = json!({"path": path, "diff": diff});
        Ok(
            ToolExecution::new(name, diff.clone(), raw.clone(), decision).with_structured(
                "file_diff",
                "file written",
                raw,
                false,
            ),
        )
    }

    async fn apply_patch(&self, args: Value) -> Result<ToolExecution> {
        let patch = required_str(&args, "patch")?;
        let patch_to_apply = normalize_patch_input(patch, args.get("path").and_then(Value::as_str));
        validate_patch_paths(&self.workspace, &patch_to_apply)?;
        let mut resolved_targets = Vec::new();
        for target in patch_target_paths(&patch_to_apply)? {
            let target = target
                .to_str()
                .ok_or_else(|| anyhow!("patch path is not valid UTF-8: {}", target.display()))?;
            resolved_targets.push(self.resolve_provider_path(target)?);
        }
        for target in &resolved_targets {
            self.ensure_subagent_path_allowed(target, true)?;
        }
        let permission_path = resolved_targets.first().unwrap_or(&self.workspace);
        let decision = self.evaluate_filesystem("apply_patch_or_write", permission_path, true)?;
        let authorization_args = json!({
            "patch": patch_to_apply,
            "targets": resolved_targets,
        });
        self.ensure_allowed("apply_patch_or_write", &decision, &authorization_args)?;
        let check =
            run_command_with_stdin(&self.workspace, "git apply --check -", &patch_to_apply).await?;
        if check.exit_code != Some(0) {
            bail!("patch check failed:\n{}", output_text(&check));
        }
        let output =
            run_command_with_stdin(&self.workspace, "git apply -", &patch_to_apply).await?;
        if output.exit_code != Some(0) {
            bail!("patch apply failed:\n{}", output_text(&output));
        }
        if let Some(session) = &self.session {
            session.save_diff("applied_patch", &patch_to_apply)?;
        }
        let raw = json!({"patch": patch_to_apply, "output": output});
        Ok(ToolExecution::new(
            "apply_patch_or_write",
            patch_to_apply.clone(),
            raw.clone(),
            decision,
        )
        .with_structured("file_diff", "patch applied", raw, false))
    }

    async fn replace_in_file(&self, name: &str, args: Value) -> Result<ToolExecution> {
        let path = self.resolve_required_path(&args)?;
        let old = required_str(&args, "old")?;
        let new = required_str(&args, "new")?;
        let decision = self.evaluate_filesystem(name, &path, true)?;
        let authorization_args = authorization_args_with_resolved_path(&args, &path);
        self.ensure_allowed(name, &decision, &authorization_args)?;

        let before = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let matches = before.matches(old).count();
        if matches == 0 {
            bail!("old text was not found in {}", path.display());
        }
        if matches > 1 {
            bail!(
                "old text matched {matches} times in {}; provide a more specific snippet",
                path.display()
            );
        }
        let after = before.replacen(old, new, 1);
        let rel = path
            .strip_prefix(&self.workspace)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        if !self.workspace.join(".git").exists() {
            if let Some(session) = &self.session {
                session.save_backup(&rel, &before)?;
            }
        }
        fs::write(&path, &after).with_context(|| format!("failed to write {}", path.display()))?;
        let diff = unified_diff(&before, &after, &path);
        if let Some(session) = &self.session {
            session.save_diff(&rel, &diff)?;
        }

        let raw = json!({"path": path, "diff": diff});
        Ok(
            ToolExecution::new(name, diff.clone(), raw.clone(), decision).with_structured(
                "file_diff",
                "file edited",
                raw,
                false,
            ),
        )
    }

    async fn run_shell(&self, args: Value) -> Result<ToolExecution> {
        let command = required_str(&args, "command")?;
        let decision = self.evaluate_declared_tool(
            "run_shell",
            ToolPermissionContext {
                command: Some(command.to_string()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("run_shell", &decision, &args)?;
        let timeout_seconds = bounded_timeout_seconds(&args, default_shell_timeout_seconds());
        let output = run_command_with_timeout(
            &self.workspace,
            command,
            Duration::from_secs(timeout_seconds),
        )
        .await?;
        let content = output_text(&output);
        let success = output.exit_code == Some(0);
        let raw = json!(output);
        Ok(
            ToolExecution::new("run_shell", content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false)
                .with_success(success),
        )
    }

    async fn git_status(&self) -> Result<ToolExecution> {
        let command =
            "git -c core.fsmonitor=false status --porcelain=v1 --untracked-files=all --no-renames";
        let decision = self.evaluate_declared_tool(
            "git_status",
            ToolPermissionContext {
                command: Some(command.to_string()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("git_status", &decision, &json!({}))?;
        let output = self.safe_git_status().await?;
        let content = output_text(&output);
        let success = output.exit_code == Some(0);
        let raw = json!(output);
        Ok(
            ToolExecution::new("git_status", content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false)
                .with_success(success),
        )
    }

    async fn git_diff(&self, args: Value) -> Result<ToolExecution> {
        let staged = args.get("staged").and_then(Value::as_bool).unwrap_or(false);
        let command = if staged {
            "git --literal-pathspecs diff --cached --no-renames -- <non-ignored paths>"
        } else {
            "git --literal-pathspecs diff --no-renames -- <non-ignored paths>"
        };
        let decision = self.evaluate_declared_tool(
            "git_diff",
            ToolPermissionContext {
                command: Some(command.to_string()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("git_diff", &decision, &args)?;
        let output = match self.safe_git_diff_paths(staged).await {
            Ok(paths) => self.run_safe_git_diff(staged, &paths, false).await?,
            Err(error) => CommandOutput {
                command: command.to_string(),
                exit_code: Some(128),
                stdout: String::new(),
                stderr: error.to_string(),
            },
        };
        let content = output_text(&output);
        let success = output.exit_code == Some(0);
        let raw = json!(output);
        Ok(
            ToolExecution::new("git_diff", content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false)
                .with_success(success),
        )
    }

    async fn git_branch(&self) -> Result<ToolExecution> {
        self.git_read_tool(
            "git_branch",
            "git branch --show-current && git branch --list",
        )
        .await
    }

    async fn git_create_branch(&self, args: Value) -> Result<ToolExecution> {
        let name = required_str(&args, "name")?;
        validate_branch_name(name)?;
        let command = format!("git switch -c {}", shell_words::quote(name));
        let decision = self.evaluate_declared_tool(
            "git_create_branch",
            ToolPermissionContext {
                command: Some(command.clone()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("git_create_branch", &decision, &args)?;
        let output = run_command(&self.workspace, &command).await?;
        let content = output_text(&output);
        let success = output.exit_code == Some(0);
        let raw = json!(output);
        Ok(
            ToolExecution::new("git_create_branch", content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false)
                .with_success(success),
        )
    }

    async fn git_commit_message(&self) -> Result<ToolExecution> {
        let decision = self.evaluate_declared_tool(
            "git_commit_message",
            ToolPermissionContext {
                command: Some("git status --short && git diff --stat".to_string()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("git_commit_message", &decision, &json!({}))?;
        let status = self.safe_git_status().await?;
        let (changed_files, stat) = match self.safe_git_diff_paths(false).await {
            Ok(changed_files) => {
                let stat = self.run_safe_git_diff(false, &changed_files, true).await?;
                (changed_files, stat)
            }
            Err(error) => (
                Vec::new(),
                CommandOutput {
                    command: "git diff --stat".to_string(),
                    exit_code: Some(128),
                    stdout: String::new(),
                    stderr: error.to_string(),
                },
            ),
        };
        let names = changed_files.join("\n");
        let message = generate_commit_message(&status.stdout, &names);
        let raw = json!({
            "message": message,
            "status": status,
            "changed_files": changed_files,
            "stat": stat.stdout
        });
        let success = status.exit_code == Some(0) && stat.exit_code == Some(0);
        Ok(
            ToolExecution::new("git_commit_message", message.clone(), raw.clone(), decision)
                .with_structured("git_commit_message", message, raw, false)
                .with_success(success),
        )
    }

    async fn git_commit(&self, args: Value) -> Result<ToolExecution> {
        let message = required_str(&args, "message")?;
        let (staged_paths, staged_digest) = self.staged_commit_snapshot().await?;
        if staged_paths.is_empty() {
            bail!("git_commit requires at least one staged change");
        }
        let command = "git commit-tree <approved-tree> && git update-ref HEAD <commit> <old-head>";
        let decision = self.evaluate_declared_tool(
            "git_commit",
            ToolPermissionContext {
                command: Some(command.to_string()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        let mut authorization_args = args.clone();
        if let Value::Object(arguments) = &mut authorization_args {
            arguments.insert(
                "_staged_digest".to_string(),
                Value::String(staged_digest.clone()),
            );
            arguments.insert("_staged_files".to_string(), json!(staged_paths));
        }
        self.ensure_allowed("git_commit", &decision, &authorization_args)?;
        let (_, current_digest) = self.staged_commit_snapshot().await?;
        if current_digest != staged_digest {
            bail!("staged changes changed after approval; review and approve the new snapshot");
        }
        let output = self
            .create_commit_from_approved_tree(message, &staged_digest)
            .await?;
        let content = output_text(&output);
        let success = output.exit_code == Some(0);
        let raw = json!(output);
        Ok(
            ToolExecution::new("git_commit", content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false)
                .with_success(success),
        )
    }

    async fn git_read_tool(&self, name: &str, command: &str) -> Result<ToolExecution> {
        let decision = self.evaluate_declared_tool(
            name,
            ToolPermissionContext {
                command: Some(command.to_string()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed(name, &decision, &json!({ "command": command }))?;
        let output = run_command(&self.workspace, command).await?;
        let content = output_text(&output);
        let success = output.exit_code == Some(0);
        let raw = json!(output);
        Ok(
            ToolExecution::new(name, content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false)
                .with_success(success),
        )
    }

    async fn safe_git_diff_paths(&self, staged: bool) -> Result<Vec<String>> {
        let paths = self.raw_git_diff_paths(staged).await?;
        let ignore = DeepIgnore::load(&self.workspace)?;
        Ok(paths
            .into_iter()
            .filter(|path| !ignore.is_ignored(self.workspace.join(path)))
            .collect())
    }

    async fn raw_git_diff_paths(&self, staged: bool) -> Result<Vec<String>> {
        let command = if staged {
            "git --literal-pathspecs diff --cached --no-renames --no-ext-diff --no-textconv --name-only -z"
        } else {
            "git --literal-pathspecs diff --no-renames --no-ext-diff --no-textconv --name-only -z"
        };
        let output = run_command(&self.workspace, command).await?;
        if output.exit_code != Some(0) {
            bail!(
                "failed to enumerate safe git diff paths: {}",
                output_text(&output)
            );
        }
        Ok(output
            .stdout
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .collect())
    }

    async fn staged_commit_snapshot(&self) -> Result<(Vec<String>, String)> {
        let paths = self.raw_git_diff_paths(true).await?;
        let ignore = DeepIgnore::load(&self.workspace)?;
        if paths
            .iter()
            .any(|path| ignore.is_ignored(self.workspace.join(path)))
        {
            bail!("git_commit refused because staged changes include paths blocked by workspace policy");
        }
        if paths.is_empty() {
            return Ok((paths, String::new()));
        }
        self.ensure_no_in_progress_git_operation().await?;
        let output = run_command(&self.workspace, "git write-tree").await?;
        if output.exit_code != Some(0) {
            bail!(
                "failed to fingerprint staged changes: {}",
                output_text(&output)
            );
        }
        let tree = parse_git_oid(&output.stdout, "staged tree")?;
        Ok((paths, tree))
    }

    async fn ensure_no_in_progress_git_operation(&self) -> Result<()> {
        for reference in ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
            let output = run_command(
                &self.workspace,
                &format!("git rev-parse -q --verify {reference}"),
            )
            .await?;
            if output.exit_code == Some(0) {
                bail!("git_commit does not support repositories with an active {reference}");
            }
        }
        Ok(())
    }

    async fn create_commit_from_approved_tree(
        &self,
        message: &str,
        approved_tree: &str,
    ) -> Result<CommandOutput> {
        let tree = parse_git_oid(approved_tree, "approved tree")?;
        let head_output =
            run_command(&self.workspace, "git rev-parse -q --verify HEAD^{commit}").await?;
        let parent = if head_output.exit_code == Some(0) {
            Some(parse_git_oid(&head_output.stdout, "HEAD")?)
        } else {
            None
        };
        let mut commit_tree_command = format!("git commit-tree {tree}");
        if let Some(parent) = &parent {
            commit_tree_command.push_str(" -p ");
            commit_tree_command.push_str(parent);
        }
        let commit_output = run_command_with_stdin(
            &self.workspace,
            &commit_tree_command,
            &format!("{}\n", message.trim_end()),
        )
        .await?;
        if commit_output.exit_code != Some(0) {
            return Ok(commit_output);
        }
        let commit = parse_git_oid(&commit_output.stdout, "created commit")?;
        let expected_head = parent.unwrap_or_else(|| "0".repeat(tree.len()));
        let update_command =
            format!("git update-ref -m deepcli-git-commit HEAD {commit} {expected_head}");
        let update_output = run_command(&self.workspace, &update_command).await?;
        if update_output.exit_code != Some(0) {
            bail!(
                "HEAD changed while creating the approved commit: {}",
                output_text(&update_output)
            );
        }
        Ok(CommandOutput {
            command: format!("{commit_tree_command} && git update-ref HEAD"),
            exit_code: Some(0),
            stdout: format!("created commit {commit}\n"),
            stderr: commit_output.stderr,
        })
    }

    async fn run_safe_git_diff(
        &self,
        staged: bool,
        paths: &[String],
        stat: bool,
    ) -> Result<CommandOutput> {
        let mut command =
            "git --literal-pathspecs diff --no-renames --no-ext-diff --no-textconv".to_string();
        if staged {
            command.push_str(" --cached");
        }
        if stat {
            command.push_str(" --stat");
        }
        command.push_str(" --");
        for path in paths {
            command.push(' ');
            command.push_str(&shell_words::quote(path));
        }
        if paths.is_empty() {
            return Ok(CommandOutput {
                command,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        run_command(&self.workspace, &command).await
    }

    async fn safe_git_status(&self) -> Result<CommandOutput> {
        let command =
            "git -c core.fsmonitor=false status --porcelain=v1 -z --untracked-files=all --no-renames";
        let mut output = run_command(&self.workspace, command).await?;
        if output.exit_code != Some(0) {
            return Ok(output);
        }
        let ignore = DeepIgnore::load(&self.workspace)?;
        output.stdout = output
            .stdout
            .split('\0')
            .filter(|record| record.len() >= 4)
            .filter(|record| !ignore.is_ignored(self.workspace.join(&record[3..])))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(output)
    }

    async fn discover_tests_tool(&self) -> Result<ToolExecution> {
        let decision = self.evaluate_filesystem("discover_tests", &self.workspace, false)?;
        self.ensure_allowed("discover_tests", &decision, &json!({}))?;
        let commands = self.discover_tests()?;
        let content = commands
            .iter()
            .map(format_discovered_test_command)
            .collect::<Vec<_>>()
            .join("\n");
        let raw = json!({ "commands": commands });
        Ok(
            ToolExecution::new("discover_tests", content.clone(), raw.clone(), decision)
                .with_structured("test_commands", first_line(&content), raw, false),
        )
    }

    async fn run_tests(&self, args: Value) -> Result<ToolExecution> {
        let command = if let Some(command) = args.get("command").and_then(Value::as_str) {
            command.to_string()
        } else {
            self.discover_tests()?
                .into_iter()
                .find(|command| command.available != Some(false))
                .map(|command| command.command)
                .ok_or_else(|| anyhow!("no available test command discovered"))?
        };
        validate_test_command(&self.workspace, &command)?;
        let mut authorization_args = args.clone();
        if let Value::Object(arguments) = &mut authorization_args {
            arguments.insert("command".to_string(), Value::String(command.clone()));
        }
        let decision = self.evaluate_declared_tool(
            "run_tests",
            ToolPermissionContext {
                command: Some(command.clone()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("run_tests", &decision, &authorization_args)?;
        let output = run_command_with_timeout(
            &self.workspace,
            &command,
            Duration::from_secs(default_shell_timeout_seconds()),
        )
        .await?;
        let passed = output.exit_code == Some(0);
        if let Some(session) = &self.session {
            session.append_test_run(&TestRunRecord {
                command,
                exit_code: output.exit_code,
                stdout: output.stdout.clone(),
                stderr: output.stderr.clone(),
                passed,
                created_at: Utc::now(),
            })?;
        }
        let content = output_text(&output);
        let raw = json!({"passed": passed, "output": output});
        Ok(
            ToolExecution::new("run_tests", content.clone(), raw.clone(), decision)
                .with_structured(
                    "test_output",
                    if passed {
                        "tests passed"
                    } else {
                        "tests failed"
                    },
                    raw,
                    false,
                )
                .with_success(passed),
        )
    }

    async fn check_environment(&self, args: Value) -> Result<ToolExecution> {
        let target = environment_target_arg(&args)?;
        let decision = self.evaluate_declared_tool(
            "check_environment",
            ToolPermissionContext {
                command: Some(format!("deepcli environment check {target}")),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("check_environment", &decision, &args)?;
        let report = check_environment_in(&self.workspace, &target).await?;
        let content = format_environment_report(&report);
        let raw = json!(report);
        Ok(
            ToolExecution::new("check_environment", content.clone(), raw.clone(), decision)
                .with_structured("environment_report", first_line(&content), raw, false),
        )
    }

    async fn setup_environment(&self, args: Value) -> Result<ToolExecution> {
        let target = environment_target_arg(&args)?;
        let install_missing = bool_arg(&args, "install_missing", true);
        let smoke_test = bool_arg(&args, "smoke_test", false);
        let decision = self.evaluate_declared_tool(
            "setup_environment",
            ToolPermissionContext {
                command: Some(format!("deepcli environment setup {target}")),
                path: Some(self.workspace.clone()),
                network_target: Some("ghcr.io, docker.io".to_string()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("setup_environment", &decision, &args)?;
        let setup =
            setup_environment_in(&self.workspace, &target, install_missing, smoke_test).await?;
        let content = format_environment_setup(&setup);
        let success = setup.ready;
        let raw = json!(setup);
        Ok(
            ToolExecution::new("setup_environment", content.clone(), raw.clone(), decision)
                .with_structured("environment_setup", first_line(&content), raw, false)
                .with_success(success),
        )
    }

    async fn todo_write(&self, args: Value) -> Result<ToolExecution> {
        let decision = self.evaluate_declared_tool(
            "todo_write",
            ToolPermissionContext {
                path: Some(self.workspace.clone()),
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("todo_write", &decision, &args)?;
        let title = args
            .get("title")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Session todo list")
            .trim()
            .to_string();
        let steps = todo_steps_from_args(&args)?;
        if let Some(session) = &self.session {
            session.save_plan(&Plan {
                title: title.clone(),
                steps: steps.clone(),
                updated_at: Utc::now(),
            })?;
        }
        let todos = steps.iter().map(plan_step_json).collect::<Vec<_>>();
        let content = format_todo_steps(&steps);
        let raw = json!({
            "title": title,
            "todos": todos,
            "count": steps.len()
        });
        Ok(
            ToolExecution::new("todo_write", content, raw.clone(), decision).with_structured(
                "todo_list",
                format!("{} todo(s) updated", steps.len()),
                raw,
                false,
            ),
        )
    }

    async fn ask_user_question(&self, args: Value) -> Result<ToolExecution> {
        let question = required_str(&args, "question")?.trim();
        let options = optional_string_array(&args, "options")?;
        let decision = self.evaluate_declared_tool(
            "ask_user_question",
            ToolPermissionContext {
                path: Some(self.workspace.clone()),
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("ask_user_question", &decision, &args)?;
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("ask_user_question requires an active session"))?;
        let item = session.enqueue_side_question_with_options(question, options)?;
        let content = format!(
            "queued user question {}: {}",
            short_id(&item.id),
            item.question
        );
        let raw = json!({ "question": item });
        Ok(
            ToolExecution::new("ask_user_question", content, raw.clone(), decision)
                .with_structured("question", "user question queued", raw, false),
        )
    }

    async fn web_search(&self, args: Value) -> Result<ToolExecution> {
        let query = required_str(&args, "query")?;
        if looks_sensitive(query) {
            bail!("web_search query appears to contain sensitive content");
        }
        let decision = self.evaluate_declared_tool(
            "web_search",
            ToolPermissionContext {
                network_target: Some("api.duckduckgo.com".to_string()),
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("web_search", &decision, &args)?;
        let url = reqwest::Url::parse_with_params(
            "https://api.duckduckgo.com/",
            &[("q", query), ("format", "json"), ("no_html", "1")],
        )?;
        let (_, response) =
            safe_web_get(url, Duration::from_secs(20), "deepcli-web-search/0.1").await?;
        if !response.status().is_success() {
            bail!("web_search returned HTTP {}", response.status().as_u16());
        }
        let body = read_bounded_response_body(response, 100_000).await?;
        if body.truncated {
            bail!("web_search response exceeded the host response limit");
        }
        let value: Value = serde_json::from_str(&body.text).context("invalid web_search JSON")?;
        let content = format_web_search_result(query, &value);
        let raw = value;
        Ok(
            ToolExecution::new("web_search", content.clone(), raw.clone(), decision)
                .with_structured("web_search", first_line(&content), raw, false),
        )
    }

    async fn web_fetch(&self, args: Value) -> Result<ToolExecution> {
        let raw_url = required_str(&args, "url")?.trim();
        if looks_sensitive(raw_url) {
            bail!("web_fetch URL appears to contain sensitive content");
        }
        let url = reqwest::Url::parse(raw_url)?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("web_fetch only supports http or https URLs");
        }
        let host = url
            .host_str()
            .ok_or_else(|| anyhow!("web_fetch URL must include a host"))?
            .to_string();
        let decision = self.evaluate_declared_tool(
            "web_fetch",
            ToolPermissionContext {
                network_target: Some(host),
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("web_fetch", &decision, &args)?;
        let max_chars = bounded_web_fetch_chars(args.get("max_chars").and_then(Value::as_u64));
        let (url, response) =
            safe_web_get(url, Duration::from_secs(20), "deepcli-web-fetch/0.1").await?;
        let status = response.status().as_u16();
        let success = (200..400).contains(&status);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = read_bounded_response_body(response, max_chars).await?;
        let extracted = format_web_fetch_text(&body.text, &content_type);
        let observed_chars = extracted.chars().count();
        let truncated = body.truncated || observed_chars > max_chars;
        let text = if observed_chars > max_chars {
            truncate_display(&extracted, max_chars)
        } else {
            extracted
        };
        let content = format!(
            "url: {}\nstatus: {}\ncontent_type: {}\n\n{}",
            url, status, content_type, text
        );
        let raw = json!({
            "url": url.as_str(),
            "status": status,
            "content_type": content_type,
            "text": text,
            "truncated": truncated,
            "body_truncated": body.truncated,
            "downloaded_bytes": body.downloaded_bytes,
            "observed_chars": observed_chars,
            "original_chars": if body.truncated { Value::Null } else { json!(observed_chars) }
        });
        Ok(
            ToolExecution::new("web_fetch", content, raw.clone(), decision)
                .with_structured(
                    "web_fetch",
                    format!("fetched {} status {}", url, status),
                    raw,
                    truncated,
                )
                .with_success(success),
        )
    }

    async fn open_terminal(&self) -> Result<ToolExecution> {
        self.open_terminal_now()
    }

    fn open_terminal_now(&self) -> Result<ToolExecution> {
        self.open_terminal_app_now("Terminal")
    }

    fn open_terminal_app_now(&self, app: &str) -> Result<ToolExecution> {
        let command = terminal_open_command(app);
        let decision = self.evaluate_declared_tool(
            "open_terminal",
            ToolPermissionContext {
                command: Some(command.clone()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("open_terminal", &decision, &json!({ "app": app }))?;
        #[cfg(target_os = "macos")]
        let output = run_command_blocking(&self.workspace, &command)?;
        #[cfg(not(target_os = "macos"))]
        let output = CommandOutput {
            command,
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "open_terminal is only implemented for macOS".to_string(),
        };
        let content = output_text(&output);
        let success = output.exit_code == Some(0);
        let raw = json!(output);
        Ok(
            ToolExecution::new("open_terminal", content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false)
                .with_success(success),
        )
    }

    async fn prompt_list(&self) -> Result<ToolExecution> {
        let decision = self.evaluate_filesystem(
            "prompt_list",
            &self.workspace.join(".deepcli/prompts"),
            false,
        )?;
        self.ensure_allowed("prompt_list", &decision, &json!({}))?;
        let store = PromptStore::new(&self.workspace);
        let prompts = store.list()?;
        let content = format_prompt_tool_list(&prompts);
        let raw = json!(prompts);
        Ok(
            ToolExecution::new("prompt_list", content.clone(), raw.clone(), decision)
                .with_structured("prompt_list", first_line(&content), raw, false),
        )
    }

    async fn prompt_get(&self, args: Value) -> Result<ToolExecution> {
        let name = required_str(&args, "name")?;
        let decision = self.evaluate_filesystem(
            "prompt_get",
            &self.workspace.join(".deepcli/prompts"),
            false,
        )?;
        self.ensure_allowed("prompt_get", &decision, &args)?;
        let store = PromptStore::new(&self.workspace);
        let prompt = store.get(name)?;
        let raw = json!(prompt);
        Ok(ToolExecution::new(
            "prompt_get",
            raw["body"].as_str().unwrap_or_default().to_string(),
            raw.clone(),
            decision,
        )
        .with_structured("prompt", name, raw, false))
    }

    async fn prompt_render(&self, args: Value) -> Result<ToolExecution> {
        let name = required_str(&args, "name")?;
        let command = "git branch --show-current && git diff";
        let decision = self.evaluate_declared_tool(
            "prompt_render",
            ToolPermissionContext {
                command: Some(command.to_string()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("prompt_render", &decision, &args)?;
        let store = PromptStore::new(&self.workspace);
        let prompt = store.get(name)?;
        let context = self.prompt_render_context(&args).await?;
        let rendered = render_prompt_body(&prompt.body, &context);
        let raw = json!({
            "name": prompt.name,
            "description": prompt.description,
            "context": context,
            "rendered": rendered
        });
        Ok(ToolExecution::new(
            "prompt_render",
            raw["rendered"].as_str().unwrap_or_default().to_string(),
            raw.clone(),
            decision,
        )
        .with_structured("prompt_render", name, raw, false))
    }

    async fn skill_list(&self) -> Result<ToolExecution> {
        let decision =
            self.evaluate_filesystem("skill_list", &self.workspace.join(".deepcli/skills"), false)?;
        self.ensure_allowed("skill_list", &decision, &json!({}))?;
        let store = SkillStore::new(&self.workspace);
        let skills = store.discover()?;
        let content = format_skill_tool_list(&skills);
        let raw = json!(skills);
        Ok(
            ToolExecution::new("skill_list", content.clone(), raw.clone(), decision)
                .with_structured("skill_list", first_line(&content), raw, false),
        )
    }

    async fn prompt_render_context(&self, args: &Value) -> Result<PromptRenderContext> {
        let max_diff_chars = args
            .get("max_diff_chars")
            .and_then(Value::as_u64)
            .unwrap_or(12_000) as usize;
        let max_file_chars = args
            .get("max_file_chars")
            .and_then(Value::as_u64)
            .unwrap_or(12_000) as usize;
        let branch = command_stdout_or_empty(&self.workspace, "git branch --show-current")
            .await?
            .trim()
            .to_string();
        let diff = match self.safe_git_diff_paths(false).await {
            Ok(diff_paths) => {
                let diff_output = self.run_safe_git_diff(false, &diff_paths, false).await?;
                truncate_display(
                    if diff_output.exit_code == Some(0) {
                        diff_output.stdout.trim()
                    } else {
                        ""
                    },
                    max_diff_chars,
                )
            }
            Err(_) => String::new(),
        };

        let (file, file_content) = if let Some(raw_file) = args.get("file").and_then(Value::as_str)
        {
            let file_path = self.resolve_provider_path(raw_file)?;
            let file_decision = self.evaluate_filesystem("prompt_render", &file_path, false)?;
            self.ensure_allowed("prompt_render", &file_decision, args)?;
            let relative = file_path
                .strip_prefix(&self.workspace)
                .unwrap_or(&file_path)
                .display()
                .to_string();
            let content = fs::read_to_string(&file_path)
                .with_context(|| format!("failed to read {}", file_path.display()))?;
            (relative, truncate_display(&content, max_file_chars))
        } else {
            (String::new(), String::new())
        };

        Ok(PromptRenderContext {
            workspace: self.workspace.display().to_string(),
            cwd: self.workspace.display().to_string(),
            branch,
            diff,
            file,
            file_content,
            variables: prompt_render_variables(args)?,
        })
    }

    async fn skill_generate(&self, args: Value) -> Result<ToolExecution> {
        let name = required_str(&args, "name")?;
        let description = required_str(&args, "description")?;
        let decision = self.evaluate_filesystem(
            "skill_generate",
            &self.workspace.join(".deepcli/skills").join(name),
            true,
        )?;
        self.ensure_allowed("skill_generate", &decision, &args)?;
        let store = SkillStore::new(&self.workspace);
        let skill = store.generate(name, description)?;
        let content = skill.instruction_path.display().to_string();
        let raw = json!(skill);
        Ok(
            ToolExecution::new("skill_generate", content.clone(), raw.clone(), decision)
                .with_structured("skill", content, raw, false),
        )
    }

    async fn skill_run(&self, args: Value) -> Result<ToolExecution> {
        let name = required_str(&args, "name")?;
        let decision =
            self.evaluate_filesystem("skill_run", &self.workspace.join(".deepcli/skills"), false)?;
        self.ensure_allowed("skill_run", &decision, &args)?;
        let store = SkillStore::new(&self.workspace);
        let loaded = store.load(name)?;
        let content = loaded.instructions.clone();
        let raw = json!(loaded);
        Ok(
            ToolExecution::new("skill_run", content, raw.clone(), decision)
                .with_structured("skill", name, raw, false),
        )
    }

    async fn spawn_subagent(&self, args: Value) -> Result<ToolExecution> {
        let depth = self
            .current_subagent_depth
            .checked_add(1)
            .ok_or_else(|| anyhow!("sub-agent depth exceeds the supported range"))?;
        let task = required_str(&args, "task")?;
        let write_scope = string_array_arg(&args, "write_scope");
        let read_scope = string_array_arg(&args, "read_scope");
        let mut allowed_tools = string_array_arg(&args, "allowed_tools")
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let context = args
            .get("context")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        validate_allowed_subagent_tools(&allowed_tools)?;
        if let Some(parent_allowed) = self
            .subagent_capability
            .as_ref()
            .and_then(|capability| capability.allowed_tools.as_ref())
        {
            if allowed_tools.is_empty() {
                allowed_tools = parent_allowed.iter().cloned().collect();
            } else if let Some(tool) = allowed_tools
                .iter()
                .find(|tool| !parent_allowed.contains(*tool))
            {
                bail!("nested sub-agent cannot widen parent tool capability with `{tool}`");
            }
        }
        let decision = self.evaluate_declared_tool(
            "spawn_subagent",
            ToolPermissionContext {
                path: Some(self.workspace.clone()),
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("spawn_subagent", &decision, &args)?;
        if depth > self.max_subagent_depth {
            bail!(
                "sub-agent depth {depth} exceeds configured maxSubagentDepth {}",
                self.max_subagent_depth
            );
        }
        let store = AgentStore::new(&self.workspace);
        let parent_session_id = self.session.as_ref().map(|session| session.id());
        let subagent =
            store.create_subagent_task_with_options(crate::agents::SubagentTaskOptions {
                parent_session_id,
                task: task.to_string(),
                depth,
                write_scope,
                read_scope,
                allowed_tools,
                context,
            })?;
        let background_start = start_subagent_background(&store, &self.workspace, subagent.id);
        let subagent = store.load(subagent.id)?;
        let next_actions = subagent_next_actions(&subagent);
        let content = format!(
            "sub-agent {} at depth {depth}: {task} ({})",
            subagent_status_text(&subagent.status),
            subagent.id
        );
        let mut raw = serde_json::to_value(&subagent)?;
        if let Value::Object(map) = &mut raw {
            map.insert("background_start".to_string(), json!(background_start));
            map.insert("next_actions".to_string(), json!(next_actions));
        }
        Ok(
            ToolExecution::new("spawn_subagent", content.clone(), raw.clone(), decision)
                .with_structured("subagent_task", content, raw, false),
        )
    }

    fn evaluate_declared_tool(
        &self,
        tool: &str,
        context: ToolPermissionContext,
    ) -> Result<PermissionDecision> {
        let registry = ToolRegistry::mvp();
        let declaration = registry
            .declaration(tool)
            .ok_or_else(|| anyhow!("unknown tool declaration `{tool}`"))?;
        Ok(self
            .permissions
            .evaluate(&declaration.permission_request(context)))
    }

    fn evaluate_filesystem(
        &self,
        tool: &str,
        path: &Path,
        writes_files: bool,
    ) -> Result<PermissionDecision> {
        self.ensure_subagent_path_allowed(path, writes_files)?;
        Ok(self.permissions.evaluate(&ToolRequest {
            tool: tool.to_string(),
            surface: ToolSurface::Filesystem,
            command: None,
            path: Some(path.to_path_buf()),
            network_target: None,
            writes_files,
            creates_process: false,
            requires_network: false,
        }))
    }

    fn ensure_subagent_tool_allowed(&self, tool: &str) -> Result<()> {
        let Some(capability) = &self.subagent_capability else {
            return Ok(());
        };
        if capability
            .allowed_tools
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(tool))
        {
            bail!("sub-agent capability does not allow tool `{tool}`");
        }
        if capability.has_path_restrictions()
            && !matches!(
                tool,
                "read_file"
                    | "list_files"
                    | "search"
                    | "write_file"
                    | "apply_patch_or_write"
                    | "todo_write"
                    | "ask_user_question"
                    | "web_search"
                    | "web_fetch"
            )
        {
            bail!("sub-agent capability cannot safely enforce path scopes for tool `{tool}`");
        }
        Ok(())
    }

    fn ensure_subagent_path_allowed(&self, path: &Path, writes_files: bool) -> Result<()> {
        let Some(capability) = &self.subagent_capability else {
            return Ok(());
        };
        let scopes = if writes_files {
            capability.write_scope.as_ref()
        } else {
            capability.read_scope.as_ref()
        };
        let Some(scopes) = scopes else {
            return Ok(());
        };
        if scopes
            .iter()
            .any(|scope| path == scope || path.starts_with(scope))
        {
            return Ok(());
        }
        let access = if writes_files { "write" } else { "read" };
        bail!(
            "sub-agent {access} capability does not include path {}",
            relative_path_string(&self.workspace, path)
        )
    }

    fn ensure_allowed(
        &self,
        tool: &str,
        decision: &PermissionDecision,
        args: &Value,
    ) -> Result<()> {
        if matches!(
            decision.outcome,
            DecisionOutcome::RequiresUserApproval | DecisionOutcome::DoubleConfirmRequired
        ) {
            let digest = invocation_digest(tool, args);
            if let Some(session) = &self.session {
                let confirmations_required =
                    if decision.outcome == DecisionOutcome::DoubleConfirmRequired {
                        2
                    } else {
                        1
                    };
                if session
                    .consume_approval_grant(tool, &digest, decision, confirmations_required)?
                    .is_some()
                {
                    return Ok(());
                }
            }
        }
        match decision.outcome {
            DecisionOutcome::Allowed | DecisionOutcome::AutoApproved => Ok(()),
            DecisionOutcome::RequiresUserApproval if self.can_assume_yes(tool, decision) => Ok(()),
            DecisionOutcome::Denied => bail!("permission denied: {}", decision.reason),
            DecisionOutcome::RequiresUserApproval => {
                let approval = self.enqueue_approval_request(tool, args, decision)?;
                bail!(
                    "operation requires approval: {} (approval {})",
                    decision.reason,
                    short_id(&approval.id)
                )
            }
            DecisionOutcome::DoubleConfirmRequired => {
                let approval = self.enqueue_approval_request(tool, args, decision)?;
                bail!(
                    "operation requires double confirmation: {} (approval {})",
                    decision.reason,
                    short_id(&approval.id)
                )
            }
        }
    }

    fn enqueue_approval_request(
        &self,
        tool: &str,
        args: &Value,
        decision: &PermissionDecision,
    ) -> Result<crate::session::ApprovalRequest> {
        let digest = invocation_digest(tool, args);
        let summary = invocation_summary(tool, args);
        let confirmations_required = if decision.outcome == DecisionOutcome::DoubleConfirmRequired {
            2
        } else {
            1
        };
        if let Some(session) = &self.session {
            session.enqueue_bound_approval_request(
                tool,
                decision.clone(),
                digest,
                summary,
                confirmations_required,
            )
        } else {
            Ok(crate::session::ApprovalRequest {
                id: uuid::Uuid::nil(),
                tool: tool.to_string(),
                decision: decision.clone(),
                status: crate::session::ApprovalStatus::Pending,
                invocation_digest: Some(digest),
                input_summary: Some(summary),
                confirmations_required,
                confirmations_received: 0,
                approved_at: None,
                consumed_at: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        }
    }

    fn can_assume_yes(&self, _tool: &str, _decision: &PermissionDecision) -> bool {
        self.assume_yes
    }

    fn resolve_required_path(&self, args: &Value) -> Result<PathBuf> {
        let raw = required_str(args, "path")?;
        self.resolve_provider_path(raw)
    }

    fn resolve_optional_path(&self, args: &Value) -> Result<Option<PathBuf>> {
        args.get("path")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(|raw| self.resolve_provider_path(raw))
            .transpose()
    }

    fn resolve_provider_path(&self, raw: &str) -> Result<PathBuf> {
        let path = resolve_workspace_path(&self.workspace, raw)?;
        let ignore = DeepIgnore::load(&self.workspace)?;
        if ignore.is_ignored(&path) {
            bail!("path is ignored by workspace policy: {}", path.display());
        }
        Ok(path)
    }
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("missing required string argument `{key}`"))
}

fn resolve_capability_scope(
    workspace: &Path,
    raw_scopes: &[PathBuf],
) -> Result<Option<Vec<PathBuf>>> {
    if raw_scopes.is_empty() {
        return Ok(None);
    }
    let scopes = raw_scopes
        .iter()
        .map(|scope| {
            let raw = scope.to_str().ok_or_else(|| {
                anyhow!("sub-agent scope is not valid UTF-8: {}", scope.display())
            })?;
            resolve_workspace_path(workspace, raw)
        })
        .collect::<Result<Vec<_>>>()?;
    if scopes.iter().any(|scope| scope == workspace) {
        Ok(None)
    } else {
        Ok(Some(scopes))
    }
}

fn parse_git_oid(value: &str, label: &str) -> Result<String> {
    let oid = value.trim();
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("git returned an invalid {label} object id");
    }
    Ok(oid.to_ascii_lowercase())
}

fn bool_arg(args: &Value, key: &str, default: bool) -> bool {
    match args.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "y" | "on"
        ),
        _ => default,
    }
}

fn bounded_timeout_seconds(args: &Value, maximum: u64) -> u64 {
    args.get("timeout_seconds")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .map(|value| value.min(maximum))
        .unwrap_or(maximum)
}

fn authorization_args_with_resolved_path(args: &Value, path: &Path) -> Value {
    let mut authorization_args = args.clone();
    if let Value::Object(arguments) = &mut authorization_args {
        arguments.insert(
            "path".to_string(),
            Value::String(path.display().to_string()),
        );
        if let Some(content) = args.get("content").and_then(Value::as_str) {
            arguments.insert("_content_bytes".to_string(), json!(content.len()));
            arguments.insert("_content_lines".to_string(), json!(content.lines().count()));
        }
    }
    authorization_args
}

fn optional_string_array(args: &Value, key: &str) -> Result<Vec<String>> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        anyhow::bail!("argument `{key}` must be an array of strings");
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("argument `{key}` must be an array of strings"))
        })
        .collect()
}

fn optional_glob(args: &Value) -> Result<Option<GlobSet>> {
    let Some(pattern) = args
        .get("glob")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let mut builder = globset::GlobSetBuilder::new();
    builder.add(Glob::new(pattern)?);
    Ok(Some(builder.build()?))
}

fn path_matches_glob(path: &Path, workspace: &Path, glob: Option<&GlobSet>) -> bool {
    let Some(glob) = glob else {
        return true;
    };
    let relative = path.strip_prefix(workspace).unwrap_or(path);
    glob.is_match(relative)
}

fn relative_path_string(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .trim_start_matches("./")
        .to_string()
}

fn format_search_matches(matches: &[Value]) -> Result<String> {
    if matches.is_empty() {
        return Ok("no matches".to_string());
    }
    Ok(matches
        .iter()
        .map(|item| {
            let path = item.get("path").and_then(Value::as_str).unwrap_or_default();
            let line = item.get("line").and_then(Value::as_u64).unwrap_or_default();
            let text = item.get("text").and_then(Value::as_str).unwrap_or_default();
            format!("{path}:{line}: {text}")
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn todo_steps_from_args(args: &Value) -> Result<Vec<PlanStep>> {
    let todos = args
        .get("todos")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("argument `todos` must be an array"))?;
    let mut seen = BTreeSet::new();
    todos
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let content = item
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("todo item {} requires `content`", index + 1))?
                .trim()
                .to_string();
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("todo-{}", index + 1));
            if !seen.insert(id.clone()) {
                bail!("duplicate todo id `{id}`");
            }
            let status = match item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending")
            {
                "pending" => PlanStepStatus::Pending,
                "in_progress" => PlanStepStatus::InProgress,
                "completed" => PlanStepStatus::Completed,
                "failed" => PlanStepStatus::Failed,
                other => bail!("unsupported todo status `{other}`"),
            };
            Ok(PlanStep {
                id,
                description: content,
                status,
            })
        })
        .collect()
}

fn plan_step_json(step: &PlanStep) -> Value {
    json!({
        "id": step.id,
        "content": step.description,
        "status": step.status,
    })
}

fn format_todo_steps(steps: &[PlanStep]) -> String {
    if steps.is_empty() {
        return "todo list cleared".to_string();
    }
    steps
        .iter()
        .map(|step| {
            format!(
                "- [{}] {}: {}",
                plan_status_label(&step.status),
                step.id,
                step.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn plan_status_label(status: &PlanStepStatus) -> &'static str {
    match status {
        PlanStepStatus::Pending => "pending",
        PlanStepStatus::InProgress => "in_progress",
        PlanStepStatus::Completed => "completed",
        PlanStepStatus::Failed => "failed",
    }
}

fn string_array_arg(args: &Value, key: &str) -> Vec<PathBuf> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn start_subagent_background(
    store: &AgentStore,
    workspace: &Path,
    id: uuid::Uuid,
) -> SubagentBackgroundStart {
    #[cfg(test)]
    {
        let _ = workspace;
        subagent_background_scheduled(store, id, "background start disabled in tests".to_string())
    }

    #[cfg(not(test))]
    {
        let exe = match std::env::current_exe() {
            Ok(path) => path,
            Err(error) => {
                return subagent_background_scheduled(
                    store,
                    id,
                    format!("failed to resolve current executable: {error}"),
                );
            }
        };
        let output_log = store.subagent_output_log_path(id);
        if let Some(parent) = output_log.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                return subagent_background_scheduled(
                    store,
                    id,
                    format!("failed to create {}: {error}", parent.display()),
                );
            }
        }
        let stdout = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&output_log)
        {
            Ok(file) => file,
            Err(error) => {
                return subagent_background_scheduled(
                    store,
                    id,
                    format!("failed to open {}: {error}", output_log.display()),
                );
            }
        };
        let stderr = match stdout.try_clone() {
            Ok(file) => file,
            Err(error) => {
                return subagent_background_scheduled(
                    store,
                    id,
                    format!("failed to clone {}: {error}", output_log.display()),
                );
            }
        };
        let command = vec![
            exe.display().to_string(),
            "-C".to_string(),
            workspace.display().to_string(),
            "--yes".to_string(),
            "agent".to_string(),
            "resume".to_string(),
            id.to_string(),
            "--background-child".to_string(),
            "--json".to_string(),
        ];
        let mut process = std::process::Command::new(&exe);
        process
            .arg("-C")
            .arg(workspace)
            .arg("--yes")
            .arg("agent")
            .arg("resume")
            .arg(id.to_string())
            .arg("--background-child")
            .arg("--json")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(stdout))
            .stderr(std::process::Stdio::from(stderr));

        match process.spawn() {
            Ok(child) => {
                let pid = child.id();
                if let Err(error) = store.record_subagent_background_process(id, Some(pid)) {
                    return subagent_background_scheduled(
                        store,
                        id,
                        format!("background process spawned but lifecycle update failed: {error}"),
                    );
                }
                SubagentBackgroundStart {
                    status: "started".to_string(),
                    pid: Some(pid),
                    reason: None,
                    command,
                }
            }
            Err(error) => subagent_background_scheduled(
                store,
                id,
                format!("failed to start background process: {error}"),
            ),
        }
    }
}

fn subagent_background_scheduled(
    store: &AgentStore,
    id: uuid::Uuid,
    reason: String,
) -> SubagentBackgroundStart {
    let _ = store.record_subagent_scheduled(id, &reason);
    SubagentBackgroundStart {
        status: "scheduled".to_string(),
        pid: None,
        reason: Some(reason),
        command: Vec::new(),
    }
}

fn subagent_next_actions(task: &SubagentTask) -> Vec<String> {
    let id = task.id.to_string();
    let mut actions = vec![
        format!("deepcli agent show {id}"),
        format!("deepcli agent logs {id} --json"),
    ];
    match task.status {
        SubagentStatus::Queued | SubagentStatus::Failed => {
            actions.push(format!("deepcli agent resume {id} --json"));
        }
        SubagentStatus::AwaitingApproval => {
            if let Some(session_id) = task.child_session_id {
                actions.push(format!(
                    "deepcli approval list --session {session_id} --json"
                ));
            }
            actions.push(format!("deepcli agent resume {id} --json"));
        }
        SubagentStatus::Running => {
            actions.push(format!("deepcli agent resume {id} --json"));
        }
        SubagentStatus::Completed => {}
    }
    actions
}

fn subagent_status_text(status: &SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::Queued => "queued",
        SubagentStatus::Running => "running",
        SubagentStatus::AwaitingApproval => "awaiting_approval",
        SubagentStatus::Completed => "completed",
        SubagentStatus::Failed => "failed",
    }
}

fn validate_allowed_subagent_tools(allowed_tools: &[String]) -> Result<()> {
    if allowed_tools.is_empty() {
        return Ok(());
    }
    let registry = ToolRegistry::mvp();
    for tool in allowed_tools {
        if !registry.has(tool) {
            bail!("sub-agent allowed_tools contains unknown tool `{tool}`");
        }
    }
    Ok(())
}

fn truncate_display(value: &str, limit: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= limit {
        return value.to_string();
    }
    let mut truncated = value.chars().take(limit).collect::<String>();
    truncated.push_str(&format!("...[truncated {char_count} chars]"));
    truncated
}

fn prompt_render_variables(args: &Value) -> Result<BTreeMap<String, String>> {
    let mut variables = BTreeMap::new();
    if let Some(object) = args.get("variables").and_then(Value::as_object) {
        for (key, value) in object {
            if !is_valid_prompt_variable_name(key) {
                bail!("invalid prompt variable name `{key}`");
            }
            let rendered = match value {
                Value::String(value) => value.clone(),
                other => other.to_string(),
            };
            variables.insert(key.clone(), rendered);
        }
    }
    Ok(variables)
}

fn is_valid_prompt_variable_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn format_prompt_tool_list(prompts: &[Prompt]) -> String {
    if prompts.is_empty() {
        return "no prompts available".to_string();
    }
    prompts
        .iter()
        .map(|prompt| format!("{} - {}", prompt.name, prompt.description))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_skill_tool_list(skills: &[SkillMetadata]) -> String {
    if skills.is_empty() {
        return "no project skills registered; use skill_generate to create one".to_string();
    }
    skills
        .iter()
        .map(|skill| {
            format!(
                "{} - {} (trigger: {})",
                skill.name, skill.description, skill.trigger
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_line(value: &str) -> &str {
    value.lines().next().unwrap_or_default().trim()
}

fn approval_status_for(outcome: DecisionOutcome) -> Option<ToolCallStatus> {
    match outcome {
        DecisionOutcome::AutoApproved => Some(ToolCallStatus::AutoApproved),
        DecisionOutcome::RequiresUserApproval | DecisionOutcome::DoubleConfirmRequired => {
            Some(ToolCallStatus::UserApproved)
        }
        DecisionOutcome::Denied => Some(ToolCallStatus::Denied),
        DecisionOutcome::Allowed => None,
    }
}

fn short_id(id: &uuid::Uuid) -> String {
    id.to_string()[..8].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PermissionConfig, SandboxConfig};
    use tempfile::tempdir;

    #[test]
    fn provider_tool_schemas_do_not_expose_authorization_controls() {
        let registry = ToolRegistry::mvp();
        for spec in registry.tool_specs() {
            let properties = spec.function.parameters["properties"]
                .as_object()
                .expect("tool properties");
            for untrusted in ["approved", "writes_files", "requires_network"] {
                assert!(
                    !properties.contains_key(untrusted),
                    "{} must not expose host-owned `{untrusted}`",
                    spec.function.name
                );
            }
        }
    }

    #[test]
    fn tool_validation_rejects_model_supplied_authorization_controls() {
        let registry = ToolRegistry::mvp();
        for (tool, args, field) in [
            (
                "write_file",
                json!({"path": "note.txt", "content": "x", "approved": true}),
                "approved",
            ),
            (
                "run_shell",
                json!({"command": "pwd", "writes_files": false}),
                "writes_files",
            ),
            (
                "run_shell",
                json!({"command": "pwd", "requires_network": false}),
                "requires_network",
            ),
        ] {
            let error = registry
                .tool(tool)
                .expect("registered tool")
                .validate_arguments(&args)
                .expect_err("host-owned fields must be rejected");
            assert!(error.to_string().contains(field));
        }
    }

    #[test]
    fn tool_prompt_content_uses_the_execution_outcome() {
        let decision = PermissionDecision {
            outcome: DecisionOutcome::Allowed,
            risk: RiskLevel::Low,
            reason: "test".to_string(),
        };
        let execution = ToolExecution::new(
            "run_tests",
            "tests failed",
            json!({"passed": false}),
            decision,
        )
        .with_success(false);

        let prompt: Value = serde_json::from_str(&execution.prompt_content()).unwrap();
        assert_eq!(prompt["ok"], false);
    }

    #[test]
    fn discovers_common_test_commands() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'").unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"vitest"}}"#,
        )
        .unwrap();
        let commands = discover_tests_in(dir.path()).unwrap();
        let command_text = commands
            .into_iter()
            .map(|command| command.command)
            .collect::<Vec<_>>();
        assert!(command_text.contains(&"cargo test".to_string()));
        assert!(command_text.contains(&"npm test".to_string()));
    }

    #[test]
    fn discovers_compiler_docker_autotest_commands() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("online-doc/docs/lv1-main")).unwrap();
        fs::create_dir_all(dir.path().join("online-doc/docs/lv9-array")).unwrap();
        fs::write(
            dir.path().join("online-doc/docs/lv1-main/testing.md"),
            "lv1 tests",
        )
        .unwrap();
        fs::write(
            dir.path().join("online-doc/docs/lv9-array/testing.md"),
            "lv9 tests",
        )
        .unwrap();

        let docker_ok = docker_available();
        let commands = discover_tests_in(dir.path()).unwrap();
        let docker_commands = commands
            .iter()
            .filter(|command| command.requires_docker)
            .collect::<Vec<_>>();

        assert_eq!(docker_commands.len(), 3);
        assert!(docker_commands.iter().all(|command| {
            command.available == Some(docker_ok)
                && command.command.contains("docker run --rm")
                && command.command.contains("maxxing/compiler-dev")
        }));
        assert!(docker_commands
            .iter()
            .any(|command| command.command.contains("autotest -koopa -s lv1")));
        assert!(docker_commands
            .iter()
            .any(|command| command.command.contains("autotest -koopa /root/compiler")));
        assert!(docker_commands
            .iter()
            .any(|command| command.command.contains("autotest -riscv /root/compiler")));
        assert!(format_discovered_test_command(docker_commands[0]).contains("requires docker"));
    }

    #[test]
    fn environment_ready_requires_target_specific_checks() {
        let checks = vec![
            EnvironmentCheck {
                name: "docker_cli".to_string(),
                available: true,
                version: Some("Docker version test".to_string()),
                detail: None,
            },
            EnvironmentCheck {
                name: "colima".to_string(),
                available: true,
                version: Some("colima version test".to_string()),
                detail: None,
            },
            EnvironmentCheck {
                name: "docker_daemon".to_string(),
                available: true,
                version: Some("29.0.0".to_string()),
                detail: None,
            },
            EnvironmentCheck {
                name: "compiler_dev_image".to_string(),
                available: false,
                version: None,
                detail: Some("missing".to_string()),
            },
        ];
        assert!(environment_ready("docker", &checks));
        assert!(!environment_ready("compiler", &checks));
        let report = EnvironmentReport {
            target: "compiler".to_string(),
            ready: false,
            checks,
            recommended_action: Some("/install compiler --smoke".to_string()),
        };
        let text = format_environment_report(&report);
        assert!(text.contains("compiler_dev_image: missing"));
        assert!(text.contains("recommended: /install compiler --smoke"));
        assert!(text.contains("next:"));
        assert!(text.contains("run `/install compiler --smoke` to continue environment setup"));
        assert!(text.contains("re-check environment with `/doctor compiler --json`"));
        assert!(compiler_image_pull_command().contains("docker.1ms.run/maxxing/compiler-dev"));
        assert!(compiler_image_pull_command().contains("docker.m.daocloud.io/maxxing/compiler-dev"));
    }

    #[tokio::test]
    async fn read_and_write_file_respect_workspace() {
        let dir = tempdir().unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        )
        .with_auto_reviewer(true);
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);
        executor
            .execute(
                "write_file",
                json!({"path": "src/lib.rs", "content": "pub fn ok() {}\n"}),
            )
            .await
            .unwrap();
        let read = executor
            .execute("read_file", json!({"path": "src/lib.rs"}))
            .await
            .unwrap();
        assert!(read.content.contains("pub fn ok"));
        fs::write(dir.path().join("src/lib.rs"), "a\nb\nc\nd\n").unwrap();
        let sliced = executor
            .execute(
                "read_file",
                json!({"path": "src/lib.rs", "start_line": 2, "limit": 2}),
            )
            .await
            .unwrap();
        assert!(sliced.content.contains("lines 2-3 of 4"));
        assert!(sliced.content.ends_with("b\nc"));
    }

    #[tokio::test]
    async fn apply_patch_tool_accepts_unified_diff() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("note.txt"), "old\n").unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);
        executor
            .execute(
                "apply_patch_or_write",
                json!({
                    "patch": "--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-old\n+new\n"
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "new\n"
        );
    }

    #[tokio::test]
    async fn apply_patch_rejects_custom_ignored_target() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".deepignore"), "secret.txt\n").unwrap();
        fs::write(dir.path().join("secret.txt"), "old\n").unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);

        let error = executor
            .execute(
                "apply_patch_or_write",
                json!({
                    "patch": "--- a/secret.txt\n+++ b/secret.txt\n@@ -1 +1 @@\n-old\n+new\n"
                }),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("ignored by workspace policy"));
        assert_eq!(
            fs::read_to_string(dir.path().join("secret.txt")).unwrap(),
            "old\n"
        );
    }

    #[tokio::test]
    async fn git_diff_excludes_default_and_custom_ignored_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".deepignore"), "secret.txt\n").unwrap();
        fs::write(dir.path().join("safe.txt"), "safe old\n").unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=old\n").unwrap();
        fs::write(dir.path().join("secret.txt"), "secret old\n").unwrap();
        let init = run_command_blocking(
            dir.path(),
            "git init -q && git add -f . && git -c user.name=test -c user.email=test@example.com commit -qm init",
        )
        .unwrap();
        assert_eq!(init.exit_code, Some(0), "{}", init.stderr);
        fs::write(dir.path().join("safe.txt"), "safe new\n").unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=new\n").unwrap();
        fs::write(dir.path().join("secret.txt"), "secret new\n").unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let execution = executor.execute("git_diff", json!({})).await.unwrap();

        assert!(execution.success);
        assert!(execution.content.contains("safe new"));
        assert!(!execution.content.contains("TOKEN="));
        assert!(!execution.content.contains("secret new"));
        assert!(!execution.content.contains(".env"));
        assert!(!execution.content.contains("secret.txt"));
    }

    #[tokio::test]
    async fn git_commit_approval_is_bound_to_the_staged_snapshot() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("safe.txt"), "old\n").unwrap();
        let init = run_command_blocking(
            dir.path(),
            "git init -q && git add safe.txt && git -c user.name=test -c user.email=test@example.com commit -qm init",
        )
        .unwrap();
        assert_eq!(init.exit_code, Some(0), "{}", init.stderr);
        fs::write(dir.path().join("safe.txt"), "first\n").unwrap();
        assert_eq!(
            run_command_blocking(dir.path(), "git add safe.txt")
                .unwrap()
                .exit_code,
            Some(0)
        );
        let session = crate::session::SessionStore::new(dir.path())
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);
        let args = json!({"message": "update safe file"});

        executor
            .execute("git_commit", args.clone())
            .await
            .unwrap_err();
        let first = session.load_approval_requests().unwrap().remove(0);
        session
            .approve_approval_request(&first.id.to_string())
            .unwrap();

        fs::write(dir.path().join("safe.txt"), "second\n").unwrap();
        assert_eq!(
            run_command_blocking(dir.path(), "git add safe.txt")
                .unwrap()
                .exit_code,
            Some(0)
        );
        let error = executor
            .execute("git_commit", args)
            .await
            .expect_err("changed staged content must require a new approval");

        assert!(error.to_string().contains("operation requires approval"));
        let approvals = session.load_approval_requests().unwrap();
        assert_eq!(approvals.len(), 2);
        assert_eq!(
            approvals[0].status,
            crate::session::ApprovalStatus::Approved
        );
        assert_eq!(approvals[1].status, crate::session::ApprovalStatus::Pending);
        assert_ne!(
            approvals[0].invocation_digest,
            approvals[1].invocation_digest
        );
    }

    #[tokio::test]
    async fn git_commit_refuses_staged_paths_blocked_by_workspace_policy() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".deepignore"), "secret.txt\n").unwrap();
        fs::write(dir.path().join("secret.txt"), "old\n").unwrap();
        let init = run_command_blocking(
            dir.path(),
            "git init -q && git add -f secret.txt && git -c user.name=test -c user.email=test@example.com commit -qm init",
        )
        .unwrap();
        assert_eq!(init.exit_code, Some(0), "{}", init.stderr);
        fs::write(dir.path().join("secret.txt"), "new secret\n").unwrap();
        assert_eq!(
            run_command_blocking(dir.path(), "git add -f secret.txt")
                .unwrap()
                .exit_code,
            Some(0)
        );
        let session = crate::session::SessionStore::new(dir.path())
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);

        let error = executor
            .execute("git_commit", json!({"message": "must not commit secret"}))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("blocked by workspace policy"));
        assert!(session.load_approval_requests().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn git_commit_uses_the_approved_tree_without_running_repository_hooks() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        fs::write(dir.path().join("safe.txt"), "old\n").unwrap();
        let init = run_command_blocking(
            dir.path(),
            "git init -q && git config user.name test && git config user.email test@example.com && git add safe.txt && git commit -qm init",
        )
        .unwrap();
        assert_eq!(init.exit_code, Some(0), "{}", init.stderr);
        fs::write(dir.path().join("safe.txt"), "approved\n").unwrap();
        assert_eq!(
            run_command_blocking(dir.path(), "git add safe.txt")
                .unwrap()
                .exit_code,
            Some(0)
        );
        let hook = dir.path().join(".git/hooks/pre-commit");
        fs::write(
            &hook,
            "#!/bin/sh\nprintf 'hook changed\\n' > safe.txt\ngit add safe.txt\n",
        )
        .unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        let session = crate::session::SessionStore::new(dir.path())
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);
        let args = json!({"message": "approved tree"});

        executor
            .execute("git_commit", args.clone())
            .await
            .unwrap_err();
        let approval = session.load_approval_requests().unwrap().remove(0);
        assert!(approval
            .input_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("staged_files=1")));
        session
            .approve_approval_request(&approval.id.to_string())
            .unwrap();
        let execution = executor.execute("git_commit", args).await.unwrap();

        assert!(execution.success, "{}", execution.content);
        let committed = run_command_blocking(dir.path(), "git show HEAD:safe.txt").unwrap();
        assert_eq!(committed.exit_code, Some(0), "{}", committed.stderr);
        assert_eq!(committed.stdout, "approved\n");
        assert_eq!(
            fs::read_to_string(dir.path().join("safe.txt")).unwrap(),
            "approved\n"
        );
    }

    #[tokio::test]
    async fn apply_patch_tool_accepts_exact_replace() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "alpha\nbeta\ngamma\n").unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);
        let result = executor
            .execute(
                "apply_patch_or_write",
                json!({
                    "path": "src/lib.rs",
                    "old": "beta\n",
                    "new": "delta\n"
                }),
            )
            .await
            .unwrap();

        assert!(result.content.contains("-beta"));
        assert!(result.content.contains("+delta"));
        assert_eq!(
            fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "alpha\ndelta\ngamma\n"
        );
    }

    #[test]
    fn normalizes_unified_diff_hunk_counts() {
        let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1,99 +1,99 @@\n\n old\n-old2\n+new2\n";
        let normalized = normalize_unified_diff_hunk_counts(patch);
        assert!(normalized.contains("@@ -1,3 +1,3 @@"));
        assert!(normalized.contains("@@\n \n old"));
    }

    #[test]
    fn normalizes_hunk_only_patch_when_path_is_present() {
        let patch = "@@ -1 +1 @@\n-old\n+new\n";
        let normalized = normalize_patch_input(patch, Some("note.txt"));
        assert!(normalized.starts_with("--- a/note.txt\n+++ b/note.txt\n"));
    }

    #[test]
    fn bool_arg_accepts_string_booleans() {
        assert!(bool_arg(
            &json!({"install_missing": "true"}),
            "install_missing",
            false
        ));
        assert!(bool_arg(
            &json!({"install_missing": "yes"}),
            "install_missing",
            false
        ));
        assert!(!bool_arg(
            &json!({"install_missing": "false"}),
            "install_missing",
            true
        ));
        assert!(bool_arg(&json!({}), "install_missing", true));
    }

    #[test]
    fn provider_timeout_can_shorten_but_not_extend_the_host_limit() {
        assert_eq!(bounded_timeout_seconds(&json!({}), 120), 120);
        assert_eq!(
            bounded_timeout_seconds(&json!({"timeout_seconds": 5}), 120),
            5
        );
        assert_eq!(
            bounded_timeout_seconds(&json!({"timeout_seconds": 999}), 120),
            120
        );
    }

    #[tokio::test]
    async fn write_file_rejects_placeholder_overwrite_of_large_file() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        let path = dir.path().join("src/lib.rs");
        let original = "pub fn keep() {}\n".repeat(100);
        fs::write(&path, &original).unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);

        let error = executor
            .execute(
                "apply_patch_or_write",
                json!({
                    "path": "src/lib.rs",
                    "content": "// placeholder"
                }),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("placeholder-like content"));
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[tokio::test]
    async fn write_file_rejects_large_file_shrink_rewrite() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        let path = dir.path().join("src/lib.rs");
        let original = "pub fn keep() {}\n".repeat(700);
        fs::write(&path, &original).unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);

        let error = executor
            .execute(
                "apply_patch_or_write",
                json!({
                    "path": "src/lib.rs",
                    "content": "pub fn replacement() {}\n".repeat(100)
                }),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("much shorter content"));
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[tokio::test]
    async fn write_file_rejects_full_rewrite_of_existing_large_file() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        let path = dir.path().join("src/lib.rs");
        let original = "pub fn keep() {}\n".repeat(700);
        fs::write(&path, &original).unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);

        let error = executor
            .execute(
                "write_file",
                json!({
                    "path": "src/lib.rs",
                    "content": "pub fn changed() {}\n".repeat(700)
                }),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("full file content"));
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[tokio::test]
    async fn run_tests_records_test_result_in_session() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Makefile"), "test:\n\t@printf ok\n").unwrap();
        let store = crate::session::SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        )
        .with_auto_reviewer(true);
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);
        let execution = executor
            .execute("run_tests", json!({"command": "make test"}))
            .await
            .unwrap();
        assert!(execution.raw["passed"].as_bool().unwrap());
        assert_eq!(session.activity_summary().unwrap().test_run_count, 1);
    }

    #[tokio::test]
    async fn default_test_approval_is_bound_to_the_resolved_command() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Makefile"), "test:\n\t@printf ok\n").unwrap();
        let session = crate::session::SessionStore::new(dir.path())
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);

        executor.execute("run_tests", json!({})).await.unwrap_err();
        let first = session.load_approval_requests().unwrap().remove(0);
        assert_eq!(first.input_summary.as_deref(), Some("command=make test"));
        session
            .approve_approval_request(&first.id.to_string())
            .unwrap();

        fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        executor.execute("run_tests", json!({})).await.unwrap_err();
        let approvals = session.load_approval_requests().unwrap();
        assert_eq!(approvals.len(), 2);
        assert_eq!(
            approvals[0].status,
            crate::session::ApprovalStatus::Approved
        );
        assert_eq!(approvals[1].status, crate::session::ApprovalStatus::Pending);
        assert_eq!(
            approvals[1].input_summary.as_deref(),
            Some("command=cargo test")
        );
    }

    #[tokio::test]
    async fn write_without_approval_records_pending_approval() {
        let dir = tempdir().unwrap();
        let store = crate::session::SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);
        let error = executor
            .execute(
                "write_file",
                json!({"path": "note.txt", "content": "pending\n"}),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("operation requires approval"));
        let approvals = session.load_approval_requests().unwrap();
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].tool, "write_file");
        assert_eq!(approvals[0].status, crate::session::ApprovalStatus::Pending);
    }

    #[tokio::test]
    async fn bound_approval_authorizes_one_exact_tool_invocation() {
        let dir = tempdir().unwrap();
        let session = crate::session::SessionStore::new(dir.path())
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);
        let args = json!({"path": "note.txt", "content": "approved once\n"});

        executor
            .execute("write_file", args.clone())
            .await
            .unwrap_err();
        let approval = session.load_approval_requests().unwrap().remove(0);
        session
            .approve_approval_request(&approval.id.to_string())
            .unwrap();

        executor.execute("write_file", args.clone()).await.unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "approved once\n"
        );

        let replay = executor
            .execute("write_file", args)
            .await
            .expect_err("the consumed approval must not authorize a replay");
        assert!(replay.to_string().contains("operation requires approval"));
    }

    #[tokio::test]
    async fn assume_yes_allows_workspace_writes_but_not_dangerous_shell() {
        let dir = tempdir().unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);
        executor
            .execute(
                "write_file",
                json!({"path": "note.txt", "content": "approved by yes\n"}),
            )
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "approved by yes\n"
        );

        let error = executor
            .execute("run_shell", json!({"command": "rm -rf target"}))
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("operation requires double confirmation"));
    }

    #[tokio::test]
    async fn tool_execution_records_lifecycle_statuses() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("note.txt"), "hello\n").unwrap();
        let store = crate::session::SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);
        executor
            .execute("read_file", json!({"path": "note.txt"}))
            .await
            .unwrap();
        assert_eq!(session.activity_summary().unwrap().tool_call_count, 4);
    }

    #[tokio::test]
    async fn spawn_subagent_starts_or_schedules_runnable_task() {
        let dir = tempdir().unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);
        let execution = executor
            .execute(
                "spawn_subagent",
                json!({
                    "task": "inspect parser",
                    "write_scope": ["src/parser.rs"]
                }),
            )
            .await
            .unwrap();
        let id = execution.raw["id"].as_str().unwrap();
        assert!(dir
            .path()
            .join(".deepcli/agents/tasks")
            .join(format!("{id}.json"))
            .exists());
        assert_eq!(
            execution.raw["event_log_path"],
            format!(".deepcli/agents/events/{id}.jsonl")
        );
        assert_eq!(
            execution.raw["output_log_path"],
            format!(".deepcli/agents/logs/{id}.log")
        );
        assert!(matches!(
            execution.raw["background_start"]["status"].as_str(),
            Some("started") | Some("scheduled")
        ));
        let events = AgentStore::new(dir.path())
            .read_subagent_events(uuid::Uuid::parse_str(id).unwrap())
            .unwrap();
        assert!(events.iter().any(|event| event.event_type == "scheduled"));
        assert!(execution.raw["next_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action == &json!(format!("deepcli agent logs {id} --json"))));
        let next_actions = execution.raw["next_actions"].as_array().unwrap();
        assert!(next_actions
            .iter()
            .any(|action| action == &json!(format!("deepcli agent resume {id} --json"))));
        assert!(!next_actions.iter().any(|action| action
            .as_str()
            .is_some_and(|value| value.contains("agent run"))));
    }

    #[tokio::test]
    async fn spawn_subagent_persists_scope_tool_and_context_hints() {
        let dir = tempdir().unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);
        let execution = executor
            .execute(
                "spawn_subagent",
                json!({
                    "task": "inspect runtime",
                    "read_scope": ["src/runtime.rs"],
                    "write_scope": ["src/tools.rs"],
                    "allowed_tools": ["read_file", "search"],
                    "context": "only inspect tool batching"
                }),
            )
            .await
            .unwrap();

        assert_eq!(execution.structured.kind, "subagent_task");
        assert_eq!(execution.raw["read_scope"][0], "src/runtime.rs");
        assert_eq!(execution.raw["write_scope"][0], "src/tools.rs");
        assert_eq!(execution.raw["allowed_tools"][0], "read_file");
        assert_eq!(execution.raw["context"], "only inspect tool batching");
        assert!(execution.raw["child_session_id"].is_null());
        assert!(execution.raw["attempts"].as_u64().unwrap() <= 1);
        assert!(
            execution.raw["background_start"]["pid"].is_u64()
                || execution.raw["background_start"]["pid"].is_null()
        );
    }

    #[tokio::test]
    async fn spawn_subagent_rejects_model_controlled_depth() {
        let dir = tempdir().unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);

        let error = executor
            .execute(
                "spawn_subagent",
                json!({"task": "inspect runtime", "depth": 257}),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("unsupported argument `depth`"));
    }

    #[tokio::test]
    async fn subagent_capability_enforces_allowed_tools_and_canonical_path_scopes() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/allowed.txt"), "allowed\n").unwrap();
        fs::write(dir.path().join("outside.txt"), "outside\n").unwrap();
        let task = AgentStore::new(dir.path())
            .create_subagent_task_with_options(crate::agents::SubagentTaskOptions {
                parent_session_id: None,
                task: "edit one file".to_string(),
                depth: 1,
                read_scope: vec![PathBuf::from("src/allowed.txt")],
                write_scope: vec![PathBuf::from("src/output.txt")],
                allowed_tools: vec!["read_file".to_string(), "write_file".to_string()],
                context: None,
            })
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let mut executor =
            ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);
        executor.restrict_to_subagent(&task).unwrap();

        assert!(executor
            .execute("read_file", json!({"path": "src/allowed.txt"}))
            .await
            .is_ok());
        let outside_read = executor
            .execute("read_file", json!({"path": "outside.txt"}))
            .await
            .unwrap_err();
        assert!(outside_read
            .to_string()
            .contains("read capability does not include path"));
        let disallowed_tool = executor
            .execute("list_files", json!({"path": "src"}))
            .await
            .unwrap_err();
        assert!(disallowed_tool
            .to_string()
            .contains("does not allow tool `list_files`"));
        executor
            .execute(
                "write_file",
                json!({"path": "src/output.txt", "content": "written\n"}),
            )
            .await
            .unwrap();
        let outside_write = executor
            .execute(
                "write_file",
                json!({"path": "src/other.txt", "content": "blocked\n"}),
            )
            .await
            .unwrap_err();
        assert!(outside_write
            .to_string()
            .contains("write capability does not include path"));
        assert!(!dir.path().join("src/other.txt").exists());
    }

    #[tokio::test]
    async fn scoped_subagent_rejects_tools_without_enforceable_path_boundaries() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        let task = AgentStore::new(dir.path())
            .create_subagent_task_with_options(crate::agents::SubagentTaskOptions {
                parent_session_id: None,
                task: "inspect src".to_string(),
                depth: 1,
                read_scope: vec![PathBuf::from("src")],
                write_scope: Vec::new(),
                allowed_tools: vec!["run_shell".to_string()],
                context: None,
            })
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let mut executor =
            ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);
        executor.restrict_to_subagent(&task).unwrap();

        let error = executor
            .execute("run_shell", json!({"command": "cat outside.txt"}))
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("cannot safely enforce path scopes for tool `run_shell`"));
    }

    #[tokio::test]
    async fn nested_subagent_depth_is_computed_by_the_host() {
        let dir = tempdir().unwrap();
        let task = AgentStore::new(dir.path())
            .create_subagent_task_with_options(crate::agents::SubagentTaskOptions {
                parent_session_id: None,
                task: "parent".to_string(),
                depth: 2,
                read_scope: Vec::new(),
                write_scope: Vec::new(),
                allowed_tools: vec!["spawn_subagent".to_string()],
                context: None,
            })
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let mut executor =
            ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);
        executor.restrict_to_subagent(&task).unwrap();

        let error = executor
            .execute("spawn_subagent", json!({"task": "must be depth three"}))
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("exceeds configured maxSubagentDepth 2"));
        assert!(AgentStore::new(dir.path()).list().unwrap().len() == 1);
    }

    #[tokio::test]
    async fn nested_subagent_cannot_widen_parent_tool_capability() {
        let dir = tempdir().unwrap();
        let task = AgentStore::new(dir.path())
            .create_subagent_task_with_options(crate::agents::SubagentTaskOptions {
                parent_session_id: None,
                task: "delegator".to_string(),
                depth: 1,
                read_scope: Vec::new(),
                write_scope: Vec::new(),
                allowed_tools: vec!["spawn_subagent".to_string()],
                context: None,
            })
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let mut executor =
            ToolExecutor::new(dir.path(), permissions, None, 3).with_assume_yes(true);
        executor.restrict_to_subagent(&task).unwrap();

        let error = executor
            .execute(
                "spawn_subagent",
                json!({"task": "escape", "allowed_tools": ["read_file"]}),
            )
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("cannot widen parent tool capability"));
        assert!(AgentStore::new(dir.path()).list().unwrap().len() == 1);
    }

    #[tokio::test]
    async fn todo_write_updates_session_plan_and_structured_result() {
        let dir = tempdir().unwrap();
        let store = crate::session::SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);

        let execution = executor
            .execute(
                "todo_write",
                json!({
                    "todos": [
                        {"id": "context", "content": "Read tool chain", "status": "completed"},
                        {"id": "implementation", "content": "Patch tool protocol", "status": "in_progress"}
                    ]
                }),
            )
            .await
            .unwrap();

        assert_eq!(execution.structured.kind, "todo_list");
        assert_eq!(execution.raw["count"], 2);
        let plan = session.load_plan().unwrap().unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].id, "context");
        assert_eq!(
            plan.steps[0].status,
            crate::session::PlanStepStatus::Completed
        );
        assert_eq!(
            plan.steps[1].status,
            crate::session::PlanStepStatus::InProgress
        );
    }

    #[tokio::test]
    async fn ask_user_question_enqueues_side_question_and_structured_result() {
        let dir = tempdir().unwrap();
        let store = crate::session::SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);

        let execution = executor
            .execute(
                "ask_user_question",
                json!({"question": "Which target should I verify?"}),
            )
            .await
            .unwrap();

        assert_eq!(execution.structured.kind, "question");
        let questions = session.load_side_questions().unwrap();
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].question, "Which target should I verify?");
        assert_eq!(execution.raw["question"]["status"], "open");
    }

    #[tokio::test]
    async fn ask_user_question_enqueues_options_for_interview_prompt() {
        let dir = tempdir().unwrap();
        let store = crate::session::SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, Some(session.clone()), 2);

        executor
            .execute(
                "ask_user_question",
                json!({
                    "question": "Which plan route should I use?",
                    "options": ["Validate first", "Implement Task 6"]
                }),
            )
            .await
            .unwrap();

        let questions = session.load_side_questions().unwrap();
        assert_eq!(
            questions[0].options,
            vec!["Validate first".to_string(), "Implement Task 6".to_string()]
        );
    }

    #[tokio::test]
    async fn web_fetch_rejects_non_http_urls_before_network() {
        let dir = tempdir().unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let error = executor
            .execute("web_fetch", json!({"url": "file:///etc/passwd"}))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("http or https"));
    }

    #[tokio::test]
    async fn list_files_supports_path_glob_and_metadata() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::create_dir_all(dir.path().join("tests")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        fs::write(dir.path().join("src/readme.txt"), "note\n").unwrap();
        fs::write(
            dir.path().join("tests/contract.rs"),
            "#[test]\nfn ok() {}\n",
        )
        .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let execution = executor
            .execute(
                "list_files",
                json!({"path": "src", "glob": "**/*.rs", "limit": 10}),
            )
            .await
            .unwrap();

        assert_eq!(execution.structured.kind, "file_list");
        assert_eq!(execution.raw["count"], 1);
        assert_eq!(execution.raw["truncated"], false);
        assert_eq!(execution.content.trim(), "src/lib.rs");
    }

    #[tokio::test]
    async fn search_supports_case_insensitive_glob_and_context() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "alpha\nBeta target\nomega\n").unwrap();
        fs::write(dir.path().join("README.md"), "beta target\n").unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let execution = executor
            .execute(
                "search",
                json!({
                    "query": "beta",
                    "glob": "src/**/*.rs",
                    "case_sensitive": false,
                    "context_lines": 1,
                    "limit": 10
                }),
            )
            .await
            .unwrap();

        assert_eq!(execution.structured.kind, "search_matches");
        assert_eq!(execution.raw["count"], 1);
        assert_eq!(execution.raw["matches"][0]["path"], "src/lib.rs");
        assert_eq!(execution.raw["matches"][0]["before"][0], "alpha");
        assert_eq!(execution.raw["matches"][0]["after"][0], "omega");
    }

    #[tokio::test]
    async fn tool_validation_rejects_extra_arguments() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("note.txt"), "hello\n").unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let error = executor
            .execute("read_file", json!({"path": "note.txt", "unexpected": true}))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("unsupported argument"));
    }

    #[tokio::test]
    async fn prompt_tools_list_and_get_project_prompts() {
        let dir = tempdir().unwrap();
        let store = PromptStore::new(dir.path());
        store.save("reviewer", "Review this diff").unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let list = executor.execute("prompt_list", json!({})).await.unwrap();
        assert!(list.content.contains("code-review"));
        assert!(list.content.contains("reviewer - Custom project prompt"));

        let get = executor
            .execute("prompt_get", json!({"name": "reviewer"}))
            .await
            .unwrap();
        assert_eq!(get.content, "Review this diff");
        assert_eq!(get.raw["name"], "reviewer");
    }

    #[tokio::test]
    async fn prompt_render_tool_expands_git_diff_file_and_custom_variables() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn before() {}\n").unwrap();
        run_command_blocking(dir.path(), "git init --quiet").unwrap();
        run_command_blocking(dir.path(), "git checkout -b feature/prompt --quiet").unwrap();
        run_command_blocking(dir.path(), "git add src/lib.rs").unwrap();
        run_command_blocking(
            dir.path(),
            "git -c user.email=a@example.test -c user.name=deepcli commit -m initial --quiet",
        )
        .unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn after() {}\n").unwrap();
        let store = PromptStore::new(dir.path());
        store
            .save(
                "render-me",
                "{{task}}\nbranch={{branch}}\nfile={{file}}\n{{file_content}}\n{{diff}}",
            )
            .unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let rendered = executor
            .execute(
                "prompt_render",
                json!({
                    "name": "render-me",
                    "file": "src/lib.rs",
                    "variables": {"task": "review"},
                    "max_diff_chars": 4000
                }),
            )
            .await
            .unwrap();

        assert!(rendered.content.contains("review"));
        assert!(rendered.content.contains("branch=feature/prompt"));
        assert!(rendered.content.contains("file=src/lib.rs"));
        assert!(rendered.content.contains("pub fn after()"));
        assert!(rendered.content.contains("-pub fn before()"));
        assert!(rendered.content.contains("+pub fn after()"));
    }

    #[tokio::test]
    async fn skill_list_tool_reports_empty_and_registered_skills() {
        let dir = tempdir().unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2).with_assume_yes(true);

        let empty = executor.execute("skill_list", json!({})).await.unwrap();
        assert!(empty.content.contains("no project skills registered"));

        executor
            .execute(
                "skill_generate",
                json!({
                    "name": "compiler",
                    "description": "SysY compiler workflow"
                }),
            )
            .await
            .unwrap();
        let listed = executor.execute("skill_list", json!({})).await.unwrap();
        assert!(listed.content.contains("compiler - SysY compiler workflow"));
        assert!(listed.content.contains("trigger:"));
    }
}
