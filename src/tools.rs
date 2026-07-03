use crate::agents::AgentStore;
use crate::permissions::{
    DecisionOutcome, PermissionDecision, PermissionEngine, RiskLevel, ToolRequest, ToolSurface,
};
use crate::privacy::{looks_sensitive, redact_sensitive_value};
use crate::prompts::{render_prompt_body, Prompt, PromptRenderContext, PromptStore};
use crate::session::{
    Plan, PlanStep, PlanStepStatus, Session, TestRunRecord, ToolCallRecord, ToolCallStatus,
};
use crate::skills::{SkillMetadata, SkillStore};
use crate::workspace::WorkspaceManager;
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use globset::{Glob, GlobSet};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod declarations;
mod environment;
mod file;
mod git;
mod process;
mod schema;
mod test_discovery;
mod validation;
mod web;

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
    normalize_patch_input, reject_large_destructive_rewrite, reject_large_existing_rewrite,
    reject_placeholder_overwrite, slice_text_by_line, unified_diff,
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
use web::{format_web_fetch_text, format_web_search_result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolExecution {
    pub tool: String,
    pub content: String,
    pub raw: Value,
    pub structured: StructuredToolResult,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredToolResult {
    pub kind: String,
    pub summary: String,
    pub data: Value,
    pub truncated: bool,
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
        }
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
            "ok": true,
            "kind": self.structured.kind,
            "summary": self.structured.summary,
            "content": self.content,
            "data": self.structured.data,
            "truncated": self.structured.truncated,
        }))
        .unwrap_or_else(|_| self.content.clone())
    }
}

pub struct ToolExecutor {
    workspace: PathBuf,
    permissions: PermissionEngine,
    session: Option<Session>,
    max_subagent_depth: u8,
    assume_yes: bool,
}

