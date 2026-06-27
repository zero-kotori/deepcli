use super::schema::schema_for;
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
    pub explicit_approval: bool,
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
            explicit_approval: context.explicit_approval,
        }
    }
}

pub struct ToolRegistry {
    declarations: Vec<ToolDeclaration>,
}

impl ToolRegistry {
    pub fn mvp() -> Self {
        Self {
            declarations: vec![
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
                    "web_search",
                    "Run a privacy-filtered web search query.",
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
                    "Spawn a bounded sub-agent task descriptor.",
                    ToolSurface::Subagent,
                    true,
                    false,
                    true,
                ),
            ],
        }
    }

    pub fn declarations(&self) -> &[ToolDeclaration] {
        &self.declarations
    }

    pub fn declaration(&self, name: &str) -> Option<&ToolDeclaration> {
        self.declarations.iter().find(|tool| tool.name == name)
    }

    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.declarations
            .iter()
            .map(|declaration| ToolSpec {
                spec_type: "function".to_string(),
                function: ToolFunctionSpec {
                    name: declaration.name.clone(),
                    description: declaration.description.clone(),
                    parameters: declaration.parameters.clone(),
                },
            })
            .collect()
    }

    pub fn has(&self, name: &str) -> bool {
        self.declarations.iter().any(|tool| tool.name == name)
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
