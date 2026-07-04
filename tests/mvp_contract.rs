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
        "/diagnose",
        "/support",
        "/doctor",
        "/trace",
        "/logs",
        "/privacy",
        "/context",
        "/permissions",
        "/login",
        "/apikey",
        "/logout",
        "/credentials",
        "/config",
        "/timeout",
        "/model",
        "/goal",
        "/plan",
        "/fork",
        "/diff",
        "/review",
        "/verify",
        "/handoff",
        "/test",
        "/compiler",
        "/install",
        "/git",
        "/web",
        "/prompt",
        "/skill",
        "/agent",
        "/btw",
        "/approval",
        "/session",
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
fn command_registry_explicitly_owns_public_command_metadata() {
    let summaries = CommandRouter::help_summaries();
    let metadata = CommandRouter::command_metadata();
    let metadata_by_name: BTreeMap<&str, _> =
        metadata.iter().map(|entry| (entry.name, entry)).collect();

    assert_eq!(
        metadata_by_name.len(),
        metadata.len(),
        "command metadata registry should not contain duplicate command names"
    );

    for summary in summaries {
        let entry = metadata_by_name
            .get(summary.name)
            .unwrap_or_else(|| panic!("{} missing explicit command metadata", summary.name));
        assert_eq!(
            entry.group, summary.group,
            "{} help summary group should come from explicit metadata",
            summary.name
        );
        assert_eq!(
            entry.running_safe, summary.running_safe,
            "{} help summary running-safe flag should come from explicit metadata",
            summary.name
        );
    }
}

#[test]
fn command_registry_owns_completion_alias_metadata() {
    let aliases = CommandRouter::completion_alias_metadata();
    let alias_names = aliases.iter().map(|alias| alias.name).collect::<Vec<_>>();

    assert_eq!(
        alias_names,
        [
            "deepseek",
            "kimi",
            "ask",
            "stream",
            "tui",
            "repl",
            "sessions",
            "completions",
        ],
        "completion-only aliases should be explicit registry metadata"
    );

    for alias in aliases {
        let expected_group = if alias.name == "repl" {
            CommandGroup::Legacy
        } else {
            CommandGroup::Support
        };
        assert_eq!(
            alias.group, expected_group,
            "{} completion alias group should be owned by registry metadata",
            alias.name
        );
        assert!(
            alias.running_safe,
            "{} completion alias should stay marked running-safe for shell catalogs",
            alias.name
        );
        assert!(
            !alias.summary.trim().is_empty(),
            "{} completion alias should carry a shell catalog summary",
            alias.name
        );
    }

    let registry_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/registry.rs");
    let registry = fs::read_to_string(&registry_path).expect("read src/commands/registry.rs");
    assert!(
        registry.contains("const fn legacy_completion_alias"),
        "legacy completion-only aliases should use an explicit successor/policy constructor"
    );
}

#[test]
fn command_registry_owns_legacy_successor_metadata() {
    let legacy = CommandRouter::legacy_command_metadata();
    let legacy_names = legacy.iter().map(|entry| entry.name).collect::<Vec<_>>();
    let registered_legacy = CommandRouter::help_summaries()
        .iter()
        .filter(|summary| summary.group == CommandGroup::Legacy)
        .map(|summary| summary.name)
        .collect::<Vec<_>>();
    assert_eq!(
        legacy_names, registered_legacy,
        "every legacy command should have explicit successor metadata"
    );

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/COMMANDS.md");
    let docs = fs::read_to_string(&path).expect("read docs/COMMANDS.md");
    let documented_rows = documented_command_rows(&docs);
    for entry in legacy {
        assert!(
            !entry.successor.trim().is_empty(),
            "{} legacy successor should not be empty",
            entry.name
        );
        assert!(
            !entry.policy.trim().is_empty(),
            "{} legacy policy should not be empty",
            entry.name
        );
        let row = documented_rows
            .get(entry.name)
            .unwrap_or_else(|| panic!("docs/COMMANDS.md missing {}", entry.name))
            .join(" ");
        assert!(
            row.contains(entry.successor),
            "{} docs row should mention successor `{}`",
            entry.name,
            entry.successor
        );
        assert!(
            row.contains("替代："),
            "{} docs row should mark the replacement explicitly",
            entry.name
        );
    }
}

#[test]
fn command_policy_owner_projects_group_and_legacy_strategy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_entrypoint = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    let source = "src/commands/command_policy.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for command group and legacy policy projection ownership"
    );
    assert!(
        commands_entrypoint.contains("mod command_policy;"),
        "src/commands.rs should register the command policy owner module"
    );
    assert!(
        commands_doc.contains(source),
        "docs/MODULES/commands.md should mention {source}"
    );

    let policy_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "struct CommandGroupPolicy",
        "fn command_group_policy_json",
        "fn legacy_command_policy_json",
        "fn command_policy_group_policies",
    ] {
        assert!(
            policy_source.contains(item),
            "{item} should live in {source}"
        );
        assert!(
            commands_doc.contains(item.trim_start_matches("struct ").trim_start_matches("fn ")),
            "docs/MODULES/commands.md should document {item} ownership"
        );
    }
}

#[test]
fn command_registry_owns_slash_alias_metadata() {
    let aliases = CommandRouter::command_alias_metadata();
    let alias_targets = aliases
        .iter()
        .map(|alias| (alias.name, alias.canonical))
        .collect::<Vec<_>>();

    assert_eq!(
        alias_targets,
        [
            ("/support", "/diagnose"),
            ("/login", "/credentials"),
            ("/apikey", "/credentials"),
            ("/logout", "/credentials"),
            ("/accept", "/verify"),
            ("/gate", "/verify"),
            ("/compiler", "/env"),
            ("/install", "/env"),
            ("/cleanup", "/session"),
        ],
        "parser compatibility aliases should be explicit registry metadata"
    );

    for alias in aliases {
        assert!(
            CommandRouter::command_metadata()
                .iter()
                .any(|metadata| metadata.name == alias.name),
            "{} alias should remain a documented command while it is public",
            alias.name
        );
    }
}

#[test]
fn command_docs_match_registry() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/COMMANDS.md");
    let doc = fs::read_to_string(&path).expect("read docs/COMMANDS.md");

    let mut documented: BTreeMap<String, String> = BTreeMap::new();
    for line in doc.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("| /") {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() < 2 {
            continue;
        }
        documented.insert(cells[0].to_string(), cells[1].to_string());
    }

    let registry: BTreeMap<String, String> = CommandRouter::help_summaries()
        .iter()
        .map(|summary| (summary.name.to_string(), summary.group.as_str().to_string()))
        .collect();

    let documented_names: BTreeSet<&String> = documented.keys().collect();
    let registry_names: BTreeSet<&String> = registry.keys().collect();
    assert_eq!(
        documented_names, registry_names,
        "docs/COMMANDS.md command list drifted from the command registry"
    );

    for (name, group) in &registry {
        assert_eq!(
            documented.get(name),
            Some(group),
            "{name} group in docs/COMMANDS.md drifted from the registry"
        );
    }
}

#[test]
fn command_surface_pruning_audit_covers_aliases_and_legacy_entries() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/COMMANDS.md");
    let docs = fs::read_to_string(&path).expect("read docs/COMMANDS.md");

    assert!(
        docs.contains("## 删除/降级审计"),
        "docs/COMMANDS.md should record the Stage 6 deletion/demotion audit"
    );
    assert!(
        docs.contains("当前未发现可直接删除的公开入口"),
        "docs/COMMANDS.md should state the current deletion audit conclusion"
    );

    for alias in CommandRouter::command_alias_metadata() {
        let marker = format!("`{}` -> `{}`", alias.name, alias.canonical);
        assert!(
            docs.contains(&marker),
            "docs/COMMANDS.md pruning audit should cover parser alias {marker}"
        );
    }

    for legacy in CommandRouter::legacy_command_metadata() {
        let marker = format!("`{}` -> `{}`", legacy.name, legacy.successor);
        assert!(
            docs.contains(&marker),
            "docs/COMMANDS.md pruning audit should cover legacy command {marker}"
        );
    }

    for alias in CommandRouter::completion_alias_metadata() {
        let marker = match alias.successor {
            Some(successor) => format!("`completion:{}` -> `{}`", alias.name, successor),
            None => format!("`completion:{}`", alias.name),
        };
        assert!(
            docs.contains(&marker),
            "docs/COMMANDS.md pruning audit should cover completion alias {marker}"
        );
    }
}

#[test]
fn authoritative_docs_exist_and_cover_schema_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for doc in [
        "docs/ARCHITECTURE.md",
        "docs/CORE_FEATURES.md",
        "docs/COMMANDS.md",
        "docs/HARNESS.md",
        "docs/ADR/0001-harness-first.md",
        "docs/ADR/0002-command-surface-pruning.md",
        "docs/ADR/0003-schema-id-registry.md",
    ] {
        let path = root.join(doc);
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("authoritative doc {doc} should exist"));
        assert!(
            body.trim().len() > 80,
            "authoritative doc {doc} should not be empty"
        );
    }

    // The stable schema-id registry must have a documented owner entry.
    let core_features = fs::read_to_string(root.join("docs/CORE_FEATURES.md")).unwrap();
    assert!(
        core_features.contains("schema_ids"),
        "CORE_FEATURES.md should document the schema-id registry owner"
    );
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
        "todo_write",
        "ask_user_question",
        "web_search",
        "web_fetch",
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
    assert_eq!(registry.tools().len(), registry.declarations().len());
    assert!(registry.tool("read_file").is_some());
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
    for section in ["## 模块地图", "## 边界原则", "## 文档同步", "## 验证"] {
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
        for section in ["## 职责", "## 边界", "## 测试", "## 文档同步"] {
            assert!(
                contents.contains(section),
                "{} missing section {section}",
                path.display()
            );
        }
    }
}

