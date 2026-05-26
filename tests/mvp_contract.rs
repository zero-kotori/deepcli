use deep_cli::commands::CommandRouter;
use deep_cli::config::AppConfig;
use deep_cli::tools::ToolRegistry;

#[test]
fn mvp_slash_commands_are_registered() {
    let help = CommandRouter::help_text();
    for command in [
        "/help",
        "/status",
        "/context",
        "/permissions",
        "/config",
        "/model",
        "/plan",
        "/diff",
        "/review",
        "/test",
        "/env",
        "/git",
        "/prompt",
        "/skill",
        "/agent",
        "/btw",
        "/approval",
        "/session",
        "/resume",
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
