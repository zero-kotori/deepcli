use deepcli::commands::CommandRouter;
use deepcli::config::AppConfig;
use deepcli::tools::ToolRegistry;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[test]
fn mvp_slash_commands_are_registered() {
    let help = CommandRouter::help_text();
    for command in [
        "/help",
        "/version",
        "/about",
        "/quickstart",
        "/recipes",
        "/scorecard",
        "/benchmark",
        "/round",
        "/selftest",
        "/preflight",
        "/completion",
        "/init",
        "/status",
        "/usage",
        "/health",
        "/diagnose",
        "/support",
        "/doctor",
        "/trace",
        "/logs",
        "/privacy",
        "/context",
        "/permissions",
        "/login",
        "/auth",
        "/apikey",
        "/key",
        "/logout",
        "/credentials",
        "/config",
        "/timeout",
        "/model",
        "/provider",
        "/use",
        "/switch",
        "/models",
        "/providers",
        "/goal",
        "/plan",
        "/fork",
        "/diff",
        "/review",
        "/verify",
        "/handoff",
        "/test",
        "/env",
        "/check",
        "/docker",
        "/compiler",
        "/setup",
        "/install",
        "/git",
        "/web",
        "/prompt",
        "/skill",
        "/agent",
        "/btw",
        "/approval",
        "/session",
        "/history",
        "/next",
        "/resume",
        "/rename",
        "/stop",
        "/terminal",
    ] {
        assert!(help.contains(command), "{command} missing from help text");
    }
}

#[test]
fn mvp_tool_registry_exposes_required_tools() {
    let registry = ToolRegistry::mvp();
    for tool in [
        "read_file",
        "list_files",
        "search",
        "write_file",
        "apply_patch_or_write",
        "run_shell",
        "git_status",
        "git_diff",
        "git_branch",
        "git_create_branch",
        "git_commit_message",
        "git_commit",
        "discover_tests",
        "run_tests",
        "check_environment",
        "setup_environment",
        "web_search",
        "open_terminal",
        "prompt_list",
        "prompt_get",
        "prompt_render",
        "skill_list",
        "skill_generate",
        "skill_run",
        "spawn_subagent",
    ] {
        assert!(registry.has(tool), "{tool} missing from registry");
    }
    assert_eq!(registry.tool_specs().len(), registry.declarations().len());
}

#[test]
fn default_config_matches_documented_mvp_defaults() {
    let config = AppConfig::default();
    assert_eq!(config.default_provider, "deepseek");
    assert_eq!(config.permissions.default_mode, "sandbox");
    assert!(config.sandbox.enabled_by_default);
    assert!(config.agent.require_plan_for_complex_tasks);
    assert_eq!(config.agent.max_subagent_depth, 2);
    assert_eq!(config.agent.max_tool_iterations, 64);
    assert_eq!(config.agent.provider_turn_timeout_seconds, 600);
    assert!(config
        .providers
        .get("deepseek")
        .unwrap()
        .capabilities
        .contains(&"tool_calling".to_string()));
}

#[test]
fn architecture_harness_docs_cover_commands_and_modules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let harness = fs::read_to_string(root.join("docs/HARNESS.md")).unwrap();
    for section in [
        "## Module Map",
        "## Boundary Principles",
        "## Documentation Sync",
        "## Verification",
    ] {
        assert!(
            harness.contains(section),
            "docs/HARNESS.md missing section {section}"
        );
    }

    let commands = fs::read_to_string(root.join("docs/COMMANDS.md")).unwrap();
    let documented_commands = documented_command_groups(&commands);
    let registered_commands = CommandRouter::command_names()
        .into_iter()
        .collect::<BTreeSet<_>>();
    for command in &registered_commands {
        assert!(
            documented_commands.contains_key(*command),
            "docs/COMMANDS.md missing registered command {command}"
        );
    }
    for command in documented_commands.keys() {
        assert!(
            registered_commands.contains(command.as_str()),
            "docs/COMMANDS.md documents unknown command {command}"
        );
    }

    for core_command in [
        "/goal",
        "/plan",
        "/fork",
        "/session",
        "/resume",
        "/git",
        "/test",
        "/tools",
        "/round",
        "/scorecard",
        "/preflight",
        "/gate",
    ] {
        if registered_commands.contains(core_command) {
            assert_eq!(
                documented_commands.get(core_command).map(String::as_str),
                Some("core"),
                "{core_command} should be documented as core"
            );
        }
    }

    for module in [
        "commands",
        "runtime",
        "tools",
        "session",
        "permissions",
        "ui",
    ] {
        let path = root.join("docs/MODULES").join(format!("{module}.md"));
        let contents = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for section in [
            "## Responsibility",
            "## Boundaries",
            "## Tests",
            "## Documentation Sync",
        ] {
            assert!(
                contents.contains(section),
                "{} missing section {section}",
                path.display()
            );
        }
    }
}

fn documented_command_groups(contents: &str) -> BTreeMap<String, String> {
    let valid_groups = ["core", "support", "legacy", "experimental"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut commands = BTreeMap::new();
    for line in contents.lines() {
        if !line.starts_with("| /") {
            continue;
        }
        let cells = line
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        if cells.len() < 2 {
            continue;
        }
        let command = cells[0].to_string();
        let group = cells[1].to_string();
        assert!(
            valid_groups.contains(group.as_str()),
            "{command} uses invalid command group {group}"
        );
        assert!(
            commands.insert(command.clone(), group).is_none(),
            "{command} documented more than once"
        );
    }
    commands
}