#[test]
fn commands_entrypoint_uses_external_test_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    assert!(
        commands_source.contains("#[cfg(test)]\nmod tests;"),
        "src/commands.rs should keep command contract tests in src/commands/tests.rs"
    );
    assert!(
        !commands_source.contains("mod tests {"),
        "src/commands.rs should not keep the large inline test module"
    );
    assert!(
        !commands_source.contains("#[cfg(test)]\nuse "),
        "test-only imports should live in src/commands/tests.rs, not src/commands.rs"
    );

    let tests_source = fs::read_to_string(root.join("src/commands/tests.rs")).unwrap();
    assert!(
        tests_source.contains("use super::*;"),
        "src/commands/tests.rs should remain a child module with access to command internals"
    );
}

#[test]
fn ui_entrypoint_uses_external_test_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let ui_tests = root.join("src/ui/tests.rs");

    assert!(
        ui_tests.exists(),
        "large UI tests should live in src/ui/tests.rs instead of src/ui.rs"
    );
    assert!(
        ui_entrypoint.contains("#[cfg(test)]\nmod tests;"),
        "src/ui.rs should delegate tests to an external test module"
    );
    assert!(
        !ui_entrypoint.contains("mod tests {"),
        "src/ui.rs should not keep the large inline test module"
    );
}

#[test]
fn commands_entrypoint_delegates_stateless_shared_helpers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let shared_source = fs::read_to_string(root.join("src/commands/shared.rs"))
        .expect("stateless command helpers should live in src/commands/shared.rs");

    assert!(
        commands_source.contains("mod shared;"),
        "src/commands.rs should register the stateless shared helper module"
    );
    for helper in [
        "fn active_default_model",
        "fn project_config_path",
        "fn workspace_relative_display",
        "fn dedup_preserve_order",
        "fn provider_env_key",
        "fn required_arg",
        "fn parse_positive_usize",
    ] {
        assert!(
            !commands_source.contains(helper),
            "{helper} should not be implemented in src/commands.rs"
        );
        assert!(
            shared_source.contains(helper),
            "{helper} should be implemented in src/commands/shared.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/shared.rs"),
        "commands module docs should document the shared helper owner"
    );
}

#[test]
fn commands_entrypoint_delegates_session_shared_helpers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let session_helpers_source = fs::read_to_string(root.join("src/commands/session_helpers.rs"))
        .expect("session command helpers should live in src/commands/session_helpers.rs");

    assert!(
        commands_source.contains("mod session_helpers;"),
        "src/commands.rs should register the session helper owner module"
    );
    for helper in [
        "fn format_session_list",
        "fn session_has_no_recorded_activity",
        "fn latest_session_with_recorded_activity",
        "fn session_state_name",
        "fn session_storage_bytes",
    ] {
        assert!(
            !commands_source.contains(helper),
            "{helper} should not be implemented in src/commands.rs"
        );
        assert!(
            session_helpers_source.contains(helper),
            "{helper} should be implemented in src/commands/session_helpers.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/session_helpers.rs"),
        "commands module docs should document the session helper owner"
    );
}

#[test]
fn session_delegates_restore_backup_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let session_source = fs::read_to_string(root.join("src/commands/session.rs")).unwrap();
    let restore_source = fs::read_to_string(root.join("src/commands/session_restore.rs"))
        .expect("session restore-backup owner should live in src/commands/session_restore.rs");

    assert!(
        commands_source.contains("mod session_restore;"),
        "src/commands.rs should register the session restore-backup owner module"
    );
    for item in [
        "struct RestoreBackupArgs",
        "struct RestoreBackupFormat",
        "fn handle_restore_backup",
        "fn handle_restore_backup_dry_run",
        "fn render_restore_backup_dry_run",
        "fn parse_restore_backup_args",
        "fn resolve_restore_backup_session",
        "fn select_backup_record",
        "fn resolve_restore_target",
        "fn restore_preview_diff",
        "fn restore_backup_next_actions",
        "fn format_restore_backup_report",
        "fn format_restore_backup_json",
    ] {
        assert!(
            restore_source.contains(item),
            "{item} should be implemented in src/commands/session_restore.rs"
        );
        assert!(
            !session_source.contains(item),
            "{item} should not remain implemented in src/commands/session.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/session_restore.rs"),
        "commands module docs should document the session restore-backup owner"
    );
    for term in [
        "restore-backup owner",
        "restore-backup dry-run",
        "restore preview JSON",
        "session backup restore",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document session restore owner term `{term}`"
        );
    }
}

#[test]
fn session_delegates_catalog_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let session_source = fs::read_to_string(root.join("src/commands/session.rs")).unwrap();
    let catalog_source = fs::read_to_string(root.join("src/commands/session_catalog.rs"))
        .expect("session catalog owner should live in src/commands/session_catalog.rs");

    assert!(
        commands_source.contains("mod session_catalog;"),
        "src/commands.rs should register the session catalog owner module"
    );
    for item in [
        "struct SessionListOptions",
        "struct SessionPruneEmptyOptions",
        "struct SessionPruneEmptyReport",
        "struct SessionSearchOptions",
        "struct SessionSearchHit",
        "struct SessionSearchReport",
        "struct SessionListReport",
        "fn handle_session_default_list",
        "fn handle_session_list",
        "fn handle_session_search",
        "fn handle_session_prune_empty",
        "fn parse_session_list_args",
        "fn parse_session_search_args",
        "fn parse_session_prune_empty_args",
        "fn prune_empty_sessions",
        "fn format_session_prune_empty_json",
        "fn collect_session_search_report",
        "fn format_session_search_report",
        "fn format_session_search_json",
        "fn session_search_matches",
        "fn collect_session_list_report",
        "fn format_session_list_json",
        "fn session_list_item_json",
        "fn filter_session_metadata_with_activity",
        "fn format_limited_session_list",
    ] {
        assert!(
            catalog_source.contains(item),
            "{item} should be implemented in src/commands/session_catalog.rs"
        );
        assert!(
            !session_source.contains(item),
            "{item} should not remain implemented in src/commands/session.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/session_catalog.rs"),
        "commands module docs should document the session catalog owner"
    );
    for term in [
        "session catalog owner",
        "session list/search projection",
        "prune-empty report",
        "session catalog JSON",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document session catalog owner term `{term}`"
        );
    }
}

#[test]
fn session_delegates_inspect_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let session_source = fs::read_to_string(root.join("src/commands/session.rs")).unwrap();
    let inspect_source = fs::read_to_string(root.join("src/commands/session_inspect.rs"))
        .expect("session inspect owner should live in src/commands/session_inspect.rs");

    assert!(
        commands_source.contains("mod session_inspect;"),
        "src/commands.rs should register the session inspect owner module"
    );
    for item in [
        "struct SessionInspectOptions",
        "struct ToolCallFilter",
        "fn handle_session_show",
        "fn handle_session_history",
        "fn handle_session_summary",
        "fn handle_session_tools",
        "fn handle_session_tests",
        "fn handle_session_diffs",
        "fn handle_session_backups",
        "fn parse_session_single_inspect_options",
        "fn parse_session_record_inspect_options",
        "fn parse_session_tools_args",
        "fn format_session_messages",
        "fn format_session_inspect_json",
        "fn session_inspect_next_actions",
        "fn session_inspect_metadata_json",
        "fn session_activity_json",
        "fn session_message_json",
        "fn tool_call_record_json",
        "fn test_run_record_json",
        "fn session_diff_record_json",
        "fn session_backup_record_json",
        "fn load_recent_failed_tool_calls",
        "fn is_failed_or_denied_tool_call",
        "fn format_tool_calls",
        "fn format_tool_call_record",
        "fn format_test_runs",
        "fn format_session_diffs",
        "fn format_session_backups",
    ] {
        assert!(
            inspect_source.contains(item),
            "{item} should be implemented in src/commands/session_inspect.rs"
        );
        assert!(
            !session_source.contains(item),
            "{item} should not remain implemented in src/commands/session.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/session_inspect.rs"),
        "commands module docs should document the session inspect owner"
    );
    for term in [
        "session inspect owner",
        "session record projection",
        "session inspect JSON",
        "session tools/tests/diffs/backups",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document session inspect owner term `{term}`"
        );
    }
}

#[test]
fn session_delegates_recovery_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let session_source = fs::read_to_string(root.join("src/commands/session.rs")).unwrap();
    let recovery_source = fs::read_to_string(root.join("src/commands/session_recovery.rs"))
        .expect("session recovery owner should live in src/commands/session_recovery.rs");

    assert!(
        commands_source.contains("mod session_recovery;"),
        "src/commands.rs should register the session recovery owner module"
    );
    for item in [
        "struct SessionNextOptions",
        "struct SessionDiagnoseOptions",
        "fn handle_session_next",
        "fn handle_session_diagnose",
        "fn resolve_session_for_next_actions",
        "fn latest_session_with_next_action_signals",
        "fn session_has_next_action_signals",
        "fn parse_session_next_options",
        "fn parse_session_diagnose_options",
        "fn format_session_next_actions",
        "fn format_session_next_json",
        "fn session_next_action_items",
        "fn session_quick_link_items",
        "fn session_next_session_json",
        "fn session_next_signals_json",
        "fn format_session_diagnosis",
        "fn format_session_diagnosis_json",
        "fn session_diagnosis_tool_call_json",
        "fn session_diagnosis_test_json",
        "fn session_diagnosis_plan_json",
        "fn session_next_action_items_from_report",
    ] {
        assert!(
            recovery_source.contains(item),
            "{item} should be implemented in src/commands/session_recovery.rs"
        );
        assert!(
            !session_source.contains(item),
            "{item} should not remain implemented in src/commands/session.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/session_recovery.rs"),
        "commands module docs should document the session recovery owner"
    );
    for term in [
        "session recovery owner",
        "session next projection",
        "session diagnose projection",
        "next-action signals",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document session recovery owner term `{term}`"
        );
    }
}

