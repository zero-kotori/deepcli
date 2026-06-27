use deepcli::commands::{CommandGroup, CommandRouter};
use deepcli::config::AppConfig;
use deepcli::permissions::ToolSurface;
use deepcli::tools::{ToolPermissionContext, ToolRegistry};
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
fn command_registry_exposes_groups_and_drives_help_metadata() {
    let summaries = CommandRouter::help_summaries();
    let summary_names = summaries
        .iter()
        .map(|summary| summary.name)
        .collect::<Vec<_>>();
    assert_eq!(CommandRouter::command_names(), summary_names);

    for summary in &summaries {
        let topic = summary.name.trim_start_matches('/').to_string();
        let help = CommandRouter::help_for(&[topic]).unwrap();
        let expected_running_safe = if summary.running_safe {
            "running-safe: yes"
        } else {
            "running-safe: no"
        };
        assert!(
            help.contains(expected_running_safe),
            "{} help should reflect registry running-safe metadata",
            summary.name
        );
    }

    for (command, expected_group) in [
        ("/goal", CommandGroup::Core),
        ("/plan", CommandGroup::Core),
        ("/fork", CommandGroup::Core),
        ("/round", CommandGroup::Core),
        ("/benchmark", CommandGroup::Support),
        ("/about", CommandGroup::Legacy),
        ("/opportunities", CommandGroup::Experimental),
    ] {
        let summary = summaries
            .iter()
            .find(|summary| summary.name == command)
            .unwrap_or_else(|| panic!("{command} missing from command summaries"));
        assert_eq!(summary.group, expected_group, "{command} group drifted");
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
fn tool_declarations_own_provider_schema() {
    let registry = ToolRegistry::mvp();
    let specs = registry.tool_specs();
    for declaration in registry.declarations() {
        assert_eq!(
            declaration.parameters["type"], "object",
            "{} should expose an object parameter schema",
            declaration.name
        );
        let spec = specs
            .iter()
            .find(|spec| spec.function.name == declaration.name)
            .unwrap_or_else(|| panic!("{} missing provider spec", declaration.name));
        assert_eq!(
            spec.function.parameters, declaration.parameters,
            "{} provider schema should come from its declaration",
            declaration.name
        );
    }
}

#[test]
fn tool_declarations_build_permission_requests() {
    let registry = ToolRegistry::mvp();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();

    let git_commit = registry
        .declaration("git_commit")
        .expect("git_commit declaration");
    let git_request = git_commit.permission_request(ToolPermissionContext {
        command: Some("git commit -m checkpoint".to_string()),
        path: Some(workspace.clone()),
        creates_process: true,
        explicit_approval: true,
        ..ToolPermissionContext::default()
    });
    assert_eq!(git_request.tool, "git_commit");
    assert_eq!(git_request.surface, ToolSurface::Git);
    assert!(git_request.writes_files);
    assert!(!git_request.requires_network);
    assert!(git_request.creates_process);
    assert!(git_request.explicit_approval);

    let shell = registry
        .declaration("run_shell")
        .expect("run_shell declaration");
    let shell_request = shell.permission_request(ToolPermissionContext {
        command: Some("cargo test".to_string()),
        path: Some(workspace),
        writes_files: Some(true),
        requires_network: Some(true),
        creates_process: true,
        ..ToolPermissionContext::default()
    });
    assert_eq!(shell_request.surface, ToolSurface::Shell);
    assert!(shell_request.writes_files);
    assert!(shell_request.requires_network);

    let setup_environment = registry
        .declaration("setup_environment")
        .expect("setup_environment declaration");
    let setup_request = setup_environment.permission_request(ToolPermissionContext {
        command: Some("deepcli environment setup compiler".to_string()),
        path: Some(Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()),
        creates_process: true,
        explicit_approval: true,
        ..ToolPermissionContext::default()
    });
    assert_eq!(setup_request.surface, ToolSurface::Docker);
    assert!(setup_request.writes_files);
    assert!(setup_request.requires_network);
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
    for summary in CommandRouter::help_summaries() {
        assert_eq!(
            documented_commands.get(summary.name).map(String::as_str),
            Some(summary.group.as_str()),
            "{} command group differs between code and docs",
            summary.name
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

#[test]
fn tools_module_docs_cover_split_source_files() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tools_doc = fs::read_to_string(root.join("docs/MODULES/tools.md")).unwrap();
    for source in [
        "src/tools/declarations.rs",
        "src/tools/schema.rs",
        "src/tools/process.rs",
        "src/tools/git.rs",
        "src/tools/environment.rs",
        "src/tools/test_discovery.rs",
        "src/tools/web.rs",
        "src/tools/file.rs",
    ] {
        assert!(
            root.join(source).exists(),
            "{source} should exist for the documented tools split"
        );
        assert!(
            tools_doc.contains(source),
            "docs/MODULES/tools.md should mention {source}"
        );
    }
}

#[test]
fn commands_module_docs_cover_split_source_files() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    for source in [
        "src/commands/response.rs",
        "src/commands/registry.rs",
        "src/commands/parser.rs",
        "src/commands/help.rs",
        "src/commands/completion.rs",
        "src/commands/version.rs",
        "src/commands/quickstart.rs",
        "src/commands/selftest.rs",
        "src/commands/preflight.rs",
        "src/commands/permissions.rs",
        "src/commands/timeout.rs",
        "src/commands/model.rs",
        "src/commands/logs.rs",
        "src/commands/trace.rs",
        "src/commands/context.rs",
        "src/commands/usage.rs",
    ] {
        assert!(
            root.join(source).exists(),
            "{source} should exist for command module ownership"
        );
        assert!(
            commands_doc.contains(source),
            "docs/MODULES/commands.md should mention {source}"
        );
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
