use super::environment::docker_available;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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

pub(super) fn format_discovered_test_command(command: &DiscoveredTestCommand) -> String {
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
