use super::{schema::schema_for, validation::validate_tool_arguments};
use crate::permissions::{ToolRequest, ToolSurface};
use crate::providers::{ToolFunctionSpec, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolPermissionContext {
    pub command: Option<String>,
    pub path: Option<PathBuf>,
    pub network_target: Option<String>,
    pub writes_files: Option<bool>,
    pub creates_process: bool,
    pub requires_network: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDeclaration {
    pub name: String,
    pub description: String,
    pub surface: ToolSurface,
    pub parameters: Value,
    pub writes_files: bool,
    pub requires_network: bool,
    pub can_run_parallel: bool,
}

impl ToolDeclaration {
    pub fn validate_arguments(&self, args: &Value) -> anyhow::Result<()> {
        validate_tool_arguments(&self.name, args)
    }

    pub fn permission_request(&self, context: ToolPermissionContext) -> ToolRequest {
        ToolRequest {
            tool: self.name.clone(),
            surface: self.surface.clone(),
            command: context.command,
            path: context.path,
            network_target: context.network_target,
            writes_files: context.writes_files.unwrap_or(self.writes_files),
            creates_process: context.creates_process,
            requires_network: context.requires_network.unwrap_or(self.requires_network),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolObject {
    declaration: ToolDeclaration,
}

impl ToolObject {
    fn new(declaration: ToolDeclaration) -> Self {
        Self { declaration }
    }

    pub fn declaration(&self) -> &ToolDeclaration {
        &self.declaration
    }

    pub fn name(&self) -> &str {
        &self.declaration.name
    }

    pub fn validate_arguments(&self, args: &Value) -> anyhow::Result<()> {
        self.declaration.validate_arguments(args)
    }

    pub fn can_run_parallel(&self) -> bool {
        self.declaration.can_run_parallel
            && !self.declaration.writes_files
            && !self.declaration.requires_network
            && matches!(
                self.declaration.surface,
                ToolSurface::Filesystem | ToolSurface::Skill
            )
    }

    pub fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            spec_type: "function".to_string(),
            function: ToolFunctionSpec {
                name: self.declaration.name.clone(),
                description: self.declaration.description.clone(),
                parameters: self.declaration.parameters.clone(),
            },
        }
    }
}

pub struct ToolRegistry {
    declarations: Vec<ToolDeclaration>,
    tools: Vec<ToolObject>,
}

impl ToolRegistry {
    pub fn mvp() -> Self {
        let declarations = vec![
                declaration(
                    "read_file",
                    "Read a UTF-8 file inside the authorized workspace.",
                    ToolSurface::Filesystem,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "list_files",
                    "List non-ignored files inside the authorized workspace.",
                    ToolSurface::Filesystem,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "search",
                    "Search text in non-ignored workspace files.",
                    ToolSurface::Filesystem,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "write_file",
                    "Write a file and record a unified diff.",
                    ToolSurface::Filesystem,
                    true,
                    false,
                    false,
                ),
                declaration(
                    "apply_patch_or_write",
                    "Apply a full-file replacement and record a diff.",
                    ToolSurface::Filesystem,
                    true,
                    false,
                    false,
                ),
                declaration(
                    "run_shell",
                    "Run a sandboxed shell command with a bounded timeout.",
                    ToolSurface::Shell,
                    false,
                    false,
                    false,
                ),
                declaration(
                    "git_status",
                    "Show local git status.",
                    ToolSurface::Git,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "git_diff",
                    "Show local git diff.",
                    ToolSurface::Git,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "git_branch",
                    "Show the current branch and local branches.",
                    ToolSurface::Git,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "git_create_branch",
                    "Create and switch to a local branch after approval.",
                    ToolSurface::Git,
                    true,
                    false,
                    false,
                ),
                declaration(
                    "git_commit_message",
                    "Generate a local commit message from git status and diff statistics.",
                    ToolSurface::Git,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "git_commit",
                    "Create a local git commit after approval.",
                    ToolSurface::Git,
                    true,
                    false,
                    false,
                ),
                declaration(
                    "discover_tests",
                    "Discover likely project test commands.",
                    ToolSurface::Filesystem,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "run_tests",
                    "Run a discovered or explicit test command.",
                    ToolSurface::Shell,
                    false,
                    false,
                    false,
                ),
                declaration(
                    "check_environment",
                    "Check local tooling needed for project workflows, such as Docker and compiler autotests.",
                    ToolSurface::Shell,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "setup_environment",
                    "Install, configure, and verify approved local tooling such as Docker/Colima and compiler-dev.",
                    ToolSurface::Docker,
                    true,
                    true,
                    false,
                ),
                declaration(
                    "todo_write",
                    "Update the session todo plan with pending, in-progress, completed, or failed items.",
                    ToolSurface::Session,
                    false,
                    false,
                    false,
                ),
                declaration(
                    "ask_user_question",
                    "Queue a focused user question in the active session without interrupting tool execution.",
                    ToolSurface::Session,
                    false,
                    false,
                    false,
                ),
                declaration(
                    "web_search",
                    "Run a privacy-filtered web search query.",
                    ToolSurface::Network,
                    false,
                    true,
                    true,
                ),
                declaration(
                    "web_fetch",
                    "Fetch and extract text from an http or https URL with bounded output.",
                    ToolSurface::Network,
                    false,
                    true,
                    true,
                ),
                declaration(
                    "open_terminal",
                    "Open a new terminal in the current workspace.",
                    ToolSurface::Terminal,
                    false,
                    false,
                    false,
                ),
                declaration(
                    "prompt_list",
                    "List built-in and project prompts available to reuse.",
                    ToolSurface::Filesystem,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "prompt_get",
                    "Read a built-in or project prompt body by name.",
                    ToolSurface::Filesystem,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "prompt_render",
                    "Render a prompt with workspace, branch, diff, file, and custom variables.",
                    ToolSurface::Git,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "skill_list",
                    "List registered project Skills and their triggers.",
                    ToolSurface::Skill,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "skill_generate",
                    "Generate a local Skill skeleton.",
                    ToolSurface::Skill,
                    true,
                    false,
                    false,
                ),
                declaration(
                    "skill_run",
                    "Read and return a registered Skill instruction file.",
                    ToolSurface::Skill,
                    false,
                    false,
                    true,
                ),
                declaration(
                    "spawn_subagent",
                    "Spawn a bounded runnable sub-agent task, start it in the background when possible, and return lifecycle metadata.",
                    ToolSurface::Subagent,
                    true,
                    false,
                    false,
                ),
            ];
        let tools = declarations
            .iter()
            .cloned()
            .map(ToolObject::new)
            .collect::<Vec<_>>();
        Self {
            declarations,
            tools,
        }
    }

    pub fn declarations(&self) -> &[ToolDeclaration] {
        &self.declarations
    }

    pub fn tools(&self) -> &[ToolObject] {
        &self.tools
    }

    pub fn tool(&self, name: &str) -> Option<&ToolObject> {
        self.tools.iter().find(|tool| tool.name() == name)
    }

    pub fn declaration(&self, name: &str) -> Option<&ToolDeclaration> {
        self.tool(name).map(ToolObject::declaration)
    }

    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(ToolObject::tool_spec).collect()
    }

    pub fn tool_specs_for_names(&self, names: &[&str]) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .filter(|tool| names.iter().any(|name| *name == tool.name()))
            .map(ToolObject::tool_spec)
            .collect()
    }

    pub fn restrict_to_names(&mut self, names: &[String]) -> anyhow::Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        for name in names {
            if !self.has(name) {
                anyhow::bail!("sub-agent allowed_tools contains unknown tool `{name}`");
            }
        }
        self.declarations
            .retain(|declaration| names.iter().any(|name| name == &declaration.name));
        self.tools
            .retain(|tool| names.iter().any(|name| name == tool.name()));
        Ok(())
    }

    pub fn has(&self, name: &str) -> bool {
        self.tool(name).is_some()
    }
}