#[test]
fn session_delegates_export_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let session_source = fs::read_to_string(root.join("src/commands/session.rs")).unwrap();
    let export_source = fs::read_to_string(root.join("src/commands/session_export.rs"))
        .expect("session export owner should live in src/commands/session_export.rs");

    assert!(
        commands_source.contains("mod session_export;"),
        "src/commands.rs should register the session export owner module"
    );
    for item in [
        "fn handle_session_export",
        "fn parse_export_args",
        "fn resolve_export_path",
        "fn export_session",
    ] {
        assert!(
            export_source.contains(item),
            "{item} should be implemented in src/commands/session_export.rs"
        );
        assert!(
            !session_source.contains(item),
            "{item} should not remain implemented in src/commands/session.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/session_export.rs"),
        "commands module docs should document the session export owner"
    );
    for term in [
        "session export owner",
        "session export JSON",
        "export path safety",
        "session export parser",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document session export owner term `{term}`"
        );
    }
}

#[test]
fn session_delegates_rename_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let session_source = fs::read_to_string(root.join("src/commands/session.rs")).unwrap();
    let rename_source = fs::read_to_string(root.join("src/commands/session_rename.rs"))
        .expect("session rename owner should live in src/commands/session_rename.rs");

    assert!(
        commands_source.contains("mod session_rename;"),
        "src/commands.rs should register the session rename owner module"
    );
    for item in ["fn handle_session_rename", "fn parse_session_rename_args"] {
        assert!(
            rename_source.contains(item),
            "{item} should be implemented in src/commands/session_rename.rs"
        );
        assert!(
            !session_source.contains(item),
            "{item} should not remain implemented in src/commands/session.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/session_rename.rs"),
        "commands module docs should document the session rename owner"
    );
    for term in [
        "session rename owner",
        "session rename parser",
        "session title update",
        "current-session rename",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document session rename owner term `{term}`"
        );
    }
}

#[test]
fn session_delegates_resumable_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let session_source = fs::read_to_string(root.join("src/commands/session.rs")).unwrap();
    let resumable_source = fs::read_to_string(root.join("src/commands/session_resumable.rs"))
        .expect("session resumable owner should live in src/commands/session_resumable.rs");

    assert!(
        commands_source.contains("mod session_resumable;"),
        "src/commands.rs should register the session resumable owner module"
    );
    for item in [
        "fn format_resumable_session_list",
        "fn sessions_with_resumable_context",
        "fn filter_session_metadata_with_resumable_context",
        "fn session_metadata_matches_workspace",
        "fn session_has_resumable_context",
        "fn session_is_low_information_clarification_only",
        "fn session_messages_are_low_information_clarification_only",
        "fn is_low_information_resume_input",
        "fn is_low_information_clarification_text",
        "fn session_is_thin_completed_chat_only",
        "fn is_short_single_line_reply",
        "fn strip_session_metric_footers",
        "fn format_limited_resumable_session_list",
        "fn resolve_resumable_session_for_workspace",
    ] {
        assert!(
            resumable_source.contains(item),
            "{item} should be implemented in src/commands/session_resumable.rs"
        );
        assert!(
            !session_source.contains(item),
            "{item} should not remain implemented in src/commands/session.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/session_resumable.rs"),
        "commands module docs should document the session resumable owner"
    );
    for term in [
        "session resumable owner",
        "resumable session filtering",
        "low-information clarification filter",
        "thin completed chat filter",
        "workspace resumable fallback",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document session resumable owner term `{term}`"
        );
    }
}

#[test]
fn session_delegates_selection_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let session_source = fs::read_to_string(root.join("src/commands/session.rs")).unwrap();
    let selection_source = fs::read_to_string(root.join("src/commands/session_selection.rs"))
        .expect("session selection owner should live in src/commands/session_selection.rs");

    assert!(
        commands_source.contains("mod session_selection;"),
        "src/commands.rs should register the session selection owner module"
    );
    for item in [
        "enum SessionFallbackKind",
        "struct ScopedListOptions",
        "struct QueueActionOptions",
        "struct ScopedActionOptions",
        "fn session_metadata_json",
        "fn session_has_recorded_activity",
        "fn resolve_session_for_inspection",
        "fn resolve_session_for_optional_inspection",
        "fn session_matches_fallback_kind",
        "fn session_fallback_label",
        "fn prefix_session_note",
        "fn parse_scoped_list_args",
        "fn parse_queue_action_options",
        "fn parse_scoped_action_args",
        "fn resolve_session_for_approval_action",
        "fn resolve_session_for_side_question_action",
        "fn short_id",
    ] {
        let entrypoint_marker = if item.starts_with("fn ") {
            format!("{item}(")
        } else {
            item.to_string()
        };
        assert!(
            selection_source.contains(&entrypoint_marker),
            "{item} should live in src/commands/session_selection.rs"
        );
        assert!(
            !session_source.contains(&entrypoint_marker),
            "{item} should not remain in src/commands/session.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/session_selection.rs"),
        "commands module docs should document the session selection owner"
    );
    for term in [
        "session selection owner",
        "SessionFallbackKind",
        "scoped action parser",
        "approval/BTW cross-session lookup",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document session selection owner term `{term}`"
        );
    }
}

#[test]
fn commands_entrypoint_delegates_environment_action_helpers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let environment_actions_source = fs::read_to_string(
        root.join("src/commands/environment_actions.rs"),
    )
    .expect("environment action helpers should live in src/commands/environment_actions.rs");

    assert!(
        commands_source.contains("mod environment_actions;"),
        "src/commands.rs should register the environment action helper owner module"
    );
    for helper in [
        "fn environment_next_actions",
        "fn default_environment_next_actions",
        "fn shell_command_from_slash_command",
    ] {
        assert!(
            !commands_source.contains(helper),
            "{helper} should not be implemented in src/commands.rs"
        );
        assert!(
            environment_actions_source.contains(helper),
            "{helper} should be implemented in src/commands/environment_actions.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/environment_actions.rs"),
        "commands module docs should document the environment action helper owner"
    );
}

#[test]
fn delivery_delegates_diff_projection_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let delivery_source = fs::read_to_string(root.join("src/commands/delivery.rs")).unwrap();
    let diff_source = fs::read_to_string(root.join("src/commands/delivery_diff.rs"))
        .expect("delivery diff projection should live in src/commands/delivery_diff.rs");

    assert!(
        commands_source.contains("mod delivery_diff;"),
        "src/commands.rs should register the delivery diff projection owner module"
    );
    for item in [
        "const SESSION_DIFF_FALLBACK_LIMIT",
        "struct SessionDiffSource",
        "struct DiffOptions",
        "enum DiffView",
        "fn parse_diff_args",
        "fn parse_review_args",
        "fn normalize_scope_path_filter",
        "fn filter_diff_by_paths",
        "fn format_verify_path_filters",
        "fn format_path_scope_args",
        "fn scoped_report_prefix",
        "fn format_diff_display",
        "fn format_session_diff_display",
        "struct DiffFileSummary",
        "fn diff_file_summaries",
        "fn format_diff_stat",
        "fn format_diff_name_only",
        "fn limit_display_lines",
        "fn no_scoped_diff_detail",
        "fn resolve_scoped_session_diff_source",
        "fn filter_session_diff_source_by_paths",
        "fn resolve_session_diff_source",
        "fn format_session_diff_fallback",
        "fn session_diff_fallback_header",
        "fn session_diff_review_input",
        "fn session_diff_record_display_path",
        "fn is_added_diff_line",
        "fn review_path_from_diff_line",
        "fn normalize_review_diff_path",
    ] {
        assert!(
            diff_source.contains(item),
            "{item} should be implemented in src/commands/delivery_diff.rs"
        );
        assert!(
            !delivery_source.contains(item),
            "{item} should not remain implemented in src/commands/delivery.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/delivery_diff.rs"),
        "commands module docs should document the delivery diff projection owner"
    );
    for term in [
        "delivery diff projection",
        "path scope filtering",
        "session diff fallback",
        "diff stat/name-only projection",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document delivery diff owner term `{term}`"
        );
    }
}

#[test]
fn delivery_delegates_review_heuristic_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let delivery_source = fs::read_to_string(root.join("src/commands/delivery.rs")).unwrap();
    let review_source = fs::read_to_string(root.join("src/commands/delivery_review.rs"))
        .expect("delivery review heuristic should live in src/commands/delivery_review.rs");

    assert!(
        commands_source.contains("mod delivery_review;"),
        "src/commands.rs should register the delivery review heuristic owner module"
    );
    for item in [
        "fn review_diff",
        "struct ReviewFindings",
        "struct ReviewFinding",
        "fn review_path_touches_credentials",
        "fn is_review_test_or_doc_path",
        "fn is_review_test_marker_line",
        "fn is_sensitive_review_line",
        "fn is_sensitive_review_detector_source_line",
        "fn has_explicit_secret_review_marker",
        "fn is_safe_sensitive_review_source_line",
        "fn is_dangerous_command_review_line",
        "fn is_review_detector_literal_line",
        "fn is_panic_prone_review_line",
        "fn is_panic_review_detector_source_line",
        "fn is_documented_invariant_expect_line",
        "fn review_finding_example",
        "fn review_worktree",
        "fn append_findings",
    ] {
        assert!(
            review_source.contains(item),
            "{item} should be implemented in src/commands/delivery_review.rs"
        );
        assert!(
            !delivery_source.contains(item),
            "{item} should not remain implemented in src/commands/delivery.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/delivery_review.rs"),
        "commands module docs should document the delivery review heuristic owner"
    );
    for term in [
        "delivery review heuristic",
        "review risk detection",
        "sensitive/dangerous/panic-prone finding projection",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document delivery review owner term `{term}`"
        );
    }
}

