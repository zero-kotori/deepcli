use crate::agents::AgentStore;
use crate::permissions::{
    DecisionOutcome, PermissionDecision, PermissionEngine, RiskLevel, ToolRequest, ToolSurface,
};
use crate::privacy::{looks_sensitive, redact_sensitive_value};
use crate::prompts::{render_prompt_body, Prompt, PromptRenderContext, PromptStore};
use crate::session::{Session, TestRunRecord, ToolCallRecord, ToolCallStatus};
use crate::skills::{SkillMetadata, SkillStore};
use crate::workspace::WorkspaceManager;
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use similar::TextDiff;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

mod declarations;
mod schema;

pub use declarations::{ToolDeclaration, ToolPermissionContext, ToolRegistry};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolExecution {
    pub tool: String,
    pub content: String,
    pub raw: Value,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveredTestCommand {
    pub source: PathBuf,
    pub command: String,
    #[serde(default)]
    pub requires_docker: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentCheck {
    pub name: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentReport {
    pub target: String,
    pub ready: bool,
    pub checks: Vec<EnvironmentCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentSetupResult {
    pub target: String,
    pub before: EnvironmentReport,
    pub actions: Vec<CommandOutput>,
    pub after: EnvironmentReport,
    pub ready: bool,
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
        let result = match name {
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
            "web_search" => self.web_search(args).await,
            "open_terminal" => self.open_terminal().await,
            "prompt_list" => self.prompt_list().await,
            "prompt_get" => self.prompt_get(args).await,
            "prompt_render" => self.prompt_render(args).await,
            "skill_list" => self.skill_list().await,
            "skill_generate" => self.skill_generate(args).await,
            "skill_run" => self.skill_run(args).await,
            "spawn_subagent" => self.spawn_subagent(args).await,
            other => Err(anyhow!("unknown tool `{other}`")),
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
        Ok(ToolExecution {
            tool: "read_file".to_string(),
            content: content.clone(),
            raw: json!({"path": path, "content": content}),
            decision,
        })
    }

    async fn list_files(&self, args: Value) -> Result<ToolExecution> {
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(256) as usize;
        let decision = self.evaluate_filesystem("list_files", &self.workspace, false)?;
        self.ensure_allowed("list_files", &decision, false)?;
        let manager = WorkspaceManager::new(&self.workspace)?;
        let files = manager
            .walk_files(limit)?
            .into_iter()
            .map(|entry| {
                entry
                    .path()
                    .strip_prefix(&self.workspace)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        Ok(ToolExecution {
            tool: "list_files".to_string(),
            content: files.join("\n"),
            raw: json!({ "files": files }),
            decision,
        })
    }

    async fn search(&self, args: Value) -> Result<ToolExecution> {
        let query = required_str(&args, "query")?;
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
        let decision = self.evaluate_filesystem("search", &self.workspace, false)?;
        self.ensure_allowed("search", &decision, false)?;
        let manager = WorkspaceManager::new(&self.workspace)?;
        let mut matches = Vec::new();
        for entry in manager.walk_files(512)? {
            let Ok(content) = fs::read_to_string(entry.path()) else {
                continue;
            };
            for (line_number, line) in content.lines().enumerate() {
                if line.contains(query) {
                    let path = entry
                        .path()
                        .strip_prefix(&self.workspace)
                        .unwrap_or(entry.path())
                        .to_string_lossy()
                        .to_string();
                    matches.push(json!({
                        "path": path,
                        "line": line_number + 1,
                        "text": line
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
        Ok(ToolExecution {
            tool: "search".to_string(),
            content: serde_json::to_string_pretty(&matches)?,
            raw: json!({ "matches": matches }),
            decision,
        })
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

        Ok(ToolExecution {
            tool: name.to_string(),
            content: diff.clone(),
            raw: json!({"path": path, "diff": diff}),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "apply_patch_or_write".to_string(),
            content: patch_to_apply.clone(),
            raw: json!({"patch": patch_to_apply, "output": output}),
            decision,
        })
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

        Ok(ToolExecution {
            tool: name.to_string(),
            content: diff.clone(),
            raw: json!({"path": path, "diff": diff}),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "run_shell".to_string(),
            content: output_text(&output),
            raw: json!(output),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "git_create_branch".to_string(),
            content: output_text(&output),
            raw: json!(output),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "git_commit_message".to_string(),
            content: message.clone(),
            raw: json!({
                "message": message,
                "status": status,
                "changed_files": names.stdout.lines().collect::<Vec<_>>(),
                "stat": stat.stdout
            }),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "git_commit".to_string(),
            content: output_text(&output),
            raw: json!(output),
            decision,
        })
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
        Ok(ToolExecution {
            tool: name.to_string(),
            content: output_text(&output),
            raw: json!(output),
            decision,
        })
    }

    async fn discover_tests_tool(&self) -> Result<ToolExecution> {
        let decision = self.evaluate_filesystem("discover_tests", &self.workspace, false)?;
        self.ensure_allowed("discover_tests", &decision, false)?;
        let commands = self.discover_tests()?;
        Ok(ToolExecution {
            tool: "discover_tests".to_string(),
            content: commands
                .iter()
                .map(format_discovered_test_command)
                .collect::<Vec<_>>()
                .join("\n"),
            raw: json!({ "commands": commands }),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "run_tests".to_string(),
            content: output_text(&output),
            raw: json!({"passed": passed, "output": output}),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "check_environment".to_string(),
            content: format_environment_report(&report),
            raw: json!(report),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "setup_environment".to_string(),
            content: format_environment_setup(&setup),
            raw: json!(setup),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "web_search".to_string(),
            content,
            raw: value,
            decision,
        })
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
        Ok(ToolExecution {
            tool: "open_terminal".to_string(),
            content: output_text(&output),
            raw: json!(output),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "prompt_list".to_string(),
            content: format_prompt_tool_list(&prompts),
            raw: json!(prompts),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "prompt_get".to_string(),
            content: prompt.body.clone(),
            raw: json!(prompt),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "prompt_render".to_string(),
            content: rendered.clone(),
            raw: json!({
                "name": prompt.name,
                "description": prompt.description,
                "context": context,
                "rendered": rendered
            }),
            decision,
        })
    }

    async fn skill_list(&self) -> Result<ToolExecution> {
        let decision =
            self.evaluate_filesystem("skill_list", &self.workspace.join(".deepcli/skills"), false)?;
        self.ensure_allowed("skill_list", &decision, false)?;
        let store = SkillStore::new(&self.workspace);
        let skills = store.discover()?;
        Ok(ToolExecution {
            tool: "skill_list".to_string(),
            content: format_skill_tool_list(&skills),
            raw: json!(skills),
            decision,
        })
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
        Ok(ToolExecution {
            tool: "skill_generate".to_string(),
            content: skill.instruction_path.display().to_string(),
            raw: json!(skill),
            decision,
        })
    }

    async fn skill_run(&self, args: Value) -> Result<ToolExecution> {
        let name = required_str(&args, "name")?;
        let decision =
            self.evaluate_filesystem("skill_run", &self.workspace.join(".deepcli/skills"), false)?;
        self.ensure_allowed("skill_run", &decision, false)?;
        let store = SkillStore::new(&self.workspace);
        let loaded = store.load(name)?;
        Ok(ToolExecution {
            tool: "skill_run".to_string(),
            content: loaded.instructions.clone(),
            raw: json!(loaded),
            decision,
        })
    }

    async fn spawn_subagent(&self, args: Value) -> Result<ToolExecution> {
        let depth = args.get("depth").and_then(Value::as_u64).unwrap_or(1) as u8;
        let task = required_str(&args, "task")?;
        let write_scope = args
            .get("write_scope")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(PathBuf::from)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
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
        let subagent = store.create_subagent_task(parent_session_id, task, depth, write_scope)?;
        Ok(ToolExecution {
            tool: "spawn_subagent".to_string(),
            content: format!(
                "sub-agent queued at depth {depth}: {task} ({})",
                subagent.id
            ),
            raw: json!(subagent),
            decision,
        })
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandOutput {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

fn terminal_open_command(app: &str) -> String {
    format!("open -a {} .", shell_words::quote(app))
}

pub async fn run_command(workspace: &Path, command: &str) -> Result<CommandOutput> {
    let output = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(workspace)
        .output()
        .await
        .with_context(|| format!("failed to run `{command}`"))?;
    Ok(CommandOutput {
        command: command.to_string(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub fn run_command_blocking(workspace: &Path, command: &str) -> Result<CommandOutput> {
    let output = std::process::Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(workspace)
        .output()
        .with_context(|| format!("failed to run `{command}`"))?;
    Ok(CommandOutput {
        command: command.to_string(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

async fn command_stdout_or_empty(workspace: &Path, command: &str) -> Result<String> {
    let output = run_command(workspace, command).await?;
    if output.exit_code == Some(0) {
        Ok(output.stdout)
    } else {
        Ok(String::new())
    }
}

fn default_shell_timeout_seconds() -> u64 {
    env::var("DEEPCLI_RUN_SHELL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(120)
}

pub async fn run_command_with_timeout(
    workspace: &Path,
    command: &str,
    timeout_duration: Duration,
) -> Result<CommandOutput> {
    let output = tokio::time::timeout(
        timeout_duration,
        Command::new("bash")
            .arg("-lc")
            .arg(command)
            .current_dir(workspace)
            .kill_on_drop(true)
            .output(),
    )
    .await;

    match output {
        Ok(output) => {
            let output = output.with_context(|| format!("failed to run `{command}`"))?;
            Ok(CommandOutput {
                command: command.to_string(),
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
        Err(_) => Ok(CommandOutput {
            command: command.to_string(),
            exit_code: None,
            stdout: String::new(),
            stderr: format!(
                "command timed out after {} seconds",
                timeout_duration.as_secs()
            ),
        }),
    }
}

pub async fn run_command_with_stdin(
    workspace: &Path,
    command: &str,
    stdin: &str,
) -> Result<CommandOutput> {
    let mut child = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run `{command}`"))?;

    if let Some(mut child_stdin) = child.stdin.take() {
        child_stdin.write_all(stdin.as_bytes()).await?;
    }
    let output = child.wait_with_output().await?;
    Ok(CommandOutput {
        command: command.to_string(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub fn discover_tests_in(workspace: &Path) -> Result<Vec<DiscoveredTestCommand>> {
    let mut commands = Vec::new();
    let cargo = workspace.join("Cargo.toml");
    if cargo.exists() {
        commands.push(DiscoveredTestCommand {
            source: cargo,
            command: "cargo test".to_string(),
            requires_docker: false,
            available: None,
            note: None,
        });
    }

    let package_json = workspace.join("package.json");
    if package_json.exists() {
        let raw = fs::read_to_string(&package_json)?;
        if raw.contains("\"test\"") {
            commands.push(DiscoveredTestCommand {
                source: package_json,
                command: "npm test".to_string(),
                requires_docker: false,
                available: None,
                note: None,
            });
        }
    }

    let pyproject = workspace.join("pyproject.toml");
    if pyproject.exists() {
        commands.push(DiscoveredTestCommand {
            source: pyproject,
            command: "python -m pytest".to_string(),
            requires_docker: false,
            available: None,
            note: None,
        });
    }

    let makefile = workspace.join("Makefile");
    if makefile.exists() {
        let raw = fs::read_to_string(&makefile)?;
        if raw.lines().any(|line| line.starts_with("test:")) {
            commands.push(DiscoveredTestCommand {
                source: makefile,
                command: "make test".to_string(),
                requires_docker: false,
                available: None,
                note: None,
            });
        }
    }

    commands.extend(discover_compiler_autotests(workspace));
    Ok(commands)
}

fn discover_compiler_autotests(workspace: &Path) -> Vec<DiscoveredTestCommand> {
    let lv1 = workspace.join("online-doc/docs/lv1-main/testing.md");
    let lv9 = workspace.join("online-doc/docs/lv9-array/testing.md");
    if !lv1.exists() && !lv9.exists() {
        return Vec::new();
    }

    let docker_ok = docker_available();
    let note = if docker_ok {
        Some("compiler-dev Docker autotest command".to_string())
    } else {
        Some("docker is not available on PATH; install Docker before running autotest".to_string())
    };
    let mount = shell_words::quote(&workspace.display().to_string()).to_string();
    let mut commands = Vec::new();
    if lv1.exists() {
        commands.push(DiscoveredTestCommand {
            source: lv1,
            command: format!(
                "docker run --rm -v {mount}:/root/compiler maxxing/compiler-dev autotest -koopa -s lv1 /root/compiler"
            ),
            requires_docker: true,
            available: Some(docker_ok),
            note: note.clone(),
        });
    }
    if lv9.exists() {
        commands.push(DiscoveredTestCommand {
            source: lv9.clone(),
            command: format!(
                "docker run --rm -v {mount}:/root/compiler maxxing/compiler-dev autotest -koopa /root/compiler"
            ),
            requires_docker: true,
            available: Some(docker_ok),
            note: note.clone(),
        });
        commands.push(DiscoveredTestCommand {
            source: lv9,
            command: format!(
                "docker run --rm -v {mount}:/root/compiler maxxing/compiler-dev autotest -riscv /root/compiler"
            ),
            requires_docker: true,
            available: Some(docker_ok),
            note,
        });
    }
    commands
}

fn docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

async fn check_environment_in(workspace: &Path, target: &str) -> Result<EnvironmentReport> {
    let mut checks = Vec::new();
    checks.push(
        environment_command_check(
            workspace,
            "homebrew",
            "command -v brew >/dev/null 2>&1 && brew --version | head -n 1",
        )
        .await?,
    );
    checks.push(
        environment_command_check(
            workspace,
            "docker_cli",
            "command -v docker >/dev/null 2>&1 && docker --version",
        )
        .await?,
    );
    checks.push(
        environment_command_check(
            workspace,
            "colima",
            "command -v colima >/dev/null 2>&1 && colima version | head -n 1",
        )
        .await?,
    );

    let docker_daemon = environment_command_check(
        workspace,
        "docker_daemon",
        "docker info --format '{{.ServerVersion}}'",
    )
    .await?;
    let docker_daemon_available = docker_daemon.available;
    checks.push(docker_daemon);

    if target == "compiler" || workspace.join("online-doc/docs").exists() {
        let compiler_image = if docker_daemon_available {
            environment_command_check(
                workspace,
                "compiler_dev_image",
                "docker image inspect maxxing/compiler-dev --format '{{.Id}}'",
            )
            .await?
        } else {
            EnvironmentCheck {
                name: "compiler_dev_image".to_string(),
                available: false,
                version: None,
                detail: Some("docker daemon is not running".to_string()),
            }
        };
        checks.push(compiler_image);
    }

    let ready = environment_ready(target, &checks);
    let recommended_action = environment_recommendation(target, &checks, ready);
    Ok(EnvironmentReport {
        target: target.to_string(),
        ready,
        checks,
        recommended_action,
    })
}

async fn setup_environment_in(
    workspace: &Path,
    target: &str,
    install_missing: bool,
    smoke_test: bool,
) -> Result<EnvironmentSetupResult> {
    let before = check_environment_in(workspace, target).await?;
    let mut actions = Vec::new();

    if install_missing
        && (!check_available(&before, "docker_cli") || !check_available(&before, "colima"))
    {
        if !check_available(&before, "homebrew") {
            bail!(
                "Homebrew is required for automated Docker/Colima setup on macOS; install Homebrew or configure Docker manually"
            );
        }
        let output = run_environment_action(
            workspace,
            "HOMEBREW_NO_AUTO_UPDATE=1 brew install docker colima",
            Duration::from_secs(1800),
        )
        .await?;
        let succeeded = output.exit_code == Some(0);
        actions.push(output);
        if !succeeded {
            let after = check_environment_in(workspace, target).await?;
            let ready = environment_setup_ready(after.ready, &actions);
            return Ok(EnvironmentSetupResult {
                target: target.to_string(),
                before,
                ready,
                after,
                actions,
            });
        }
    }

    let after_install = check_environment_in(workspace, target).await?;
    if check_available(&after_install, "docker_cli")
        && check_available(&after_install, "colima")
        && !check_available(&after_install, "docker_daemon")
    {
        let output = run_environment_action(
            workspace,
            "colima start --cpu 4 --memory 6 --disk 60 --mount-inotify=false",
            Duration::from_secs(1800),
        )
        .await?;
        let succeeded = output.exit_code == Some(0);
        actions.push(output);
        if !succeeded {
            let after = check_environment_in(workspace, target).await?;
            let ready = environment_setup_ready(after.ready, &actions);
            return Ok(EnvironmentSetupResult {
                target: target.to_string(),
                before,
                ready,
                after,
                actions,
            });
        }
    }

    let after_start = check_environment_in(workspace, target).await?;
    if target == "compiler"
        && check_available(&after_start, "docker_daemon")
        && !check_available(&after_start, "compiler_dev_image")
    {
        let output = run_environment_action(
            workspace,
            compiler_image_pull_command(),
            Duration::from_secs(1800),
        )
        .await?;
        let succeeded = output.exit_code == Some(0);
        actions.push(output);
        if !succeeded {
            let after = check_environment_in(workspace, target).await?;
            let ready = environment_setup_ready(after.ready, &actions);
            return Ok(EnvironmentSetupResult {
                target: target.to_string(),
                before,
                ready,
                after,
                actions,
            });
        }
    }

    if smoke_test {
        let smoke_command = if target == "compiler" {
            "docker run --rm maxxing/compiler-dev sh -lc 'command -v autotest >/dev/null && autotest --help >/dev/null 2>&1'"
        } else {
            "docker run --rm hello-world"
        };
        actions.push(
            run_environment_action(workspace, smoke_command, Duration::from_secs(600)).await?,
        );
    }

    let after = check_environment_in(workspace, target).await?;
    let ready = environment_setup_ready(after.ready, &actions);
    Ok(EnvironmentSetupResult {
        target: target.to_string(),
        before,
        ready,
        after,
        actions,
    })
}

fn environment_setup_ready(after_ready: bool, actions: &[CommandOutput]) -> bool {
    after_ready && actions.iter().all(|action| action.exit_code == Some(0))
}

fn compiler_image_pull_command() -> &'static str {
    "docker pull maxxing/compiler-dev || (docker pull docker.1ms.run/maxxing/compiler-dev && docker tag docker.1ms.run/maxxing/compiler-dev:latest maxxing/compiler-dev:latest) || (docker pull docker.m.daocloud.io/maxxing/compiler-dev && docker tag docker.m.daocloud.io/maxxing/compiler-dev:latest maxxing/compiler-dev:latest)"
}

async fn run_environment_action(
    workspace: &Path,
    command: &str,
    timeout_duration: Duration,
) -> Result<CommandOutput> {
    run_command_with_timeout(workspace, command, timeout_duration).await
}

async fn environment_command_check(
    workspace: &Path,
    name: &str,
    command: &str,
) -> Result<EnvironmentCheck> {
    let output = run_command_with_timeout(workspace, command, Duration::from_secs(30)).await?;
    let available = output.exit_code == Some(0);
    let text = output_text(&output);
    let trimmed = text.trim();
    Ok(EnvironmentCheck {
        name: name.to_string(),
        available,
        version: available
            .then(|| {
                trimmed
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .filter(|value| !value.is_empty()),
        detail: (!available && !trimmed.is_empty()).then(|| trimmed.to_string()),
    })
}

fn environment_target_arg(args: &Value) -> Result<String> {
    let target = args
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .trim();
    match target {
        "" | "auto" => Ok("auto".to_string()),
        "docker" | "compiler" => Ok(target.to_string()),
        other => bail!("unsupported environment target `{other}`"),
    }
}

fn check_available(report: &EnvironmentReport, name: &str) -> bool {
    report
        .checks
        .iter()
        .any(|check| check.name == name && check.available)
}

fn environment_ready(target: &str, checks: &[EnvironmentCheck]) -> bool {
    let available = |name: &str| {
        checks
            .iter()
            .any(|check| check.name == name && check.available)
    };
    match target {
        "compiler" => {
            available("docker_cli")
                && available("colima")
                && available("docker_daemon")
                && available("compiler_dev_image")
        }
        "docker" | "auto" => available("docker_cli") && available("docker_daemon"),
        _ => false,
    }
}

fn environment_recommendation(
    target: &str,
    checks: &[EnvironmentCheck],
    ready: bool,
) -> Option<String> {
    if ready {
        return None;
    }
    let available = |name: &str| {
        checks
            .iter()
            .any(|check| check.name == name && check.available)
    };
    if !available("homebrew") {
        return Some("install Homebrew or configure Docker manually".to_string());
    }
    if !available("docker_cli") || !available("colima") || !available("docker_daemon") {
        return Some("/setup docker --smoke".to_string());
    }
    if target == "compiler" && !available("compiler_dev_image") {
        return Some("/setup compiler --smoke".to_string());
    }
    Some("/env check".to_string())
}

pub fn resolve_workspace_path(workspace: &Path, raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("path traversal is not allowed: {raw}");
    }
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    if !resolved.starts_with(workspace) {
        bail!("path is outside workspace: {}", resolved.display());
    }
    Ok(resolved)
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

fn output_text(output: &CommandOutput) -> String {
    let mut text = String::new();
    if !output.stdout.is_empty() {
        text.push_str(&output.stdout);
    }
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&output.stderr);
    }
    text
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

fn normalize_patch_input(patch: &str, path: Option<&str>) -> String {
    let patch = if patch.trim_start().starts_with("@@") {
        if let Some(path) = path {
            format!("--- a/{path}\n+++ b/{path}\n{patch}")
        } else {
            patch.to_string()
        }
    } else {
        patch.to_string()
    };
    normalize_unified_diff_hunk_counts(&patch)
}

fn normalize_unified_diff_hunk_counts(patch: &str) -> String {
    let mut output = Vec::new();
    let mut active_hunk: Option<HunkAccumulator> = None;

    for line in patch.lines() {
        if let Some(header) = parse_hunk_header(line) {
            if let Some(hunk) = active_hunk.take() {
                output.extend(hunk.into_lines());
            }
            active_hunk = Some(HunkAccumulator {
                header,
                body: Vec::new(),
            });
            continue;
        }

        if let Some(hunk) = active_hunk.as_mut() {
            if line.is_empty() {
                hunk.body.push(" ".to_string());
            } else {
                hunk.body.push(line.to_string());
            }
        } else {
            output.push(line.to_string());
        }
    }

    if let Some(hunk) = active_hunk {
        output.extend(hunk.into_lines());
    }

    let mut normalized = output.join("\n");
    if patch.ends_with('\n') {
        normalized.push('\n');
    }
    normalized
}

#[derive(Debug)]
struct HunkAccumulator {
    header: HunkHeader,
    body: Vec<String>,
}

impl HunkAccumulator {
    fn into_lines(self) -> Vec<String> {
        let mut old_count = 0usize;
        let mut new_count = 0usize;
        for line in &self.body {
            match line.as_bytes().first().copied() {
                Some(b' ') => {
                    old_count += 1;
                    new_count += 1;
                }
                Some(b'-') => old_count += 1,
                Some(b'+') => new_count += 1,
                _ => {}
            }
        }
        let mut lines = vec![format!(
            "@@ -{},{} +{},{} @@{}",
            self.header.old_start, old_count, self.header.new_start, new_count, self.header.suffix
        )];
        lines.extend(self.body);
        lines
    }
}

#[derive(Debug)]
struct HunkHeader {
    old_start: usize,
    new_start: usize,
    suffix: String,
}

fn parse_hunk_header(line: &str) -> Option<HunkHeader> {
    let rest = line.strip_prefix("@@ -")?;
    let (old_start, rest) = parse_hunk_start(rest)?;
    let rest = rest.strip_prefix(" +")?;
    let (new_start, rest) = parse_hunk_start(rest)?;
    let suffix = rest.strip_prefix(" @@")?;
    Some(HunkHeader {
        old_start,
        new_start,
        suffix: suffix.to_string(),
    })
}

fn parse_hunk_start(input: &str) -> Option<(usize, &str)> {
    let digits = input
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    let start = input[..digits].parse::<usize>().ok()?;
    let rest = &input[digits..];
    if let Some(rest) = rest.strip_prefix(',') {
        let count_digits = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if count_digits == 0 {
            return None;
        }
        Some((start, &rest[count_digits..]))
    } else {
        Some((start, rest))
    }
}

fn reject_placeholder_overwrite(path: &Path, before: &str, content: &str) -> Result<()> {
    if before.len() < 1024 || content.len() * 10 >= before.len() {
        return Ok(());
    }
    if !looks_like_placeholder_content(content) {
        return Ok(());
    }
    bail!(
        "refusing to overwrite existing large file {} with placeholder-like content",
        path.display()
    )
}

fn reject_large_destructive_rewrite(path: &Path, before: &str, content: &str) -> Result<()> {
    if before.len() < 8 * 1024 || content.len() * 100 >= before.len() * 80 {
        return Ok(());
    }
    bail!(
        "refusing to overwrite existing large file {} with much shorter content; use a unified diff patch instead",
        path.display()
    )
}

fn reject_large_existing_rewrite(path: &Path, before: &str, content: &str) -> Result<()> {
    if before.len() < 8 * 1024 || before == content {
        return Ok(());
    }
    bail!(
        "refusing to rewrite existing large file {} with full file content; use a unified diff patch instead",
        path.display()
    )
}

fn looks_like_placeholder_content(content: &str) -> bool {
    let normalized = content.trim().to_ascii_lowercase();
    let stripped = normalized
        .trim_start_matches("//")
        .trim_start_matches('#')
        .trim_start_matches("/*")
        .trim_end_matches("*/")
        .trim();
    stripped == "placeholder"
        || stripped == "todo"
        || stripped == "..."
        || stripped == "<omitted>"
        || stripped == "omitted"
        || stripped.contains("placeholder")
        || stripped.contains("content omitted")
}

fn slice_text_by_line(content: &str, start_line: usize, limit: Option<usize>) -> String {
    if start_line <= 1 && limit.is_none() {
        return content.to_string();
    }
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let start_index = start_line.saturating_sub(1).min(lines.len());
    let end_index = match limit {
        Some(limit) => start_index.saturating_add(limit).min(lines.len()),
        None => lines.len(),
    };
    let mut selected = lines[start_index..end_index].join("\n");
    if content.ends_with('\n') && end_index == lines.len() {
        selected.push('\n');
    }
    if start_index > 0 || end_index < lines.len() {
        format!(
            "[deepcli read_file slice: lines {}-{} of {}]\n{}",
            start_index + 1,
            end_index,
            lines.len(),
            selected
        )
    } else {
        selected
    }
}

fn format_discovered_test_command(command: &DiscoveredTestCommand) -> String {
    let mut line = command.command.clone();
    if command.requires_docker {
        let status = match command.available {
            Some(true) => "available",
            Some(false) => "missing",
            None => "unknown",
        };
        line.push_str(&format!(" # requires docker: {status}"));
    }
    if let Some(note) = &command.note {
        line.push_str(&format!(" ({note})"));
    }
    line
}

fn format_environment_report(report: &EnvironmentReport) -> String {
    let mut lines = vec![format!(
        "environment target: {}\nready: {}",
        report.target, report.ready
    )];
    for check in &report.checks {
        let status = if check.available { "ok" } else { "missing" };
        let mut line = format!("- {}: {}", check.name, status);
        if let Some(version) = &check.version {
            line.push_str(&format!(" ({version})"));
        }
        if let Some(detail) = &check.detail {
            line.push_str(&format!(" - {}", first_line(detail)));
        }
        lines.push(line);
    }
    if let Some(action) = &report.recommended_action {
        lines.push(format!(
            "recommended: {}",
            environment_action_shortcut(action)
        ));
    }
    append_environment_next_actions(&mut lines, report);
    lines.join("\n")
}

fn format_environment_setup(setup: &EnvironmentSetupResult) -> String {
    let mut lines = vec![
        format!("environment setup target: {}", setup.target),
        format!("ready before: {}", setup.before.ready),
        format!("actions: {}", setup.actions.len()),
    ];
    for action in &setup.actions {
        let passed = action.exit_code == Some(0);
        lines.push(format!(
            "- [{}] {}",
            if passed { "ok" } else { "failed" },
            action.command
        ));
        let text = output_text(action);
        if !text.trim().is_empty() {
            lines.push(format!("  {}", first_line(&text)));
        }
    }
    lines.push(format!("ready after: {}", setup.ready));
    if let Some(action) = &setup.after.recommended_action {
        lines.push(format!(
            "recommended: {}",
            environment_action_shortcut(action)
        ));
    }
    append_environment_next_actions(&mut lines, &setup.after);
    lines.join("\n")
}

fn append_environment_next_actions(lines: &mut Vec<String>, report: &EnvironmentReport) {
    let actions = environment_report_next_actions(report);
    if actions.is_empty() {
        return;
    }
    lines.push("next:".to_string());
    lines.extend(actions.into_iter().map(|action| format!("  - {action}")));
}

fn environment_report_next_actions(report: &EnvironmentReport) -> Vec<String> {
    if report.ready {
        let target = environment_followup_target(&report.target);
        return vec![
            format!("run `/env test {target} --json` to capture smoke-test evidence"),
            "run `/test discover --json` to inspect project test commands".to_string(),
        ];
    }

    let mut actions = Vec::new();
    if let Some(action) = &report.recommended_action {
        let action = environment_action_shortcut(action);
        if action.starts_with('/') {
            actions.push(format!("run `{action}` to continue environment setup"));
        } else {
            actions.push(action);
        }
    }
    let target = environment_followup_target(&report.target);
    actions.push(format!(
        "preview setup first with `/env plan {target} --smoke --json`"
    ));
    dedup_environment_actions(actions)
}

fn environment_followup_target(target: &str) -> &str {
    if target == "compiler" {
        "compiler"
    } else {
        "docker"
    }
}

fn dedup_environment_actions(actions: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for action in actions {
        if !deduped.contains(&action) {
            deduped.push(action);
        }
    }
    deduped
}

fn environment_action_shortcut(command: &str) -> String {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    let target = match parts.as_slice() {
        ["/env", "setup", target, ..] => *target,
        ["/setup", target, ..] => *target,
        _ => return command.to_string(),
    };
    if matches!(target, "docker" | "compiler") {
        format!("/setup {target} --smoke")
    } else {
        command.to_string()
    }
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

fn unified_diff(before: &str, after: &str, path: &Path) -> String {
    TextDiff::from_lines(before, after)
        .unified_diff()
        .header(
            &format!("a/{}", path.display()),
            &format!("b/{}", path.display()),
        )
        .to_string()
}

fn validate_branch_name(name: &str) -> Result<()> {
    if name.starts_with('-')
        || name.contains("..")
        || name.contains('@')
        || name.contains('\\')
        || name.contains(' ')
        || name.trim().is_empty()
    {
        bail!("invalid branch name `{name}`");
    }
    Ok(())
}

fn generate_commit_message(status: &str, changed_files: &str) -> String {
    let files = changed_files
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let status_lines = status
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let file_count = files.len().max(status_lines.len());

    if file_count == 0 {
        return "chore: record workspace state".to_string();
    }

    let joined = files
        .iter()
        .chain(status_lines.iter())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let scope = if joined.contains("test") {
        "test"
    } else if joined.contains("doc") || joined.contains("README") {
        "docs"
    } else if joined.contains("Cargo.toml") || joined.contains("Cargo.lock") {
        "build"
    } else {
        "cli"
    };

    let verb = if status_lines.iter().any(|line| line.starts_with("A ")) {
        "add"
    } else if status_lines.iter().any(|line| line.starts_with("D ")) {
        "remove"
    } else {
        "update"
    };

    format!(
        "{scope}: {verb} {file_count} workspace file{}",
        if file_count == 1 { "" } else { "s" }
    )
}

fn format_web_search_result(query: &str, value: &Value) -> String {
    let mut lines = Vec::new();
    let heading = value
        .get("Heading")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());
    let answer = value
        .get("Answer")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());
    let abstract_text = value
        .get("AbstractText")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());
    let abstract_url = value
        .get("AbstractURL")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty());

    if let Some(heading) = heading {
        lines.push(format!("heading: {}", heading.trim()));
    }
    if let Some(answer) = answer {
        lines.push(format!("answer: {}", answer.trim()));
    }
    if let Some(abstract_text) = abstract_text {
        lines.push(format!("summary: {}", abstract_text.trim()));
    }
    if let Some(abstract_url) = abstract_url {
        lines.push(format!("source: {}", abstract_url.trim()));
    }

    let related = collect_web_related_topics(value, 5);
    if !related.is_empty() {
        lines.push("related:".to_string());
        for (text, url) in related {
            match url {
                Some(url) if !url.trim().is_empty() => {
                    lines.push(format!("  - {} ({})", text.trim(), url.trim()))
                }
                _ => lines.push(format!("  - {}", text.trim())),
            }
        }
    }

    if lines.is_empty() {
        format!("no web search summary found for `{query}`")
    } else {
        lines.join("\n")
    }
}

fn collect_web_related_topics(value: &Value, limit: usize) -> Vec<(String, Option<String>)> {
    let mut results = Vec::new();
    if let Some(topics) = value.get("RelatedTopics") {
        collect_web_related_topic_values(topics, limit, &mut results);
    }
    results
}

fn collect_web_related_topic_values(
    value: &Value,
    limit: usize,
    results: &mut Vec<(String, Option<String>)>,
) {
    if results.len() >= limit {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_web_related_topic_values(item, limit, results);
                if results.len() >= limit {
                    break;
                }
            }
        }
        Value::Object(map) => {
            if let Some(text) = map
                .get("Text")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
            {
                let url = map
                    .get("FirstURL")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                results.push((text.to_string(), url));
                return;
            }
            if let Some(topics) = map.get("Topics") {
                collect_web_related_topic_values(topics, limit, results);
            }
        }
        _ => {}
    }
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
            recommended_action: Some("/env setup compiler".to_string()),
        };
        let text = format_environment_report(&report);
        assert!(text.contains("compiler_dev_image: missing"));
        assert!(text.contains("recommended: /setup compiler --smoke"));
        assert!(text.contains("next:"));
        assert!(text.contains("run `/setup compiler --smoke` to continue environment setup"));
        assert!(text.contains("preview setup first with `/env plan compiler --smoke --json`"));
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

    #[test]
    fn generates_commit_message_from_changed_files() {
        let message = generate_commit_message("A  src/main.rs\n", "src/main.rs\nCargo.toml\n");
        assert_eq!(message, "build: add 2 workspace files");
        let docs =
            generate_commit_message("M  docs/ai/REQUIREMENTS.md\n", "docs/ai/REQUIREMENTS.md\n");
        assert_eq!(docs, "docs: update 1 workspace file");
    }

    #[test]
    fn formats_web_search_result_with_related_topic_fallback() {
        let value = json!({
            "Heading": "",
            "AbstractText": "",
            "RelatedTopics": [
                {
                    "Text": "Rust is a systems programming language.",
                    "FirstURL": "https://duckduckgo.com/Rust"
                },
                {
                    "Topics": [
                        {
                            "Text": "Ownership is Rust's memory model.",
                            "FirstURL": "https://duckduckgo.com/Ownership"
                        }
                    ]
                }
            ]
        });
        let output = format_web_search_result("rust ownership", &value);
        assert!(output.contains("related:"));
        assert!(output.contains("Rust is a systems programming language."));
        assert!(output.contains("Ownership is Rust's memory model."));
    }

    #[test]
    fn formats_web_search_empty_result_with_query() {
        let output = format_web_search_result("unknown", &json!({}));
        assert_eq!(output, "no web search summary found for `unknown`");
    }
}
