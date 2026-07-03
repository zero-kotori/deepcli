use crate::config::{PermissionConfig, SandboxConfig};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Read,
    Write,
    FullControl,
    Sandbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    DoubleConfirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    Allowed,
    AutoApproved,
    RequiresUserApproval,
    DoubleConfirmRequired,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDecision {
    pub outcome: DecisionOutcome,
    pub risk: RiskLevel,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSurface {
    Filesystem,
    Shell,
    Git,
    Network,
    Docker,
    Provider,
    Session,
    Skill,
    Subagent,
    Terminal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequest {
    pub tool: String,
    pub surface: ToolSurface,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub network_target: Option<String>,
    #[serde(default)]
    pub writes_files: bool,
    #[serde(default)]
    pub creates_process: bool,
    #[serde(default)]
    pub requires_network: bool,
    #[serde(default)]
    pub explicit_approval: bool,
}

#[derive(Debug, Clone)]
pub struct PermissionEngine {
    workspace_root: PathBuf,
    permissions: PermissionConfig,
    sandbox: SandboxConfig,
    mode: PermissionMode,
}

impl PermissionEngine {
    pub fn new(
        workspace_root: impl AsRef<Path>,
        permissions: PermissionConfig,
        sandbox: SandboxConfig,
    ) -> Self {
        let mode = parse_permission_mode(&permissions.default_mode);
        Self {
            workspace_root: workspace_root.as_ref().to_path_buf(),
            permissions,
            sandbox,
            mode,
        }
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    pub fn evaluate(&self, request: &ToolRequest) -> PermissionDecision {
        let risk = self.classify(request);

        if risk == RiskLevel::DoubleConfirm {
            return PermissionDecision {
                outcome: if request.explicit_approval && self.sandbox.allow_dangerous_commands {
                    DecisionOutcome::Allowed
                } else {
                    DecisionOutcome::DoubleConfirmRequired
                },
                risk,
                reason: "operation matches a double-confirmation risk rule".to_string(),
            };
        }

        if !self.path_within_workspace(request) {
            return PermissionDecision {
                outcome: DecisionOutcome::Denied,
                risk: RiskLevel::High,
                reason: "path is outside the authorized workspace".to_string(),
            };
        }

        if self.sandbox.enabled_by_default {
            if let Some(decision) = self.evaluate_sandbox(request, risk) {
                return decision;
            }
        }

        match self.mode {
            PermissionMode::Read if request.writes_files || request.creates_process => {
                PermissionDecision {
                    outcome: DecisionOutcome::RequiresUserApproval,
                    risk,
                    reason: "read mode does not allow writes or process execution".to_string(),
                }
            }
            PermissionMode::FullControl if risk < RiskLevel::High => PermissionDecision {
                outcome: DecisionOutcome::Allowed,
                risk,
                reason: "full_control mode allows this non-dangerous operation".to_string(),
            },
            PermissionMode::FullControl => PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk,
                reason: "high-risk operations still require approval in full_control mode"
                    .to_string(),
            },
            _ if risk == RiskLevel::Low => PermissionDecision {
                outcome: DecisionOutcome::Allowed,
                risk,
                reason: "low-risk operation is allowed by current policy".to_string(),
            },
            _ if self.permissions.approval_policy == "auto_reviewer_then_user"
                && risk == RiskLevel::Medium =>
            {
                PermissionDecision {
                    outcome: DecisionOutcome::AutoApproved,
                    risk,
                    reason: "auto-reviewer approved a medium-risk sandbox escalation".to_string(),
                }
            }
            _ => PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk,
                reason: "operation requires approval under current policy".to_string(),
            },
        }
    }

    pub fn classify(&self, request: &ToolRequest) -> RiskLevel {
        if let Some(command) = &request.command {
            let normalized = command.trim().to_ascii_lowercase();
            if self
                .permissions
                .dangerous_command_patterns
                .iter()
                .any(|pattern| normalized.contains(&pattern.to_ascii_lowercase()))
                || is_destructive_shell(&normalized)
            {
                return RiskLevel::DoubleConfirm;
            }

            if is_git_destructive(&normalized) {
                return RiskLevel::DoubleConfirm;
            }

            if is_package_install(&normalized)
                || normalized.contains("docker run")
                || normalized.contains("docker pull")
                || normalized.contains("chmod ")
                || normalized.contains("chown ")
            {
                return RiskLevel::High;
            }

            if is_read_only_shell(&normalized) {
                return RiskLevel::Low;
            }

            if is_test_or_build_shell(&normalized) {
                return RiskLevel::Medium;
            }
        }

        match request.surface {
            ToolSurface::Filesystem if request.writes_files => RiskLevel::High,
            ToolSurface::Filesystem => RiskLevel::Low,
            ToolSurface::Git if request.writes_files => RiskLevel::High,
            ToolSurface::Git => RiskLevel::Low,
            ToolSurface::Network | ToolSurface::Provider => {
                if self.sandbox.allow_network {
                    RiskLevel::Medium
                } else {
                    RiskLevel::High
                }
            }
            ToolSurface::Session => RiskLevel::Low,
            ToolSurface::Docker => RiskLevel::High,
            ToolSurface::Shell if request.creates_process => RiskLevel::Medium,
            ToolSurface::Skill | ToolSurface::Subagent => RiskLevel::Medium,
            ToolSurface::Terminal => RiskLevel::Low,
            ToolSurface::Shell => RiskLevel::Low,
        }
    }

    fn evaluate_sandbox(
        &self,
        request: &ToolRequest,
        risk: RiskLevel,
    ) -> Option<PermissionDecision> {
        if matches!(
            request.surface,
            ToolSurface::Network | ToolSurface::Provider
        ) && !self.sandbox.allow_network
        {
            return Some(PermissionDecision {
                outcome: DecisionOutcome::RequiresUserApproval,
                risk,
                reason: "sandbox network access is disabled".to_string(),
            });
        }

        if request.writes_files
            && !self.sandbox.allow_system_write
            && !self.path_within_workspace(request)
        {
            return Some(PermissionDecision {
                outcome: DecisionOutcome::Denied,
                risk: RiskLevel::High,
                reason: "sandbox denied write outside workspace".to_string(),
            });
        }

        if risk == RiskLevel::Low {
            return Some(PermissionDecision {
                outcome: DecisionOutcome::Allowed,
                risk,
                reason: "sandbox permits low-risk operation".to_string(),
            });
        }

        if risk == RiskLevel::Medium
            && self.permissions.approval_policy == "auto_reviewer_then_user"
        {
            return Some(PermissionDecision {
                outcome: DecisionOutcome::AutoApproved,
                risk,
                reason: "auto-reviewer approved sandbox escalation".to_string(),
            });
        }

        None
    }

    fn path_within_workspace(&self, request: &ToolRequest) -> bool {
        let Some(path) = &request.path else {
            return true;
        };
        if path.is_relative() {
            return true;
        }
        path.starts_with(&self.workspace_root)
    }
}

fn parse_permission_mode(value: &str) -> PermissionMode {
    match value {
        "read" => PermissionMode::Read,
        "write" => PermissionMode::Write,
        "full_control" => PermissionMode::FullControl,
        _ => PermissionMode::Sandbox,
    }
}

fn is_read_only_shell(command: &str) -> bool {
    let Ok(parts) = shell_words::split(command) else {
        return false;
    };
    matches!(
        parts.first().map(String::as_str),
        Some("ls")
            | Some("pwd")
            | Some("rg")
            | Some("grep")
            | Some("sed")
            | Some("cat")
            | Some("head")
            | Some("tail")
            | Some("find")
            | Some("git")
    ) && !command.contains("git commit")
        && !command.contains("git checkout")
        && !command.contains("git switch")
        && !command.contains("git reset")
        && !command.contains("git clean")
}

fn is_test_or_build_shell(command: &str) -> bool {
    let prefixes = [
        "cargo test",
        "cargo check",
        "cargo build",
        "npm test",
        "npm run",
        "pnpm test",
        "pnpm run",
        "yarn test",
        "pytest",
        "python -m pytest",
        "make test",
        "go test",
    ];
    prefixes.iter().any(|prefix| command.starts_with(prefix))
}

fn is_package_install(command: &str) -> bool {
    let patterns = [
        "brew install",
        "cargo install",
        "npm install",
        "pnpm install",
        "yarn add",
        "pip install",
        "uv pip install",
        "apt install",
    ];
    patterns.iter().any(|pattern| command.starts_with(pattern))
}

fn is_destructive_shell(command: &str) -> bool {
    command.contains("rm -rf")
        || command.contains("rm -fr")
        || command.starts_with("rm -r ")
        || command.starts_with("rm -f /")
}

fn is_git_destructive(command: &str) -> bool {
    command.contains("git reset --hard")
        || command.contains("git clean -fd")
        || command.contains("git push --force")
        || command.contains("git branch -d")
        || command.contains("git branch -D")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PermissionConfig, SandboxConfig};

    fn engine() -> PermissionEngine {
        PermissionEngine::new(
            "/tmp/project",
            PermissionConfig::default(),
            SandboxConfig::default(),
        )
    }

    #[test]
    fn read_only_shell_is_allowed() {
        let request = ToolRequest {
            tool: "run_shell".to_string(),
            surface: ToolSurface::Shell,
            command: Some("rg TODO src".to_string()),
            path: None,
            network_target: None,
            writes_files: false,
            creates_process: true,
            requires_network: false,
            explicit_approval: false,
        };
        let decision = engine().evaluate(&request);
        assert_eq!(decision.outcome, DecisionOutcome::Allowed);
        assert_eq!(decision.risk, RiskLevel::Low);
    }

    #[test]
    fn destructive_shell_requires_double_confirmation() {
        let request = ToolRequest {
            tool: "run_shell".to_string(),
            surface: ToolSurface::Shell,
            command: Some("rm -rf target".to_string()),
            path: None,
            network_target: None,
            writes_files: true,
            creates_process: true,
            requires_network: false,
            explicit_approval: false,
        };
        let decision = engine().evaluate(&request);
        assert_eq!(decision.outcome, DecisionOutcome::DoubleConfirmRequired);
        assert_eq!(decision.risk, RiskLevel::DoubleConfirm);
    }

    #[test]
    fn medium_risk_shell_can_be_auto_approved() {
        let request = ToolRequest {
            tool: "run_shell".to_string(),
            surface: ToolSurface::Shell,
            command: Some("cargo test".to_string()),
            path: None,
            network_target: None,
            writes_files: false,
            creates_process: true,
            requires_network: false,
            explicit_approval: false,
        };
        let decision = engine().evaluate(&request);
        assert_eq!(decision.outcome, DecisionOutcome::AutoApproved);
        assert_eq!(decision.risk, RiskLevel::Medium);
    }

    #[test]
    fn docker_commands_require_user_approval() {
        let request = ToolRequest {
            tool: "run_shell".to_string(),
            surface: ToolSurface::Docker,
            command: Some("docker run --rm rust:latest cargo test".to_string()),
            path: None,
            network_target: None,
            writes_files: false,
            creates_process: true,
            requires_network: true,
            explicit_approval: false,
        };
        let decision = engine().evaluate(&request);
        assert_eq!(decision.outcome, DecisionOutcome::RequiresUserApproval);
        assert_eq!(decision.risk, RiskLevel::High);
    }

    #[test]
    fn package_install_commands_require_user_approval() {
        let request = ToolRequest {
            tool: "run_shell".to_string(),
            surface: ToolSurface::Shell,
            command: Some("npm install".to_string()),
            path: None,
            network_target: None,
            writes_files: true,
            creates_process: true,
            requires_network: true,
            explicit_approval: false,
        };
        let decision = engine().evaluate(&request);
        assert_eq!(decision.outcome, DecisionOutcome::RequiresUserApproval);
        assert_eq!(decision.risk, RiskLevel::High);
    }

    #[test]
    fn docker_environment_setup_requires_user_approval() {
        let request = ToolRequest {
            tool: "setup_environment".to_string(),
            surface: ToolSurface::Docker,
            command: Some("deepcli environment setup docker".to_string()),
            path: Some(PathBuf::from("/tmp/project")),
            network_target: Some("ghcr.io, docker.io".to_string()),
            writes_files: true,
            creates_process: true,
            requires_network: true,
            explicit_approval: false,
        };
        let decision = engine().evaluate(&request);
        assert_eq!(decision.outcome, DecisionOutcome::RequiresUserApproval);
        assert_eq!(decision.risk, RiskLevel::High);
    }
}