impl ToolExecutor {
    pub fn new(
        workspace: impl AsRef<Path>,
        permissions: PermissionEngine,
        session: Option<Session>,
        max_subagent_depth: u8,
    ) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
            permissions,
            session,
            max_subagent_depth,
            assume_yes: false,
        }
    }

    pub fn with_assume_yes(mut self, assume_yes: bool) -> Self {
        self.assume_yes = assume_yes;
        self
    }

    pub fn set_session(&mut self, session: Option<Session>) {
        self.session = session;
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
                Ok(execution) => (ToolCallStatus::Succeeded, execution.raw.clone()),
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
        let validation = if let Some(tool) = ToolRegistry::mvp().tool(name) {
            tool.validate_arguments(&args)
        } else {
            validate_tool_arguments(name, &args)
        };
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
                Ok(execution) => (ToolCallStatus::Succeeded, execution.raw.clone()),
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
        self.ensure_allowed("read_file", &decision, false)?;
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
        self.ensure_allowed("list_files", &decision, false)?;
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
        self.ensure_allowed("search", &decision, false)?;
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
            .unwrap_or(1_000_000) as u64;
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
        let approved = bool_arg(&args, "approved", false);
        let decision = self.evaluate_filesystem(name, &path, true)?;
        self.ensure_allowed(name, &decision, approved)?;

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
        let approved = bool_arg(&args, "approved", false);
        let decision = self.evaluate_filesystem("apply_patch_or_write", &self.workspace, true)?;
        self.ensure_allowed("apply_patch_or_write", &decision, approved)?;

        let patch_to_apply = normalize_patch_input(patch, args.get("path").and_then(Value::as_str));
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
        let approved = bool_arg(&args, "approved", false);
        let decision = self.evaluate_filesystem(name, &path, true)?;
        self.ensure_allowed(name, &decision, approved)?;

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
        let approved = bool_arg(&args, "approved", false);
        let decision = self.evaluate_declared_tool(
            "run_shell",
            ToolPermissionContext {
                command: Some(command.to_string()),
                path: Some(self.workspace.clone()),
                writes_files: Some(bool_arg(&args, "writes_files", false)),
                creates_process: true,
                requires_network: Some(bool_arg(&args, "requires_network", false)),
                explicit_approval: approved,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("run_shell", &decision, approved)?;
        let timeout_seconds = args
            .get("timeout_seconds")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .unwrap_or_else(default_shell_timeout_seconds);
        let output = run_command_with_timeout(
            &self.workspace,
            command,
            Duration::from_secs(timeout_seconds),
        )
        .await?;
        let content = output_text(&output);
        let raw = json!(output);
        Ok(
            ToolExecution::new("run_shell", content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false),
        )
    }

    async fn git_status(&self) -> Result<ToolExecution> {
        self.git_read_tool("git_status", "git status --short").await
    }

    async fn git_diff(&self, args: Value) -> Result<ToolExecution> {
        let staged = args.get("staged").and_then(Value::as_bool).unwrap_or(false);
        let command = if staged {
            "git diff --cached"
        } else {
            "git diff"
        };
        self.git_read_tool("git_diff", command).await
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
        let approved = bool_arg(&args, "approved", false);
        let command = format!("git switch -c {}", shell_words::quote(name));
        let decision = self.evaluate_declared_tool(
            "git_create_branch",
            ToolPermissionContext {
                command: Some(command.clone()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                explicit_approval: approved,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("git_create_branch", &decision, approved)?;
        let output = run_command(&self.workspace, &command).await?;
        let content = output_text(&output);
        let raw = json!(output);
        Ok(
            ToolExecution::new("git_create_branch", content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false),
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
        self.ensure_allowed("git_commit_message", &decision, false)?;
        let status = run_command(&self.workspace, "git status --short").await?;
        let names = run_command(&self.workspace, "git diff --name-only").await?;
        let stat = run_command(&self.workspace, "git diff --stat").await?;
        let message = generate_commit_message(&status.stdout, &names.stdout);
        let raw = json!({
            "message": message,
            "status": status,
            "changed_files": names.stdout.lines().collect::<Vec<_>>(),
            "stat": stat.stdout
        });
        Ok(
            ToolExecution::new("git_commit_message", message.clone(), raw.clone(), decision)
                .with_structured("git_commit_message", message, raw, false),
        )
    }

    async fn git_commit(&self, args: Value) -> Result<ToolExecution> {
        let message = required_str(&args, "message")?;
        let approved = bool_arg(&args, "approved", false);
        let command = format!("git commit -m {}", shell_words::quote(message));
        let decision = self.evaluate_declared_tool(
            "git_commit",
            ToolPermissionContext {
                command: Some(command.clone()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                explicit_approval: approved,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("git_commit", &decision, approved)?;
        let output = run_command(&self.workspace, &command).await?;
        let content = output_text(&output);
        let raw = json!(output);
        Ok(
            ToolExecution::new("git_commit", content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false),
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
        self.ensure_allowed(name, &decision, false)?;
        let output = run_command(&self.workspace, command).await?;
        let content = output_text(&output);
        let raw = json!(output);
        Ok(
            ToolExecution::new(name, content.clone(), raw.clone(), decision).with_structured(
                "command_output",
                first_line(&content),
                raw,
                false,
            ),
        )
    }

    async fn discover_tests_tool(&self) -> Result<ToolExecution> {
        let decision = self.evaluate_filesystem("discover_tests", &self.workspace, false)?;
        self.ensure_allowed("discover_tests", &decision, false)?;
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
        let decision = self.evaluate_declared_tool(
            "run_tests",
            ToolPermissionContext {
                command: Some(command.clone()),
                path: Some(self.workspace.clone()),
                creates_process: true,
                explicit_approval: true,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("run_tests", &decision, true)?;
        let output = run_command(&self.workspace, &command).await?;
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
                ),
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
        self.ensure_allowed("check_environment", &decision, false)?;
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
        let approved = bool_arg(&args, "approved", false);
        let install_missing = bool_arg(&args, "install_missing", true);
        let smoke_test = bool_arg(&args, "smoke_test", false);
        let decision = self.evaluate_declared_tool(
            "setup_environment",
            ToolPermissionContext {
                command: Some(format!("deepcli environment setup {target}")),
                path: Some(self.workspace.clone()),
                network_target: Some("ghcr.io, docker.io".to_string()),
                creates_process: true,
                explicit_approval: approved,
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("setup_environment", &decision, approved)?;
        let setup =
            setup_environment_in(&self.workspace, &target, install_missing, smoke_test).await?;
        let content = format_environment_setup(&setup);
        let raw = json!(setup);
        Ok(
            ToolExecution::new("setup_environment", content.clone(), raw.clone(), decision)
                .with_structured("environment_setup", first_line(&content), raw, false),
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
        self.ensure_allowed("todo_write", &decision, false)?;
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
        let decision = self.evaluate_declared_tool(
            "ask_user_question",
            ToolPermissionContext {
                path: Some(self.workspace.clone()),
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("ask_user_question", &decision, false)?;
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("ask_user_question requires an active session"))?;
        let item = session.enqueue_side_question(question)?;
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
        self.ensure_allowed("web_search", &decision, false)?;
        let url = reqwest::Url::parse_with_params(
            "https://api.duckduckgo.com/",
            &[("q", query), ("format", "json"), ("no_html", "1")],
        )?;
        let value: Value = reqwest::get(url).await?.json().await?;
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
        self.ensure_allowed("web_fetch", &decision, false)?;
        let max_chars = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .unwrap_or(20_000) as usize;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("deepcli-web-fetch/0.1")
            .build()?;
        let response = client.get(url.clone()).send().await?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.text().await?;
        let extracted = format_web_fetch_text(&body, &content_type);
        let original_chars = extracted.chars().count();
        let truncated = original_chars > max_chars;
        let text = if truncated {
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
            "original_chars": original_chars
        });
        Ok(
            ToolExecution::new("web_fetch", content, raw.clone(), decision).with_structured(
                "web_fetch",
                format!("fetched {} status {}", url, status),
                raw,
                truncated,
            ),
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
        self.ensure_allowed("open_terminal", &decision, false)?;
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
        let raw = json!(output);
        Ok(
            ToolExecution::new("open_terminal", content.clone(), raw.clone(), decision)
                .with_structured("command_output", first_line(&content), raw, false),
        )
    }

    async fn prompt_list(&self) -> Result<ToolExecution> {
        let decision = self.evaluate_filesystem(
            "prompt_list",
            &self.workspace.join(".deepcli/prompts"),
            false,
        )?;
        self.ensure_allowed("prompt_list", &decision, false)?;
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
        self.ensure_allowed("prompt_get", &decision, false)?;
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
        self.ensure_allowed("prompt_render", &decision, false)?;
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
        self.ensure_allowed("skill_list", &decision, false)?;
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
        let diff_command =
            "git diff -- . ':(exclude).deepcli/credentials/**' ':(exclude).env' ':(exclude).env.*'";
        let diff = truncate_display(
            command_stdout_or_empty(&self.workspace, diff_command)
                .await?
                .trim(),
            max_diff_chars,
        );

        let (file, file_content) = if let Some(raw_file) = args.get("file").and_then(Value::as_str)
        {
            let file_path = resolve_workspace_path(&self.workspace, raw_file)?;
            let file_decision = self.evaluate_filesystem("prompt_render", &file_path, false)?;
            self.ensure_allowed("prompt_render", &file_decision, false)?;
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
        let approved = bool_arg(&args, "approved", false);
        let decision = self.evaluate_filesystem(
            "skill_generate",
            &self.workspace.join(".deepcli/skills").join(name),
            true,
        )?;
        self.ensure_allowed("skill_generate", &decision, approved)?;
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
        self.ensure_allowed("skill_run", &decision, false)?;
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
        let depth = args.get("depth").and_then(Value::as_u64).unwrap_or(1) as u8;
        let task = required_str(&args, "task")?;
        let write_scope = string_array_arg(&args, "write_scope");
        let read_scope = string_array_arg(&args, "read_scope");
        let allowed_tools = string_array_arg(&args, "allowed_tools")
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
        let decision = self.evaluate_declared_tool(
            "spawn_subagent",
            ToolPermissionContext {
                path: Some(self.workspace.clone()),
                ..ToolPermissionContext::default()
            },
        )?;
        self.ensure_allowed("spawn_subagent", &decision, false)?;
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
        let content = format!(
            "sub-agent queued at depth {depth}: {task} ({})",
            subagent.id
        );
        let raw = json!(subagent);
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
        Ok(self.permissions.evaluate(&ToolRequest {
            tool: tool.to_string(),
            surface: ToolSurface::Filesystem,
            command: None,
            path: Some(path.to_path_buf()),
            network_target: None,
            writes_files,
            creates_process: false,
            requires_network: false,
            explicit_approval: false,
        }))
    }

    fn ensure_allowed(
        &self,
        tool: &str,
        decision: &PermissionDecision,
        approved: bool,
    ) -> Result<()> {
        match decision.outcome {
            DecisionOutcome::Allowed | DecisionOutcome::AutoApproved => Ok(()),
            DecisionOutcome::RequiresUserApproval if approved => Ok(()),
            DecisionOutcome::RequiresUserApproval if self.can_assume_yes(tool, decision) => Ok(()),
            DecisionOutcome::DoubleConfirmRequired if approved => Ok(()),
            DecisionOutcome::Denied => bail!("permission denied: {}", decision.reason),
            DecisionOutcome::RequiresUserApproval => {
                let approval = self.enqueue_approval_request(tool, decision)?;
                bail!(
                    "operation requires approval: {} (approval {})",
                    decision.reason,
                    short_id(&approval.id)
                )
            }
            DecisionOutcome::DoubleConfirmRequired => {
                let approval = self.enqueue_approval_request(tool, decision)?;
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
        decision: &PermissionDecision,
    ) -> Result<crate::session::ApprovalRequest> {
        if let Some(session) = &self.session {
            session.enqueue_approval_request(tool, decision.clone())
        } else {
            Ok(crate::session::ApprovalRequest {
                id: uuid::Uuid::nil(),
                tool: tool.to_string(),
                decision: decision.clone(),
                status: crate::session::ApprovalStatus::Pending,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        }
    }

    fn can_assume_yes(&self, tool: &str, decision: &PermissionDecision) -> bool {
        self.assume_yes
            && decision.risk == RiskLevel::High
            && matches!(
                tool,
                "write_file" | "apply_patch_or_write" | "skill_generate" | "spawn_subagent"
            )
    }

    fn resolve_required_path(&self, args: &Value) -> Result<PathBuf> {
        let raw = required_str(args, "path")?;
        resolve_workspace_path(&self.workspace, raw)
    }

    fn resolve_optional_path(&self, args: &Value) -> Result<Option<PathBuf>> {
        args.get("path")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(|raw| resolve_workspace_path(&self.workspace, raw))
            .transpose()
    }
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("missing required string argument `{key}`"))
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
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);
        executor
            .execute(
                "write_file",
                json!({"path": "src/lib.rs", "content": "pub fn ok() {}\n", "approved": true}),
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
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);
        executor
            .execute(
                "apply_patch_or_write",
                json!({
                    "patch": "--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-old\n+new\n",
                    "approved": true
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
    async fn apply_patch_tool_accepts_exact_replace() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "alpha\nbeta\ngamma\n").unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);
        let result = executor
            .execute(
                "apply_patch_or_write",
                json!({
                    "path": "src/lib.rs",
                    "old": "beta\n",
                    "new": "delta\n",
                    "approved": true
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
        assert!(bool_arg(&json!({"approved": "true"}), "approved", false));
        assert!(bool_arg(&json!({"approved": "yes"}), "approved", false));
        assert!(!bool_arg(&json!({"approved": "false"}), "approved", true));
        assert!(bool_arg(&json!({}), "approved", true));
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
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let error = executor
            .execute(
                "apply_patch_or_write",
                json!({
                    "path": "src/lib.rs",
                    "content": "// placeholder",
                    "approved": true
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
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let error = executor
            .execute(
                "apply_patch_or_write",
                json!({
                    "path": "src/lib.rs",
                    "content": "pub fn replacement() {}\n".repeat(100),
                    "approved": true
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
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let error = executor
            .execute(
                "write_file",
                json!({
                    "path": "src/lib.rs",
                    "content": "pub fn changed() {}\n".repeat(700),
                    "approved": true
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
            .execute("run_tests", json!({"command": "printf ok"}))
            .await
            .unwrap();
        assert!(execution.raw["passed"].as_bool().unwrap());
        assert_eq!(session.activity_summary().unwrap().test_run_count, 1);
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
    async fn spawn_subagent_persists_task_descriptor() {
        let dir = tempdir().unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);
        let execution = executor
            .execute(
                "spawn_subagent",
                json!({
                    "task": "inspect parser",
                    "depth": 1,
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
    }

    #[tokio::test]
    async fn spawn_subagent_persists_scope_tool_and_context_hints() {
        let dir = tempdir().unwrap();
        let permissions = PermissionEngine::new(
            dir.path(),
            PermissionConfig::default(),
            SandboxConfig::default(),
        );
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);
        let execution = executor
            .execute(
                "spawn_subagent",
                json!({
                    "task": "inspect runtime",
                    "depth": 1,
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
        let executor = ToolExecutor::new(dir.path(), permissions, None, 2);

        let empty = executor.execute("skill_list", json!({})).await.unwrap();
        assert!(empty.content.contains("no project skills registered"));

        executor
            .execute(
                "skill_generate",
                json!({
                    "name": "compiler",
                    "description": "SysY compiler workflow",
                    "approved": true
                }),
            )
            .await
            .unwrap();
        let listed = executor.execute("skill_list", json!({})).await.unwrap();
        assert!(listed.content.contains("compiler - SysY compiler workflow"));
        assert!(listed.content.contains("trigger:"));
    }
}