#[test]
fn delivery_delegates_verify_handoff_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let delivery_source = fs::read_to_string(root.join("src/commands/delivery.rs")).unwrap();
    let verify_source = fs::read_to_string(root.join("src/commands/delivery_verify.rs"))
        .expect("delivery verify/handoff owner should live in src/commands/delivery_verify.rs");

    assert!(
        commands_source.contains("mod delivery_verify;"),
        "src/commands.rs should register the delivery verify/handoff owner module"
    );
    for item in [
        "struct VerifyOptions",
        "struct HandoffOptions",
        "enum HandoffFormat",
        "fn handle_verify",
        "fn handle_handoff",
        "fn parse_verify_args",
        "fn parse_handoff_args",
        "fn resolve_session_for_verify",
        "fn run_verification_tests",
        "fn run_verification_environment_checks",
        "fn verification_test_run_from_output",
        "fn persist_verification_test_run_if_needed",
    ] {
        assert!(
            verify_source.contains(item),
            "{item} should be implemented in src/commands/delivery_verify.rs"
        );
        assert!(
            !delivery_source.contains(item),
            "{item} should not remain implemented in src/commands/delivery.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/delivery_verify.rs"),
        "commands module docs should document the delivery verify/handoff owner"
    );
    for term in [
        "delivery verify/handoff owner",
        "verify/handoff option parser",
        "test/env execution helper",
        "verification session selection",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document delivery verify/handoff owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_scorecard_opportunity_projection() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let opportunities_source = fs::read_to_string(
        root.join("src/commands/scorecard_opportunities.rs"),
    )
    .expect(
        "scorecard opportunity projection should live in src/commands/scorecard_opportunities.rs",
    );

    assert!(
        commands_source.contains("mod scorecard_opportunities;"),
        "src/commands.rs should register the scorecard opportunity projection owner module"
    );
    for item in [
        "struct ScorecardOpportunity",
        "const SCORECARD_ROUND_REPORT_ACTION",
        "const SCORECARD_OPPORTUNITIES_ACTION",
        "fn scorecard_product_opportunities",
        "fn opportunity_baseline_next_actions",
        "fn scorecard_opportunities_json",
        "fn scorecard_recommended_opportunity_json",
        "fn scorecard_opportunity_priority_counts_json",
        "fn scorecard_opportunity_effort_counts_json",
        "fn scorecard_opportunity_summary_text",
    ] {
        assert!(
            opportunities_source.contains(item),
            "{item} should be implemented in src/commands/scorecard_opportunities.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/scorecard_opportunities.rs"),
        "commands module docs should document the scorecard opportunity projection owner"
    );
    for term in [
        "scorecard opportunities",
        "opportunity projection",
        "recommended opportunity",
        "opportunity counts",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document scorecard opportunity owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_scorecard_report_builder() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let scorecard_report_source = fs::read_to_string(root.join("src/commands/scorecard_report.rs"))
        .expect("scorecard report builder should live in src/commands/scorecard_report.rs");

    assert!(
        commands_source.contains("mod scorecard_report;"),
        "src/commands.rs should register the scorecard report builder owner module"
    );
    for item in [
        "struct ScorecardOptions",
        "struct ScorecardCategory",
        "struct ScorecardReport",
        "const SCORECARD_BENCHMARK_REMEDIATION_ACTION",
        "fn handle_scorecard",
        "fn parse_scorecard_options",
        "fn parse_scorecard_threshold",
        "fn build_scorecard_report",
        "fn scorecard_command_category",
        "fn scorecard_add_evidence",
        "fn scorecard_add_gap",
        "fn scorecard_global_next_actions",
        "fn scorecard_prioritize_category_next_actions",
        "fn scorecard_category_checklist",
        "fn scorecard_percent",
        "fn scorecard_tier",
        "fn scorecard_category_status",
        "fn scorecard_score_scale_json",
        "struct ScorecardTextInput",
        "fn format_scorecard_text",
        "fn format_scorecard_json",
        "fn scorecard_summary_json",
    ] {
        assert!(
            scorecard_report_source.contains(item),
            "{item} should be implemented in src/commands/scorecard_report.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/scorecard_report.rs"),
        "commands module docs should document the scorecard report builder owner"
    );
    for term in [
        "scorecard report builder",
        "scorecard category projection",
        "scorecard summary JSON",
        "scorecard text/JSON output",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document scorecard report owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_round_report_builder() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let round_report_source = fs::read_to_string(root.join("src/commands/round_report.rs"))
        .expect("round report builder should live in src/commands/round_report.rs");

    assert!(
        commands_source.contains("mod round_report;"),
        "src/commands.rs should register the round report builder owner module"
    );
    for item in [
        "const DEFAULT_ROUND_SCORE_THRESHOLD",
        "struct RoundOptions",
        "struct RoundGate",
        "struct RoundReport",
        "struct RoundBenchmarkRun",
        "struct RoundTextInput",
        "fn handle_round",
        "fn parse_round_options",
        "fn run_round_benchmark_suite",
        "fn build_round_report",
        "fn scorecard_has_standalone_round_gaps",
        "fn format_round_text",
        "fn format_round_json",
        "fn round_summary_json",
        "fn round_gate_checklist",
        "fn round_benchmark_run_json",
    ] {
        assert!(
            round_report_source.contains(item),
            "{item} should be implemented in src/commands/round_report.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/round_report.rs"),
        "commands module docs should document the round report builder owner"
    );
    for term in [
        "round report builder",
        "round text/JSON output",
        "round summary JSON",
        "round benchmark suite wrapper",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document round report owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_benchmark_dispatch() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let benchmark_dispatch_source =
        fs::read_to_string(root.join("src/commands/benchmark_dispatch.rs"))
            .expect("benchmark dispatch should live in src/commands/benchmark_dispatch.rs");

    assert!(
        commands_source.contains("mod benchmark_dispatch;"),
        "src/commands.rs should register the benchmark dispatch owner module"
    );
    for item in [
        "fn handle_benchmark",
        "fn benchmark_status_args_request_failure",
        "fn benchmark_args_are_scorecard_compatible",
    ] {
        assert!(
            benchmark_dispatch_source.contains(item),
            "{item} should be implemented in src/commands/benchmark_dispatch.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/benchmark_dispatch.rs"),
        "commands module docs should document the benchmark dispatch owner"
    );
    for term in [
        "benchmark dispatch",
        "scorecard-compatible benchmark args",
        "benchmark gate dispatch",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document benchmark dispatch owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_round_benchmark_gate_projection() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let round_benchmark_gates_source = fs::read_to_string(
        root.join("src/commands/round_benchmark_gates.rs"),
    )
    .expect("round benchmark gate projection should live in src/commands/round_benchmark_gates.rs");

    assert!(
        commands_source.contains("mod round_benchmark_gates;"),
        "src/commands.rs should register the round benchmark gate projection owner module"
    );
    for item in [
        "fn round_benchmark_trends_needs_attention",
        "fn round_benchmark_trends_gate_summary",
        "fn round_benchmark_trends_gap",
        "fn round_benchmark_trends_next_action",
        "fn round_benchmark_gate_summary",
        "fn format_round_benchmark_preset_names",
        "fn round_benchmark_status_json",
        "fn format_round_benchmark_freshness_suffix",
    ] {
        assert!(
            round_benchmark_gates_source.contains(item),
            "{item} should be implemented in src/commands/round_benchmark_gates.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/round_benchmark_gates.rs"),
        "commands module docs should document the round benchmark gate projection owner"
    );
    for term in [
        "round benchmark gate projection",
        "benchmark trend gate",
        "round benchmark status projection",
        "freshness suffix",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document round benchmark gate owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_round_goal_status_projection() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let round_goal_status_source = fs::read_to_string(
        root.join("src/commands/round_goal_status.rs"),
    )
    .expect("round goal status projection should live in src/commands/round_goal_status.rs");

    assert!(
        commands_source.contains("mod round_goal_status;"),
        "src/commands.rs should register the round goal status projection owner module"
    );
    for item in [
        "struct RoundGoalStatus",
        "fn build_round_goal_status",
        "fn round_goal_status_json",
    ] {
        assert!(
            round_goal_status_source.contains(item),
            "{item} should be implemented in src/commands/round_goal_status.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/round_goal_status.rs"),
        "commands module docs should document the round goal status projection owner"
    );
    for term in [
        "round goal status",
        "goal readiness projection",
        "goalStatus JSON",
        "goal_readiness gate",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document round goal status owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_benchmark_status_projection() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let benchmark_status_source = fs::read_to_string(root.join("src/commands/benchmark_status.rs"))
        .expect("benchmark status projection should live in src/commands/benchmark_status.rs");

    assert!(
        commands_source.contains("mod benchmark_status;"),
        "src/commands.rs should register the benchmark status projection owner module"
    );
    for item in [
        "const BENCHMARK_STATUS_SCHEMA",
        "const BENCHMARK_EVIDENCE_REFRESH_AFTER_DAYS",
        "const BENCHMARK_EVIDENCE_STALE_AFTER_DAYS",
        "struct BenchmarkStatusOptions",
        "struct BenchmarkStatusReport",
        "fn handle_benchmark_status",
        "fn parse_benchmark_status_options",
        "fn build_benchmark_status_report",
        "fn format_benchmark_status_json",
        "fn format_benchmark_status_text",
        "fn benchmark_freshness_json",
        "fn benchmark_freshness_age_seconds",
        "fn benchmark_required_preset_statuses",
    ] {
        assert!(
            benchmark_status_source.contains(item),
            "{item} should be implemented in src/commands/benchmark_status.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/benchmark_status.rs"),
        "commands module docs should document the benchmark status projection owner"
    );
    for term in [
        "benchmark status",
        "freshness",
        "required preset",
        "format_benchmark_status_json",
        "format_benchmark_status_text",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document benchmark status owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_benchmark_baselines_projection() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let baselines_source = fs::read_to_string(root.join("src/commands/benchmark_baselines.rs"))
        .expect(
            "benchmark baselines projection should live in src/commands/benchmark_baselines.rs",
        );

    assert!(
        commands_source.contains("mod benchmark_baselines;"),
        "src/commands.rs should register the benchmark baselines projection owner module"
    );
    for item in [
        "fn sota_baseline_next_actions",
        "fn handle_benchmark_baselines",
        "fn handle_benchmark_baseline_template",
        "fn load_benchmark_baseline",
        "fn format_benchmark_baselines_json",
        "fn benchmark_baseline_template_value",
        "struct BenchmarkBaselineReport",
        "struct BenchmarkBaselineInventoryEntry",
    ] {
        assert!(
            baselines_source.contains(item),
            "{item} should be implemented in src/commands/benchmark_baselines.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/benchmark_baselines.rs"),
        "commands module docs should document the benchmark baselines projection owner"
    );
    for term in [
        "baseline-template",
        "baseline inventory",
        "compare-ready",
        "sota_baseline_next_actions",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document benchmark baselines owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_benchmark_history_projection() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let history_source = fs::read_to_string(root.join("src/commands/benchmark_history.rs"))
        .expect("benchmark history projection should live in src/commands/benchmark_history.rs");

    assert!(
        commands_source.contains("mod benchmark_history;"),
        "src/commands.rs should register the benchmark history projection owner module"
    );
    for item in [
        "fn handle_benchmark_summary",
        "fn handle_benchmark_trends",
        "fn handle_benchmark_compare",
        "fn build_benchmark_case_summaries",
        "fn build_benchmark_case_trends",
        "fn format_benchmark_summary_json",
        "fn format_benchmark_trends_json",
        "fn format_benchmark_compare_json",
        "struct BenchmarkCaseSummary",
        "struct BenchmarkCaseTrend",
        "struct BenchmarkCaseComparison",
    ] {
        assert!(
            history_source.contains(item),
            "{item} should be implemented in src/commands/benchmark_history.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/benchmark_history.rs"),
        "commands module docs should document the benchmark history projection owner"
    );
    for term in [
        "benchmark summary",
        "benchmark trends",
        "benchmark compare",
        "trend gate",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document benchmark history owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_benchmark_artifact_projection() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let artifacts_source = fs::read_to_string(root.join("src/commands/benchmark_artifacts.rs"))
        .expect("benchmark artifact projection should live in src/commands/benchmark_artifacts.rs");

    assert!(
        commands_source.contains("mod benchmark_artifacts;"),
        "src/commands.rs should register the benchmark artifact projection owner module"
    );
    for item in [
        "const BENCHMARK_ARTIFACT_SCHEMA",
        "struct BenchmarkArtifact",
        "fn handle_benchmark_list",
        "fn handle_benchmark_show",
        "fn handle_benchmark_cleanup",
        "fn load_benchmark_artifacts",
        "fn resolve_benchmark_artifact",
        "fn format_benchmark_list_json",
        "fn format_benchmark_cleanup_json",
        "fn format_benchmark_artifact_text",
        "fn benchmark_value_with_action_checklist",
        "fn benchmark_artifact_summary_json",
    ] {
        assert!(
            artifacts_source.contains(item),
            "{item} should be implemented in src/commands/benchmark_artifacts.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/benchmark_artifacts.rs"),
        "commands module docs should document the benchmark artifact projection owner"
    );
    for term in [
        "benchmark list",
        "benchmark show",
        "benchmark cleanup",
        "artifact projection",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document benchmark artifact owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_benchmark_presets_catalog() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let presets_source = fs::read_to_string(root.join("src/commands/benchmark_presets.rs"))
        .expect("benchmark presets catalog should live in src/commands/benchmark_presets.rs");

    assert!(
        commands_source.contains("mod benchmark_presets;"),
        "src/commands.rs should register the benchmark presets catalog owner module"
    );
    for item in [
        "struct BenchmarkPreset",
        "const BENCHMARK_PRESETS",
        "const MEANINGFUL_BENCHMARK_PRESETS",
        "const DEFAULT_BENCHMARK_RUN_SUITE_PRESETS",
        "fn handle_benchmark_presets",
        "fn benchmark_preset_by_name",
        "fn format_benchmark_presets_json",
        "fn format_benchmark_presets_text",
        "fn benchmark_preset_json",
    ] {
        assert!(
            presets_source.contains(item),
            "{item} should be implemented in src/commands/benchmark_presets.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/benchmark_presets.rs"),
        "commands module docs should document the benchmark presets catalog owner"
    );
    for term in [
        "benchmark presets",
        "preset catalog",
        "required evidence presets",
        "default suite presets",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document benchmark presets owner term `{term}`"
        );
    }
}

#[test]
fn productloop_delegates_benchmark_runs_execution() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let productloop_source = fs::read_to_string(root.join("src/commands/productloop.rs")).unwrap();
    let runs_source = fs::read_to_string(root.join("src/commands/benchmark_runs.rs"))
        .expect("benchmark run execution should live in src/commands/benchmark_runs.rs");

    assert!(
        commands_source.contains("mod benchmark_runs;"),
        "src/commands.rs should register the benchmark run execution owner module"
    );
    for item in [
        "const BENCHMARK_RUN_SUITE_REMEDIATION_ACTION",
        "const BENCHMARK_SUITE_SCHEMA",
        "struct BenchmarkRunArtifact",
        "struct BenchmarkCommandExecution",
        "fn handle_benchmark_run(",
        "fn execute_benchmark_run_artifact(",
        "fn handle_benchmark_run_suite(",
        "fn handle_benchmark_record(",
        "fn build_benchmark_run_json(",
        "fn build_benchmark_record_json(",
        "fn run_benchmark_shell_command(",
        "fn benchmark_execution_from_output(",
        "fn truncate_benchmark_output(",
        "fn unique_benchmark_artifact_path(",
        "fn benchmark_slug(",
    ] {
        assert!(
            runs_source.contains(item),
            "{item} should be implemented in src/commands/benchmark_runs.rs"
        );
        assert!(
            !productloop_source.contains(item),
            "{item} should not remain implemented in src/commands/productloop.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/benchmark_runs.rs"),
        "commands module docs should document the benchmark run execution owner"
    );
    for term in [
        "benchmark run",
        "benchmark record",
        "benchmark run-suite",
        "execution artifact",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document benchmark runs owner term `{term}`"
        );
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
fn delivery_delegates_report_builder_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_source = fs::read_to_string(root.join("src/commands.rs")).unwrap();
    let delivery_source = fs::read_to_string(root.join("src/commands/delivery.rs")).unwrap();
    let reports_source = fs::read_to_string(root.join("src/commands/delivery_reports.rs"))
        .expect("delivery report builder should live in src/commands/delivery_reports.rs");

    assert!(
        commands_source.contains("mod delivery_reports;"),
        "src/commands.rs should register the delivery report builder owner module"
    );
    for item in [
        "enum VerificationDiffSource",
        "enum VerificationTestRun",
        "enum VerificationEnvironmentCheck",
        "struct VerificationStatusSource",
        "struct VerificationReportInput",
        "struct HandoffReportInput",
        "fn format_verification_report",
        "fn format_verification_report_json",
        "fn verification_output_has_blockers",
        "fn format_handoff_report",
        "fn format_handoff_report_markdown",
        "fn format_handoff_report_pr_description",
        "fn format_handoff_report_json",
        "fn handoff_report_blockers",
        "fn verification_next_actions",
        "fn append_verification_environment",
        "fn format_git_status_summary",
    ] {
        assert!(
            reports_source.contains(item),
            "{item} should be implemented in src/commands/delivery_reports.rs"
        );
        assert!(
            !delivery_source.contains(item),
            "{item} should not remain implemented in src/commands/delivery.rs"
        );
    }

    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    assert!(
        commands_doc.contains("src/commands/delivery_reports.rs"),
        "commands module docs should document the delivery report builder owner"
    );
    for term in [
        "delivery report builder",
        "verification report projection",
        "handoff report projection",
        "delivery report JSON",
    ] {
        assert!(
            commands_doc.contains(term),
            "commands module docs should document delivery report owner term `{term}`"
        );
    }
}

#[test]
fn ui_module_docs_cover_monitor_projection_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let monitor_source = fs::read_to_string(root.join("src/ui/monitor.rs")).unwrap();
    let source = "src/ui/monitor.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for monitor projection ownership"
    );
    assert!(
        ui_entrypoint.contains("mod monitor;"),
        "src/ui.rs should register the monitor projection owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );
    for formatter in [
        "format_usage_tab_lines",
        "format_deliver_tab_lines",
        "format_tests_tab_lines",
        "format_session_tab_lines",
        "format_context_tab_lines",
        "format_environment_tab_lines",
        "format_approvals_tab_lines",
    ] {
        assert!(
            monitor_source.contains(&format!("fn {formatter}")),
            "{formatter} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&format!("fn {formatter}")),
            "{formatter} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(formatter),
            "docs/MODULES/ui.md should document {formatter} ownership"
        );
    }
    for quick_action in [
        "tool_quick_actions",
        "deliver_quick_actions",
        "environment_quick_actions",
        "environment_action_target",
        "environment_needs_setup",
    ] {
        assert!(
            monitor_source.contains(&format!("fn {quick_action}")),
            "{quick_action} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&format!("fn {quick_action}")),
            "{quick_action} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(quick_action),
            "docs/MODULES/ui.md should document {quick_action} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_monitor_health_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/monitor_health.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for health monitor projection ownership"
    );
    assert!(
        ui_entrypoint.contains("mod monitor_health;"),
        "src/ui.rs should register the health monitor projection owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let health_source = fs::read_to_string(root.join(source)).unwrap();
    for function in [
        "format_health_tab_lines",
        "health_quick_actions_for_state",
        "provider_needs_credentials_for_ui",
        "provider_env_key_for_ui",
        "presence_label",
    ] {
        assert!(
            health_source.contains(&format!("fn {function}")),
            "{function} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&format!("fn {function}")),
            "{function} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(function),
            "docs/MODULES/ui.md should document {function} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_monitor_library_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/monitor_library.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for library monitor projection ownership"
    );
    assert!(
        ui_entrypoint.contains("mod monitor_library;"),
        "src/ui.rs should register the library monitor projection owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let library_source = fs::read_to_string(root.join(source)).unwrap();
    for function in [
        "format_library_tab_lines",
        "library_quick_actions_for_state",
        "format_library_item",
    ] {
        assert!(
            library_source.contains(&format!("fn {function}")),
            "{function} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&format!("fn {function}")),
            "{function} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(function),
            "docs/MODULES/ui.md should document {function} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_monitor_output_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/monitor_output.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for result/trace monitor projection ownership"
    );
    assert!(
        ui_entrypoint.contains("mod monitor_output;"),
        "src/ui.rs should register the result/trace monitor projection owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let output_source = fs::read_to_string(root.join(source)).unwrap();
    for function in [
        "format_result_tab_lines",
        "result_output_window_size",
        "format_trace_tab_lines",
    ] {
        assert!(
            output_source.contains(&format!("fn {function}")),
            "{function} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&format!("fn {function}")),
            "{function} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(function),
            "docs/MODULES/ui.md should document {function} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_monitor_changes_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/monitor_changes.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for changes monitor projection ownership"
    );
    assert!(
        ui_entrypoint.contains("mod monitor_changes;"),
        "src/ui.rs should register the changes monitor projection owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let changes_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "struct WorkspaceChangesSnapshot",
        "struct WorkspaceDiffSection",
        "fn handle_changes_tab_key",
        "fn select_change_patch_at_row",
        "fn refresh_workspace_changes_snapshot",
        "fn format_changes_tab_lines",
        "fn append_workspace_changes_lines",
        "fn load_workspace_changes_snapshot",
        "fn parse_git_status_snapshot",
        "fn parse_diff_sections",
    ] {
        assert!(
            changes_source.contains(item),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(item),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("struct ").trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_monitor_tools_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/monitor_tools.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for tools monitor projection ownership"
    );
    assert!(
        ui_entrypoint.contains("mod monitor_tools;"),
        "src/ui.rs should register the tools monitor projection owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let tools_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "struct ToolLogItem",
        "struct ToolTabLine",
        "fn handle_tools_tab_key",
        "fn prefill_tools_session_command",
        "fn toggle_selected_tool",
        "fn toggle_tool_at_row",
        "fn move_selected_tool_by",
        "fn select_tool_at_index",
        "fn visible_tool_index_at_line",
        "fn selected_tool_panel_line",
        "fn format_tool_tab_lines",
        "fn tool_tab_lines",
        "fn append_tool_quick_action_lines",
        "fn tool_detail_preview_lines",
        "fn tool_detail_is_truncated",
    ] {
        assert!(
            tools_source.contains(item),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(item),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("struct ").trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_monitor_shell_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/monitor_shell.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for task monitor shell ownership"
    );
    assert!(
        ui_entrypoint.contains("mod monitor_shell;"),
        "src/ui.rs should register the task monitor shell owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let shell_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "fn render_task_monitor",
        "fn format_task_monitor_text",
        "fn format_task_overview_lines",
        "fn monitor_quick_actions_for_tab",
        "fn monitor_tab_strip",
        "fn format_monitor_tabs",
        "fn select_monitor_tab_at_position",
        "fn clicked_monitor_quick_action_index",
        "fn visible_panel_line_indices",
        "fn truncate_panel_lines",
        "fn truncate_panel_lines_with_focus",
        "fn selected_monitor_quick_action_line",
    ] {
        let marker = format!("{item}(");
        assert!(
            shell_source.contains(&marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_running_command_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/running_commands.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for running command ownership"
    );
    assert!(
        ui_entrypoint.contains("mod running_commands;"),
        "src/ui.rs should register the running command owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let running_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "fn handle_running_tui_local_command",
        "fn running_tui_supported_command_hint",
        "fn running_tui_deferred_input_hint",
        "fn ensure_running_no_output",
        "fn ensure_running_completion_is_observation_only",
        "fn ensure_running_round_is_read_only",
        "fn ensure_running_benchmark_is_read_only",
        "fn ensure_running_preflight_is_planned",
        "fn ensure_running_session_is_read_only",
        "fn ensure_running_git_is_read_only",
        "fn handle_tui_running_git",
        "fn format_tui_running_status",
        "fn handle_tui_running_btw",
        "fn handle_tui_running_terminal",
    ] {
        assert!(
            running_source.contains(item),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(item),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_resume_picker_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/resume_picker.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for resume picker ownership"
    );
    assert!(
        ui_entrypoint.contains("mod resume_picker;"),
        "src/ui.rs should register the resume picker owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let picker_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "struct ResumePicker",
        "enum ResumeSelection",
        "fn pick_resume_session",
        "fn session_matches_resume_query",
        "fn run_resume_picker_loop",
        "fn resume_filter_accepts_char",
        "fn handle_resume_picker_key",
        "fn handle_resume_picker_mouse_for_state",
        "fn handle_resume_picker_mouse",
        "fn resume_picker_layout",
        "fn render_resume_picker",
        "fn format_resume_preview_text",
    ] {
        assert!(
            picker_source.contains(item),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(item),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(
                item.trim_start_matches("struct ")
                    .trim_start_matches("enum ")
                    .trim_start_matches("fn ")
            ),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_approval_interaction_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/approvals.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for approval interaction ownership"
    );
    assert!(
        ui_entrypoint.contains("mod approvals;"),
        "src/ui.rs should register the approval interaction owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let approval_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "struct SideQuestionPrompt",
        "enum SelectedBlocker",
        "fn handle_approval_tab_key",
        "fn handle_approvals_mouse_for_state",
        "fn clicked_approvals_tab_index",
        "fn selected_blocker",
        "fn activate_selected_blocker",
        "fn deny_selected_blocker",
        "fn open_side_question_answer_prompt",
        "fn handle_side_question_prompt_key",
        "fn confirm_side_question_prompt",
        "fn answer_side_question_for_state",
        "fn update_selected_approval",
        "fn update_approval_for_state",
        "fn clamp_selected_blocker_to_monitor",
    ] {
        assert!(
            approval_source.contains(item),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(item),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(
                item.trim_start_matches("struct ")
                    .trim_start_matches("enum ")
                    .trim_start_matches("fn ")
            ),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_dialog_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/dialogs.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for dialog ownership"
    );
    assert!(
        ui_entrypoint.contains("mod dialogs;"),
        "src/ui.rs should register the dialog owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let dialog_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "enum DialogKind",
        "enum TuiDialog",
        "struct DiffDialog",
        "struct AgentEditorDialog",
        "struct SettingsDialog",
        "struct InterviewDialog",
        "fn open_diff_dialog",
        "fn open_agent_editor_dialog",
        "fn open_latest_agent_editor_dialog",
        "fn open_settings_dialog",
        "fn open_interview_dialog",
        "fn render_dialog",
        "fn dialog_view_for_state",
        "fn dialog_body_for_state",
        "fn handle_dialog_key",
        "fn replace_dialog_field",
        "fn clamp_selected_blocker_to_monitor",
    ] {
        assert!(
            dialog_source.contains(item),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(item),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(
                item.trim_start_matches("struct ")
                    .trim_start_matches("enum ")
                    .trim_start_matches("fn ")
            ),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn runtime_docs_cover_context_manager_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let architecture_doc = fs::read_to_string(root.join("docs/ARCHITECTURE.md")).unwrap();
    let lib = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    let runtime = fs::read_to_string(root.join("src/runtime.rs")).unwrap();
    let source = "src/context_manager.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for context preparation ownership"
    );
    assert!(
        lib.contains("pub mod context_manager;"),
        "src/lib.rs should register the context manager owner module"
    );
    assert!(
        architecture_doc.contains(source),
        "docs/ARCHITECTURE.md should mention {source}"
    );

    let context_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "struct ContextManager",
        "struct ContextCompactionOptions",
        "struct ContextPreparation",
        "fn prepare(",
        "fn microcompact_tool_outputs",
        "fn compact_messages_for_provider",
        "fn provider_messages_to_retained_segment",
        "fn message_groups_omitted_after_compaction",
    ] {
        assert!(
            context_source.contains(item),
            "{item} should live in {source}"
        );
        let runtime_needle = if item.starts_with("fn ") && !item.ends_with('(') {
            format!("{item}(")
        } else {
            item.to_string()
        };
        assert!(
            !runtime.contains(&runtime_needle),
            "{item} should not remain in src/runtime.rs"
        );
        let documented_name = item
            .trim_start_matches("struct ")
            .trim_start_matches("fn ")
            .trim_end_matches('(');
        assert!(
            architecture_doc.contains(documented_name),
            "docs/ARCHITECTURE.md should document {item} ownership"
        );
    }
}

#[test]
fn agent_docs_cover_subagent_lifecycle_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let architecture_doc = fs::read_to_string(root.join("docs/ARCHITECTURE.md")).unwrap();
    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    let features_doc = fs::read_to_string(root.join("docs/FEATURES.md")).unwrap();
    let agents_source = fs::read_to_string(root.join("src/agents.rs")).unwrap();
    let tools_source = fs::read_to_string(root.join("src/tools.rs")).unwrap();
    let agent_command_source = fs::read_to_string(root.join("src/commands/agent.rs")).unwrap();

    for item in [
        "struct SubagentEvent",
        "fn mark_subagent_started",
        "fn heartbeat_subagent",
        "fn complete_subagent",
        "fn fail_subagent",
        "fn read_subagent_events",
    ] {
        assert!(
            agents_source.contains(item),
            "{item} should live in src/agents.rs"
        );
    }
    for item in [
        "fn start_subagent_background",
        "\"background_start\"",
        "fn subagent_next_actions",
    ] {
        assert!(
            tools_source.contains(item),
            "{item} should live in src/tools.rs"
        );
    }
    for item in [
        "async fn resume_subagent_command",
        "AgentRuntime::new",
        "fn format_agent_logs_json",
    ] {
        assert!(
            agent_command_source.contains(item),
            "{item} should live in src/commands/agent.rs"
        );
    }
    assert!(architecture_doc.contains("子 Agent task、lifecycle、事件日志和恢复元数据"));
    assert!(commands_doc.contains("/agent resume|logs"));
    assert!(features_doc.contains("spawn_subagent"));
    assert!(features_doc.contains("agent list|show|resume|logs --json"));
    assert!(!commands_doc.contains("/agent run|resume|logs"));
    assert!(!features_doc.contains("agent list|show|run|resume|logs"));
}

#[test]
fn ui_module_docs_cover_command_palette_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/command_palette.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for command palette ownership"
    );
    assert!(
        ui_entrypoint.contains("mod command_palette;"),
        "src/ui.rs should register the command palette owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let palette_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "const COMMAND_PALETTE_MATCH_LIMIT",
        "const RUNNING_SAFE_PALETTE_PRIORITY",
        "fn handle_command_palette_key",
        "fn handle_command_palette_mouse_for_state",
        "fn clicked_command_palette_index",
        "fn command_palette_selection_event",
        "fn complete_selected_command",
        "fn clamp_selected_command",
        "fn slash_command_suggestions_for_state",
        "fn prioritize_running_safe_suggestions",
        "fn running_safe_palette_priority",
        "fn slash_command_query",
        "fn render_command_palette",
        "fn format_command_palette_text",
        "fn command_palette_match_token",
        "fn command_palette_matches_line_index",
    ] {
        let entrypoint_marker = if item.starts_with("fn ") {
            format!("{item}(")
        } else {
            item.to_string()
        };
        let owner_marker = entrypoint_marker.clone();
        assert!(
            palette_source.contains(&owner_marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&entrypoint_marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(
                item.trim_start_matches("const ")
                    .trim_start_matches("fn ")
                    .split(':')
                    .next()
                    .unwrap_or(item)
            ),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_credential_prompt_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/credential_prompt.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for credential prompt ownership"
    );
    assert!(
        ui_entrypoint.contains("mod credential_prompt;"),
        "src/ui.rs should register the credential prompt owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let credential_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "struct CredentialPrompt",
        "struct CredentialPromptSpec",
        "fn handle_tui_credential_set_for_state",
        "fn handle_credential_prompt_key",
        "fn confirm_credential_prompt",
        "fn parse_tui_credential_set",
        "fn credential_prompt_hidden_body",
        "fn credential_prompt_hidden_cursor",
    ] {
        let entrypoint_marker = if item.starts_with("fn ") {
            format!("{item}(")
        } else {
            item.to_string()
        };
        let owner_marker = entrypoint_marker.clone();
        assert!(
            credential_source.contains(&owner_marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&entrypoint_marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("struct ").trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_chat_view_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/chat_view.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for chat view ownership"
    );
    assert!(
        ui_entrypoint.contains("mod chat_view;"),
        "src/ui.rs should register the chat view owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let chat_view_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "struct ChatUiLayout",
        "fn chat_ui_layout",
        "fn transcript_visible_message_count",
        "fn transcript_window",
        "fn format_transcript_text",
        "fn format_messages_title",
        "fn render_chat_ui",
        "fn message_box_cursor_position",
    ] {
        let entrypoint_marker = if item.starts_with("fn ") {
            format!("{item}(")
        } else {
            item.to_string()
        };
        let owner_marker = entrypoint_marker.clone();
        assert!(
            chat_view_source.contains(&owner_marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&entrypoint_marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("struct ").trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_chat_history_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/chat_history.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for chat history ownership"
    );
    assert!(
        ui_entrypoint.contains("mod chat_history;"),
        "src/ui.rs should register the chat history owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let chat_history_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "struct ChatLine",
        "const TUI_HISTORY_MESSAGE_CHARS",
        "fn chat_lines_from_runtime",
        "fn session_messages_to_chat_lines",
        "fn truncate_history_message",
    ] {
        let entrypoint_marker = if item.starts_with("fn ") {
            format!("{item}(")
        } else {
            item.to_string()
        };
        assert!(
            chat_history_source.contains(&entrypoint_marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&entrypoint_marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("struct ").trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_session_projection_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/session_projection.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for UI session projection ownership"
    );
    assert!(
        ui_entrypoint.contains("mod session_projection;"),
        "src/ui.rs should register the session projection owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let projection_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "struct ActiveSessionRef",
        "struct HeaderStatus",
        "fn active_session_ref",
        "fn sync_active_session_ref",
        "fn session_monitor_for_state",
        "fn header_status_for_state",
        "fn load_active_session_header",
        "fn load_active_session_monitor",
        "fn session_monitor_from_session",
        "fn summarize_plan_for_tui",
        "fn workspace_for_state",
    ] {
        let entrypoint_marker = if item.starts_with("fn ") {
            format!("{item}(")
        } else {
            item.to_string()
        };
        assert!(
            projection_source.contains(&entrypoint_marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&entrypoint_marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("struct ").trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_message_box_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/message_box.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for UI input message box ownership"
    );
    assert!(
        ui_entrypoint.contains("mod message_box;"),
        "src/ui.rs should register the message box owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let message_box_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "enum MessageBoxAction",
        "struct MessageBox",
        "fn handle_prompt_input_key",
        "fn handle_key",
        "fn insert_str",
    ] {
        let marker = if item.starts_with("fn ") {
            format!("{item}(")
        } else {
            item.to_string()
        };
        assert!(
            message_box_source.contains(&marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(
                item.trim_start_matches("enum ")
                    .trim_start_matches("struct ")
                    .trim_start_matches("fn ")
            ),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_worker_drain_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/worker.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for UI worker drain ownership"
    );
    assert!(
        ui_entrypoint.contains("mod worker;"),
        "src/ui.rs should register the worker drain owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let worker_source = fs::read_to_string(root.join(source)).unwrap();
    for item in ["struct WorkerDone", "fn drain_progress", "fn drain_done"] {
        let marker = if item.starts_with("fn ") {
            format!("{item}(")
        } else {
            item.to_string()
        };
        assert!(
            worker_source.contains(&marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("struct ").trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_runtime_lifecycle_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/runtime_lifecycle.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for UI runtime lifecycle ownership"
    );
    assert!(
        ui_entrypoint.contains("mod runtime_lifecycle;"),
        "src/ui.rs should register the runtime lifecycle owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let lifecycle_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "fn stop_running_task",
        "fn mark_active_session_paused",
        "fn rebuild_runtime_for_active_session",
    ] {
        let marker = format!("{item}(");
        assert!(
            lifecycle_source.contains(&marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_input_submission_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/input_submission.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for UI input submission ownership"
    );
    assert!(
        ui_entrypoint.contains("mod input_submission;"),
        "src/ui.rs should register the input submission owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let input_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "fn submit_tui_input",
        "fn handle_tui_local_command",
        "fn apply_resume_result",
    ] {
        let marker = format!("{item}(");
        assert!(
            input_source.contains(&marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_text_helper_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/text.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for shared UI text helper ownership"
    );
    assert!(
        ui_entrypoint.contains("mod text;"),
        "src/ui.rs should register the shared UI text owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let text_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "fn format_optional_u64",
        "fn format_optional_bytes",
        "fn format_cache_hit_rate",
        "fn format_latest_environment",
        "fn compact_ui_text",
        "fn format_action_event",
        "fn latest_action_result_line",
        "fn latest_action_result",
        "fn non_empty_output_lines",
        "fn first_non_empty_line",
        "fn short_id",
    ] {
        let marker = format!("{item}(");
        assert!(
            text_source.contains(&marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_scrolling_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/scrolling.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for UI scrolling ownership"
    );
    assert!(
        ui_entrypoint.contains("mod scrolling;"),
        "src/ui.rs should register the UI scrolling owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let scrolling_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "const TRANSCRIPT_SCROLL_STEP",
        "const TRANSCRIPT_MOUSE_SCROLL_STEP",
        "const RESULT_SCROLL_STEP",
        "const RESULT_MOUSE_SCROLL_STEP",
        "fn handle_transcript_scroll_key",
        "fn handle_result_scroll_key",
        "fn scroll_result_from_mouse",
        "fn scroll_result",
        "fn scroll_result_down",
        "fn result_scroll_event",
        "fn result_output_line_count",
        "fn scroll_transcript",
        "fn transcript_scroll_event",
    ] {
        let marker = match item.strip_prefix("fn ") {
            Some(_) => format!("{item}("),
            None => item.to_string(),
        };
        assert!(
            scrolling_source.contains(&marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("fn ").trim_start_matches("const ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_quick_action_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/quick_actions.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for UI quick action ownership"
    );
    assert!(
        ui_entrypoint.contains("mod quick_actions;"),
        "src/ui.rs should register the UI quick action owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let quick_action_source = fs::read_to_string(root.join(source)).unwrap();
    for item in [
        "fn handle_monitor_quick_action_key",
        "fn selected_quick_action_event",
        "fn activate_selected_monitor_quick_action",
        "fn activate_monitor_quick_action_at_row",
    ] {
        let marker = format!("{item}(");
        assert!(
            quick_action_source.contains(&marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_paste_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/paste.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for UI paste ownership"
    );
    assert!(
        ui_entrypoint.contains("mod paste;"),
        "src/ui.rs should register the UI paste owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let paste_source = fs::read_to_string(root.join(source)).unwrap();
    for item in ["fn handle_tui_paste", "fn normalize_pasted_text"] {
        let marker = format!("{item}(");
        assert!(
            paste_source.contains(&marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_module_docs_cover_geometry_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/geometry.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for UI geometry helper ownership"
    );
    assert!(
        ui_entrypoint.contains("mod geometry;"),
        "src/ui.rs should register the UI geometry owner module"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let geometry_source = fs::read_to_string(root.join(source)).unwrap();
    for item in ["fn rect_contains", "fn rect_content_row_contains"] {
        let marker = format!("{item}(");
        assert!(
            geometry_source.contains(&marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_entrypoint_is_final_orchestration_boundary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();

    let allowed_function_prefixes = [
        "pub async fn run_basic_repl",
        "pub async fn run_tui",
        "async fn run_tui_loop",
        "fn handle_tui_mouse",
        "fn handle_tools_scroll_mouse",
        "fn handle_tui_key",
        "fn cycle_monitor_tab",
    ];
    let unexpected_functions: Vec<_> = ui_entrypoint
        .lines()
        .map(str::trim_start)
        .filter(|line| {
            line.starts_with("pub async fn ")
                || line.starts_with("async fn ")
                || line.starts_with("fn ")
        })
        .filter(|line| {
            !allowed_function_prefixes
                .iter()
                .any(|allowed| line.starts_with(allowed))
        })
        .collect();

    assert!(
        unexpected_functions.is_empty(),
        "src/ui.rs should only keep TUI lifecycle and event dispatch functions, found: {unexpected_functions:?}"
    );

    for function in allowed_function_prefixes {
        assert!(
            ui_entrypoint.contains(function),
            "src/ui.rs should retain the orchestration boundary function {function}"
        );
        let documented_name = function
            .trim_start_matches("pub async fn ")
            .trim_start_matches("async fn ")
            .trim_start_matches("fn ");
        assert!(
            ui_doc.contains(documented_name),
            "docs/MODULES/ui.md should document remaining src/ui.rs boundary function {documented_name}"
        );
    }

    assert!(
        ui_doc.contains("UI entrypoint final orchestration boundary"),
        "docs/MODULES/ui.md should explicitly document the final UI entrypoint boundary"
    );
    assert!(
        ui_doc.contains("TuiState"),
        "docs/MODULES/ui.md should document that TuiState remains in src/ui.rs"
    );
}

#[test]
fn ui_module_docs_cover_dashboard_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let ui_entrypoint = fs::read_to_string(root.join("src/ui.rs")).unwrap();
    let source = "src/ui/dashboard.rs";

    assert!(
        root.join(source).exists(),
        "{source} should exist for dashboard snapshot ownership"
    );
    assert!(
        ui_entrypoint.contains("mod dashboard;"),
        "src/ui.rs should register the dashboard owner module"
    );
    assert!(
        ui_entrypoint.contains("pub use dashboard::{render_dashboard, TuiSnapshot};"),
        "src/ui.rs should re-export dashboard's public API"
    );
    assert!(
        ui_doc.contains(source),
        "docs/MODULES/ui.md should mention {source}"
    );

    let dashboard_source = fs::read_to_string(root.join(source)).unwrap();
    for item in ["struct TuiSnapshot", "fn render_dashboard"] {
        let entrypoint_marker = if item.starts_with("fn ") {
            format!("pub {item}(")
        } else {
            format!("pub {item}")
        };
        let owner_marker = if item.starts_with("fn ") {
            format!("pub {item}(")
        } else {
            format!("pub {item}")
        };
        assert!(
            dashboard_source.contains(&owner_marker),
            "{item} should live in {source}"
        );
        assert!(
            !ui_entrypoint.contains(&entrypoint_marker),
            "{item} should not remain in src/ui.rs"
        );
        assert!(
            ui_doc.contains(item.trim_start_matches("struct ").trim_start_matches("fn ")),
            "docs/MODULES/ui.md should document {item} ownership"
        );
    }
}

#[test]
fn ui_terminal_smoke_gate_is_documented() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script_path = root.join("scripts/tui-smoke");
    let ui_doc = fs::read_to_string(root.join("docs/MODULES/ui.md")).unwrap();
    let harness_doc = fs::read_to_string(root.join("docs/HARNESS.md")).unwrap();

    assert!(
        script_path.exists(),
        "scripts/tui-smoke should exist for real terminal UI smoke acceptance"
    );
    let script = fs::read_to_string(&script_path).expect("read scripts/tui-smoke");
    for marker in [
        "/usr/bin/script",
        "mktemp -d",
        "stty rows 32 cols 100",
        "TERM=xterm-256color",
        "printf '/quit\\n'",
        "deepcli session",
        "> ",
        "found removed terminal marker",
    ] {
        assert!(
            script.contains(marker),
            "scripts/tui-smoke should contain `{marker}`"
        );
    }
    for marker in [
        "Messages",
        "Message Box",
        "Status",
        "Task Monitor",
        "deepcli>",
    ] {
        assert!(
            script.contains(marker),
            "scripts/tui-smoke should assert removed marker `{marker}` stays absent"
        );
    }
    assert!(
        !script.contains("\"Overview\""),
        "scripts/tui-smoke should not require the removed task monitor overview"
    );
    for doc in [&ui_doc, &harness_doc] {
        assert!(
            doc.contains("scripts/tui-smoke"),
            "UI terminal smoke gate should be documented"
        );
    }
}

#[test]
fn commands_module_docs_cover_split_source_files() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands_doc = fs::read_to_string(root.join("docs/MODULES/commands.md")).unwrap();
    for source in [
        "src/commands/response.rs",
        "src/commands/action_checklist.rs",
        "src/commands/benchmark_artifacts.rs",
        "src/commands/benchmark_baselines.rs",
        "src/commands/benchmark_dispatch.rs",
        "src/commands/benchmark_history.rs",
        "src/commands/benchmark_presets.rs",
        "src/commands/benchmark_runs.rs",
        "src/commands/benchmark_status.rs",
        "src/commands/command_policy.rs",
        "src/commands/delivery_diff.rs",
        "src/commands/delivery_reports.rs",
        "src/commands/delivery_verify.rs",
        "src/commands/round_benchmark_gates.rs",
        "src/commands/round_goal_status.rs",
        "src/commands/round_report.rs",
        "src/commands/scorecard_opportunities.rs",
        "src/commands/scorecard_report.rs",
        "src/commands/registry.rs",
        "src/commands/parser.rs",
        "src/commands/help.rs",
        "src/commands/completion.rs",
        "src/commands/version.rs",
        "src/commands/quickstart.rs",
        "src/commands/resume.rs",
        "src/commands/selftest.rs",
        "src/commands/preflight.rs",
        "src/commands/credentials.rs",
        "src/commands/session_catalog.rs",
        "src/commands/session_inspect.rs",
        "src/commands/session_recovery.rs",
        "src/commands/session_restore.rs",
        "src/commands/session_selection.rs",
        "src/commands/agent.rs",
        "src/commands/approval.rs",
        "src/commands/btw.rs",
        "src/commands/permissions.rs",
        "src/commands/timeout.rs",
        "src/commands/model.rs",
        "src/commands/logs.rs",
        "src/commands/trace.rs",
        "src/commands/context.rs",
        "src/commands/plan.rs",
        "src/commands/fork.rs",
        "src/commands/git.rs",
        "src/commands/git_identity.rs",
        "src/commands/terminal.rs",
        "src/commands/usage.rs",
        "src/commands/status.rs",
        "src/commands/test.rs",
        "src/commands/web.rs",
        "src/commands/prompt.rs",
        "src/commands/skill.rs",
        "src/commands/config.rs",
        "src/commands/privacy.rs",
        "src/commands/goal.rs",
        "src/commands/diagnose.rs",
        "src/commands/doctor.rs",
        "src/commands/recipes.rs",
        "src/commands/opportunities.rs",
        "src/commands/productloop.rs",
        "src/commands/session.rs",
        "src/commands/env.rs",
        "src/commands/delivery.rs",
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

fn documented_command_rows(contents: &str) -> BTreeMap<String, Vec<String>> {
    let mut commands = BTreeMap::new();
    for line in contents.lines() {
        if !line.starts_with("| /") {
            continue;
        }
        let cells = line
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if cells.len() < 5 {
            continue;
        }
        let command = cells[0].clone();
        assert!(
            commands.insert(command.clone(), cells).is_none(),
            "{command} documented more than once"
        );
    }
    commands
}