fn declaration(
    name: &str,
    description: &str,
    surface: ToolSurface,
    writes_files: bool,
    requires_network: bool,
    can_run_parallel: bool,
) -> ToolDeclaration {
    ToolDeclaration {
        name: name.to_string(),
        description: description.to_string(),
        surface,
        parameters: schema_for(name),
        writes_files,
        requires_network,
        can_run_parallel,
    }
}

#[cfg(test)]
mod tests {
    use super::ToolRegistry;

    #[test]
    fn restricted_registry_only_exposes_the_subagent_tool_capability() {
        let mut registry = ToolRegistry::mvp();
        registry
            .restrict_to_names(&["read_file".to_string(), "search".to_string()])
            .unwrap();

        let names = registry
            .tool_specs()
            .into_iter()
            .map(|tool| tool.function.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["read_file", "search"]);
        assert!(!registry.has("run_shell"));
    }

    #[test]
    fn parallel_tools_are_limited_to_scope_safe_local_reads() {
        let registry = ToolRegistry::mvp();

        assert!(registry.tool("read_file").unwrap().can_run_parallel());
        assert!(registry.tool("search").unwrap().can_run_parallel());
        assert!(!registry.tool("web_fetch").unwrap().can_run_parallel());
        assert!(!registry.tool("git_diff").unwrap().can_run_parallel());
        assert!(!registry
            .tool("check_environment")
            .unwrap()
            .can_run_parallel());
    }
}
