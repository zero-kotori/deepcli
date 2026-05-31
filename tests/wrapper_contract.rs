use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

struct WrapperRun {
    _home: TempDir,
    workspace: TempDir,
    args: Vec<String>,
}

fn run_wrapper(args: &[&str]) -> WrapperRun {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    let fake_bin = home.path().join("fake-deepcli");
    write_fake_binary(&fake_bin);

    let output = Command::new("bash")
        .arg(wrapper_script())
        .args(args)
        .env("DEEPCLI_HOME", home.path())
        .env("DEEPCLI_BIN", &fake_bin)
        .env(
            "DEEPCLI_CONFIG",
            home.path().join(".deepcli").join("config.json"),
        )
        .current_dir(workspace.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "wrapper failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    WrapperRun {
        _home: home,
        workspace,
        args: String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect(),
    }
}

fn write_fake_binary(path: &Path) {
    fs::write(
        path,
        "#!/usr/bin/env bash\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\"; done\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn wrapper_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("deepcli")
}

fn count_arg(args: &[String], needle: &str) -> usize {
    args.iter().filter(|arg| arg.as_str() == needle).count()
}

fn has_adjacent(args: &[String], first: &str, second: &str) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == first && pair[1] == second)
}

fn ends_with_args(args: &[String], suffix: &[&str]) -> bool {
    args.len() >= suffix.len()
        && args[args.len() - suffix.len()..]
            .iter()
            .map(String::as_str)
            .eq(suffix.iter().copied())
}

#[test]
fn wrapper_maps_common_top_level_commands_to_slash_commands() {
    let run = run_wrapper(&["doctor", "--quick"]);
    let workspace = run.workspace.path().canonicalize().unwrap();

    assert_eq!(count_arg(&run.args, "-C"), 1);
    assert!(has_adjacent(&run.args, "-C", workspace.to_str().unwrap()));
    assert_eq!(count_arg(&run.args, "--config"), 1);
    assert_eq!(count_arg(&run.args, "--yes"), 1);
    assert!(ends_with_args(&run.args, &["/doctor", "--quick"]));

    let doctor_json = run_wrapper(&[
        "doctor",
        "--quick",
        "--json",
        "--output",
        ".deepcli/exports/doctor.json",
    ]);
    assert!(ends_with_args(
        &doctor_json.args,
        &[
            "/doctor",
            "--quick",
            "--json",
            "--output",
            ".deepcli/exports/doctor.json",
        ]
    ));

    let doctor_env = run_wrapper(&["doctor", "docker", "--json"]);
    assert!(ends_with_args(
        &doctor_env.args,
        &["/doctor", "docker", "--json"]
    ));

    let doctor_shell = run_wrapper(&["doctor", "shell", "--json"]);
    assert!(ends_with_args(
        &doctor_shell.args,
        &["/doctor", "shell", "--json"]
    ));

    let health_shell = run_wrapper(&["health", "shell", "--json"]);
    assert!(ends_with_args(
        &health_shell.args,
        &["/health", "shell", "--json"]
    ));

    let next = run_wrapper(&["next"]);
    assert!(ends_with_args(&next.args, &["/next"]));

    let next_json = run_wrapper(&["next", "--json", "--output", ".deepcli/exports/next.json"]);
    assert!(ends_with_args(
        &next_json.args,
        &["/next", "--json", "--output", ".deepcli/exports/next.json",]
    ));

    let status = run_wrapper(&[
        "status",
        "--json",
        "--output",
        ".deepcli/exports/status.json",
    ]);
    assert!(ends_with_args(
        &status.args,
        &[
            "/status",
            "--json",
            "--output",
            ".deepcli/exports/status.json",
        ]
    ));

    let usage = run_wrapper(&["usage", "--json", "--output", ".deepcli/exports/usage.json"]);
    assert!(ends_with_args(
        &usage.args,
        &[
            "/usage",
            "--json",
            "--output",
            ".deepcli/exports/usage.json",
        ]
    ));

    let health = run_wrapper(&["health", "--json"]);
    assert!(ends_with_args(&health.args, &["/health", "--json"]));

    let login = run_wrapper(&["login", "deepseek", "--stdin"]);
    assert!(ends_with_args(
        &login.args,
        &["/login", "deepseek", "--stdin"]
    ));

    let auth = run_wrapper(&["auth", "--stdin"]);
    assert!(ends_with_args(&auth.args, &["/auth", "--stdin"]));

    let logout = run_wrapper(&["logout", "deepseek"]);
    assert!(ends_with_args(&logout.args, &["/logout", "deepseek"]));

    let docker = run_wrapper(&["docker", "--json"]);
    assert!(ends_with_args(&docker.args, &["/docker", "--json"]));

    let compiler = run_wrapper(&["compiler", "setup", "--smoke"]);
    assert!(ends_with_args(
        &compiler.args,
        &["/compiler", "setup", "--smoke"]
    ));

    let trace = run_wrapper(&["trace", "--json", "--output", ".deepcli/exports/trace.json"]);
    assert!(ends_with_args(
        &trace.args,
        &[
            "/trace",
            "--json",
            "--output",
            ".deepcli/exports/trace.json",
        ]
    ));

    let logs = run_wrapper(&["logs", "--json", "--output", ".deepcli/exports/logs.json"]);
    assert!(ends_with_args(
        &logs.args,
        &["/logs", "--json", "--output", ".deepcli/exports/logs.json",]
    ));

    let privacy = run_wrapper(&[
        "privacy",
        "--json",
        "--output",
        ".deepcli/exports/privacy.json",
    ]);
    assert!(ends_with_args(
        &privacy.args,
        &[
            "/privacy",
            "--json",
            "--output",
            ".deepcli/exports/privacy.json",
        ]
    ));

    let selftest = run_wrapper(&["selftest", "--json", "--fail-on-issues"]);
    assert!(ends_with_args(
        &selftest.args,
        &["/selftest", "--json", "--fail-on-issues"]
    ));

    let recipes = run_wrapper(&["recipes", "release", "--json"]);
    assert!(ends_with_args(
        &recipes.args,
        &["/recipes", "release", "--json"]
    ));

    let playbook = run_wrapper(&["playbook", "support"]);
    assert!(ends_with_args(&playbook.args, &["/playbook", "support"]));

    let scorecard = run_wrapper(&["scorecard", "--json"]);
    assert!(ends_with_args(&scorecard.args, &["/scorecard", "--json"]));

    let round = run_wrapper(&["round", "--json"]);
    assert!(ends_with_args(&round.args, &["/round", "--json"]));

    let iterate = run_wrapper(&["iterate", "--json", "--fail-on-gaps"]);
    assert!(ends_with_args(
        &iterate.args,
        &["/iterate", "--json", "--fail-on-gaps"]
    ));

    let benchmark = run_wrapper(&["benchmark", "--fail-below", "85"]);
    assert!(ends_with_args(
        &benchmark.args,
        &["/benchmark", "--fail-below", "85"]
    ));

    let benchmark_status = run_wrapper(&["benchmark", "status", "--json"]);
    assert!(ends_with_args(
        &benchmark_status.args,
        &["/benchmark", "status", "--json"]
    ));

    let benchmark_gate = run_wrapper(&["benchmark", "gate", "--json"]);
    assert!(ends_with_args(
        &benchmark_gate.args,
        &["/benchmark", "gate", "--json"]
    ));

    let benchmark_trends = run_wrapper(&["benchmark", "trends", "--json"]);
    assert!(ends_with_args(
        &benchmark_trends.args,
        &["/benchmark", "trends", "--json"]
    ));

    let benchmark_clean = run_wrapper(&["benchmark", "clean", "--dry-run", "--json"]);
    assert!(ends_with_args(
        &benchmark_clean.args,
        &["/benchmark", "clean", "--dry-run", "--json"]
    ));

    let preflight = run_wrapper(&["preflight", "--dry-run", "--json"]);
    assert!(ends_with_args(
        &preflight.args,
        &["/preflight", "--dry-run", "--json"]
    ));

    let release_check = run_wrapper(&["release-check", "--dry-run"]);
    assert!(ends_with_args(
        &release_check.args,
        &["/release-check", "--dry-run"]
    ));

    let completion = run_wrapper(&["completion", "zsh"]);
    assert!(ends_with_args(&completion.args, &["/completion", "zsh"]));

    let completion_install = run_wrapper(&["completion", "install", "zsh", "--force"]);
    assert!(ends_with_args(
        &completion_install.args,
        &["/completion", "install", "zsh", "--force"]
    ));

    let completion_status = run_wrapper(&["completion", "status", "zsh", "--json"]);
    assert!(ends_with_args(
        &completion_status.args,
        &["/completion", "status", "zsh", "--json"]
    ));

    let completions = run_wrapper(&["completions", "json"]);
    assert!(ends_with_args(&completions.args, &["/completions", "json"]));

    let diagnose = run_wrapper(&["diagnose", "--limit", "3"]);
    assert!(ends_with_args(
        &diagnose.args,
        &["/diagnose", "--limit", "3"]
    ));

    let diagnose_env = run_wrapper(&["diagnose", "compiler", "--json"]);
    assert!(ends_with_args(
        &diagnose_env.args,
        &["/diagnose", "compiler", "--json"]
    ));

    let diagnose_bundle = run_wrapper(&["diagnose", "--bundle", ".deepcli/support/latest"]);
    assert!(ends_with_args(
        &diagnose_bundle.args,
        &["/diagnose", "--bundle", ".deepcli/support/latest"]
    ));

    let support = run_wrapper(&["support", ".deepcli/support/slow-run"]);
    assert!(ends_with_args(
        &support.args,
        &["/support", ".deepcli/support/slow-run"]
    ));

    let cleanup = run_wrapper(&[
        "cleanup",
        "sessions",
        "--json",
        "--output",
        ".deepcli/exports/cleanup.json",
    ]);
    assert!(ends_with_args(
        &cleanup.args,
        &[
            "/cleanup",
            "sessions",
            "--json",
            "--output",
            ".deepcli/exports/cleanup.json",
        ]
    ));

    let verify = run_wrapper(&["verify", "--limit", "3"]);
    assert!(ends_with_args(&verify.args, &["/verify", "--limit", "3"]));

    let accept = run_wrapper(&["accept", "--json"]);
    assert!(ends_with_args(&accept.args, &["/accept", "--json"]));

    let gate = run_wrapper(&["gate", "--json"]);
    assert!(ends_with_args(&gate.args, &["/gate", "--json"]));

    let verify_env = run_wrapper(&[
        "verify",
        "--env-check",
        "docker",
        "--json",
        "--output",
        ".deepcli/exports/verify-env.json",
    ]);
    assert!(ends_with_args(
        &verify_env.args,
        &[
            "/verify",
            "--env-check",
            "docker",
            "--json",
            "--output",
            ".deepcli/exports/verify-env.json",
        ]
    ));

    let handoff = run_wrapper(&["handoff", "--path", "src"]);
    assert!(ends_with_args(
        &handoff.args,
        &["/handoff", "--path", "src"]
    ));

    let handoff_env = run_wrapper(&[
        "handoff",
        "--env-check",
        "docker",
        "--json",
        "--output",
        ".deepcli/exports/handoff-env.json",
    ]);
    assert!(ends_with_args(
        &handoff_env.args,
        &[
            "/handoff",
            "--env-check",
            "docker",
            "--json",
            "--output",
            ".deepcli/exports/handoff-env.json",
        ]
    ));

    let approvals = run_wrapper(&[
        "approval",
        "list",
        "--json",
        "--output",
        ".deepcli/exports/approvals.json",
    ]);
    assert!(ends_with_args(
        &approvals.args,
        &[
            "/approval",
            "list",
            "--json",
            "--output",
            ".deepcli/exports/approvals.json",
        ]
    ));

    let credentials = run_wrapper(&[
        "credentials",
        "status",
        "--json",
        "--output",
        ".deepcli/exports/credentials.json",
    ]);
    assert!(ends_with_args(
        &credentials.args,
        &[
            "/credentials",
            "status",
            "--json",
            "--output",
            ".deepcli/exports/credentials.json",
        ]
    ));

    let config_sources = run_wrapper(&[
        "config",
        "sources",
        "--json",
        "--output",
        ".deepcli/exports/config-sources.json",
    ]);
    assert!(ends_with_args(
        &config_sources.args,
        &[
            "/config",
            "sources",
            "--json",
            "--output",
            ".deepcli/exports/config-sources.json",
        ]
    ));

    let config_get = run_wrapper(&[
        "config",
        "get",
        "agent.providerTurnTimeoutSeconds",
        "--json",
        "--output",
        ".deepcli/exports/config-timeout.json",
    ]);
    assert!(ends_with_args(
        &config_get.args,
        &[
            "/config",
            "get",
            "agent.providerTurnTimeoutSeconds",
            "--json",
            "--output",
            ".deepcli/exports/config-timeout.json",
        ]
    ));

    let version = run_wrapper(&[
        "version",
        "--json",
        "--output",
        ".deepcli/exports/version.json",
    ]);
    assert!(ends_with_args(
        &version.args,
        &[
            "/version",
            "--json",
            "--output",
            ".deepcli/exports/version.json",
        ]
    ));

    let about = run_wrapper(&["about", "--json"]);
    assert!(ends_with_args(&about.args, &["/about", "--json"]));

    let timeout = run_wrapper(&["timeout", "900"]);
    assert!(ends_with_args(&timeout.args, &["/timeout", "900"]));

    let timeout_json = run_wrapper(&[
        "timeout",
        "--json",
        "--output",
        ".deepcli/exports/timeout.json",
    ]);
    assert!(ends_with_args(
        &timeout_json.args,
        &[
            "/timeout",
            "--json",
            "--output",
            ".deepcli/exports/timeout.json",
        ]
    ));

    let permissions = run_wrapper(&[
        "permissions",
        "show",
        "--json",
        "--output",
        ".deepcli/exports/permissions.json",
    ]);
    assert!(ends_with_args(
        &permissions.args,
        &[
            "/permissions",
            "show",
            "--json",
            "--output",
            ".deepcli/exports/permissions.json",
        ]
    ));

    let models = run_wrapper(&[
        "model",
        "list",
        "--json",
        "--output",
        ".deepcli/exports/models.json",
    ]);
    assert!(ends_with_args(
        &models.args,
        &[
            "/model",
            "list",
            "--json",
            "--output",
            ".deepcli/exports/models.json",
        ]
    ));

    let providers = run_wrapper(&["providers", "--json"]);
    assert!(ends_with_args(&providers.args, &["/providers", "--json"]));

    let model_alias = run_wrapper(&["models", "--json"]);
    assert!(ends_with_args(&model_alias.args, &["/models", "--json"]));

    let use_model = run_wrapper(&["use", "kimi"]);
    assert!(ends_with_args(&use_model.args, &["/use", "kimi"]));

    let switch_model = run_wrapper(&["switch", "deepseek", "deepseek-v4-pro"]);
    assert!(ends_with_args(
        &switch_model.args,
        &["/switch", "deepseek", "deepseek-v4-pro"]
    ));

    let provider_set = run_wrapper(&["provider", "kimi"]);
    assert!(ends_with_args(&provider_set.args, &["/provider", "kimi"]));

    let provider_show = run_wrapper(&["provider", "--json"]);
    assert!(ends_with_args(
        &provider_show.args,
        &["/provider", "--json"]
    ));

    let prompts = run_wrapper(&[
        "prompt",
        "list",
        "--json",
        "--output",
        ".deepcli/exports/prompts.json",
    ]);
    assert!(ends_with_args(
        &prompts.args,
        &[
            "/prompt",
            "list",
            "--json",
            "--output",
            ".deepcli/exports/prompts.json",
        ]
    ));

    let skills = run_wrapper(&[
        "skill",
        "list",
        "--json",
        "--output",
        ".deepcli/exports/skills.json",
    ]);
    assert!(ends_with_args(
        &skills.args,
        &[
            "/skill",
            "list",
            "--json",
            "--output",
            ".deepcli/exports/skills.json",
        ]
    ));

    let agents = run_wrapper(&[
        "agent",
        "list",
        "--json",
        "--output",
        ".deepcli/exports/agents.json",
    ]);
    assert!(ends_with_args(
        &agents.args,
        &[
            "/agent",
            "list",
            "--json",
            "--output",
            ".deepcli/exports/agents.json",
        ]
    ));

    let tests = run_wrapper(&[
        "test",
        "discover",
        "--json",
        "--output",
        ".deepcli/exports/tests.json",
    ]);
    assert!(ends_with_args(
        &tests.args,
        &[
            "/test",
            "discover",
            "--json",
            "--output",
            ".deepcli/exports/tests.json",
        ]
    ));

    let test_run = run_wrapper(&[
        "test",
        "run",
        "--json",
        "--output",
        ".deepcli/exports/test-run.json",
        "--",
        "cargo",
        "test",
    ]);
    assert!(ends_with_args(
        &test_run.args,
        &[
            "/test",
            "run",
            "--json",
            "--output",
            ".deepcli/exports/test-run.json",
            "--",
            "cargo",
            "test",
        ]
    ));

    let env_test = run_wrapper(&["test", "docker", "--json"]);
    assert!(ends_with_args(
        &env_test.args,
        &["/env", "test", "docker", "--json"]
    ));

    let env_check = run_wrapper(&[
        "env",
        "check",
        "docker",
        "--json",
        "--output",
        ".deepcli/exports/env-check.json",
    ]);
    assert!(ends_with_args(
        &env_check.args,
        &[
            "/env",
            "check",
            "docker",
            "--json",
            "--output",
            ".deepcli/exports/env-check.json",
        ]
    ));

    let check = run_wrapper(&["check", "docker", "--json"]);
    assert!(ends_with_args(&check.args, &["/check", "docker", "--json"]));

    let env_plan = run_wrapper(&[
        "env",
        "plan",
        "compiler",
        "--smoke",
        "--json",
        "--output",
        ".deepcli/exports/env-plan.json",
    ]);
    assert!(ends_with_args(
        &env_plan.args,
        &[
            "/env",
            "plan",
            "compiler",
            "--smoke",
            "--json",
            "--output",
            ".deepcli/exports/env-plan.json",
        ]
    ));

    let setup = run_wrapper(&["setup", "docker", "--smoke"]);
    assert!(ends_with_args(
        &setup.args,
        &["/setup", "docker", "--smoke"]
    ));

    let install = run_wrapper(&["install", "compiler", "--smoke"]);
    assert!(ends_with_args(
        &install.args,
        &["/install", "compiler", "--smoke"]
    ));

    let btw = run_wrapper(&[
        "btw",
        "list",
        "--json",
        "--output",
        ".deepcli/exports/btw.json",
    ]);
    assert!(ends_with_args(
        &btw.args,
        &[
            "/btw",
            "list",
            "--json",
            "--output",
            ".deepcli/exports/btw.json",
        ]
    ));
}

#[test]
fn wrapper_preserves_explicit_cwd_config_and_yes() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    let explicit_workspace = TempDir::new().unwrap();
    let fake_bin = home.path().join("fake-deepcli");
    let explicit_config = home.path().join("explicit-config.json");
    write_fake_binary(&fake_bin);

    let output = Command::new("bash")
        .arg(wrapper_script())
        .args([
            "-C",
            explicit_workspace.path().to_str().unwrap(),
            "--config",
            explicit_config.to_str().unwrap(),
            "--yes",
            "/doctor",
            "--quick",
        ])
        .env("DEEPCLI_HOME", home.path())
        .env("DEEPCLI_BIN", &fake_bin)
        .env(
            "DEEPCLI_CONFIG",
            home.path().join(".deepcli").join("config.json"),
        )
        .current_dir(workspace.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "wrapper failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let args = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(count_arg(&args, "-C"), 1);
    assert!(has_adjacent(
        &args,
        "-C",
        explicit_workspace.path().to_str().unwrap()
    ));
    assert_eq!(count_arg(&args, "--config"), 1);
    assert!(has_adjacent(
        &args,
        "--config",
        explicit_config.to_str().unwrap()
    ));
    assert_eq!(count_arg(&args, "--yes"), 1);
    assert!(ends_with_args(&args, &["/doctor", "--quick"]));
}

#[test]
fn wrapper_session_aliases_distinguish_list_and_actions() {
    let list = run_wrapper(&["sessions", "--all"]);
    assert!(ends_with_args(&list.args, &["/session", "list", "--all"]));

    let limited = run_wrapper(&["sessions", "--limit", "5"]);
    assert!(ends_with_args(
        &limited.args,
        &["/session", "list", "--limit", "5"]
    ));

    let history_alias = run_wrapper(&["history", "--limit", "5"]);
    assert!(ends_with_args(
        &history_alias.args,
        &["/history", "--limit", "5"]
    ));

    let list_json = run_wrapper(&[
        "sessions",
        "--json",
        "--output",
        ".deepcli/exports/sessions.json",
    ]);
    assert!(ends_with_args(
        &list_json.args,
        &[
            "/session",
            "list",
            "--json",
            "--output",
            ".deepcli/exports/sessions.json",
        ]
    ));

    let search_json = run_wrapper(&[
        "session",
        "search",
        "compiler",
        "--json",
        "--output",
        ".deepcli/exports/session-search.json",
    ]);
    assert!(ends_with_args(
        &search_json.args,
        &[
            "/session",
            "search",
            "compiler",
            "--json",
            "--output",
            ".deepcli/exports/session-search.json",
        ]
    ));

    let history = run_wrapper(&["session", "history", "--limit", "20"]);
    assert!(ends_with_args(
        &history.args,
        &["/session", "history", "--limit", "20"]
    ));

    let diagnose = run_wrapper(&[
        "session",
        "diagnose",
        "--json",
        "--output",
        ".deepcli/exports/session-diagnose.json",
    ]);
    assert!(ends_with_args(
        &diagnose.args,
        &[
            "/session",
            "diagnose",
            "--json",
            "--output",
            ".deepcli/exports/session-diagnose.json",
        ]
    ));

    let history = run_wrapper(&[
        "session",
        "history",
        "--json",
        "--output",
        ".deepcli/exports/session-history.json",
    ]);
    assert!(ends_with_args(
        &history.args,
        &[
            "/session",
            "history",
            "--json",
            "--output",
            ".deepcli/exports/session-history.json",
        ]
    ));

    let tools = run_wrapper(&[
        "session",
        "tools",
        "--failed",
        "--json",
        "--output",
        ".deepcli/exports/session-tools.json",
    ]);
    assert!(ends_with_args(
        &tools.args,
        &[
            "/session",
            "tools",
            "--failed",
            "--json",
            "--output",
            ".deepcli/exports/session-tools.json",
        ]
    ));
}

#[test]
fn wrapper_help_topic_forwards_to_slash_help() {
    let run = run_wrapper(&["help", "doctor"]);

    assert!(ends_with_args(&run.args, &["/help", "doctor"]));
}

#[test]
fn wrapper_quickstart_forwards_to_slash_quickstart() {
    let run = run_wrapper(&["quickstart"]);

    assert!(ends_with_args(&run.args, &["/quickstart"]));

    let gate = run_wrapper(&["quickstart", "--json", "--fail-on-missing"]);
    assert!(ends_with_args(
        &gate.args,
        &["/quickstart", "--json", "--fail-on-missing"]
    ));
}

#[test]
fn provider_aliases_accept_top_level_slash_commands() {
    let run = run_wrapper(&["deepseek", "doctor", "--quick"]);

    assert!(has_adjacent(&run.args, "--provider", "deepseek"));
    assert!(has_adjacent(&run.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&run.args, &["/doctor", "--quick"]));

    let doctor_shell = run_wrapper(&["deepseek", "doctor", "shell", "--json"]);
    assert!(has_adjacent(&doctor_shell.args, "--provider", "deepseek"));
    assert!(has_adjacent(
        &doctor_shell.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &doctor_shell.args,
        &["/doctor", "shell", "--json"]
    ));

    let diagnose = run_wrapper(&["deepseek", "diagnose", "--limit", "2"]);
    assert!(has_adjacent(&diagnose.args, "--provider", "deepseek"));
    assert!(has_adjacent(&diagnose.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(
        &diagnose.args,
        &["/diagnose", "--limit", "2"]
    ));

    let support = run_wrapper(&["deepseek", "support"]);
    assert!(has_adjacent(&support.args, "--provider", "deepseek"));
    assert!(has_adjacent(&support.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&support.args, &["/support"]));

    let logs = run_wrapper(&["deepseek", "logs", "--limit", "20"]);
    assert!(has_adjacent(&logs.args, "--provider", "deepseek"));
    assert!(has_adjacent(&logs.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&logs.args, &["/logs", "--limit", "20"]));

    let privacy = run_wrapper(&["deepseek", "privacy", "--json"]);
    assert!(has_adjacent(&privacy.args, "--provider", "deepseek"));
    assert!(has_adjacent(&privacy.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&privacy.args, &["/privacy", "--json"]));

    let selftest = run_wrapper(&["deepseek", "selftest", "--json"]);
    assert!(has_adjacent(&selftest.args, "--provider", "deepseek"));
    assert!(has_adjacent(&selftest.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&selftest.args, &["/selftest", "--json"]));

    let recipes = run_wrapper(&["deepseek", "recipes", "release", "--json"]);
    assert!(has_adjacent(&recipes.args, "--provider", "deepseek"));
    assert!(has_adjacent(&recipes.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(
        &recipes.args,
        &["/recipes", "release", "--json"]
    ));

    let scorecard = run_wrapper(&["deepseek", "scorecard", "--json"]);
    assert!(has_adjacent(&scorecard.args, "--provider", "deepseek"));
    assert!(has_adjacent(&scorecard.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&scorecard.args, &["/scorecard", "--json"]));

    let round = run_wrapper(&["deepseek", "round", "--json"]);
    assert!(has_adjacent(&round.args, "--provider", "deepseek"));
    assert!(has_adjacent(&round.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&round.args, &["/round", "--json"]));

    let benchmark_status = run_wrapper(&["deepseek", "benchmark", "status", "--json"]);
    assert!(has_adjacent(
        &benchmark_status.args,
        "--provider",
        "deepseek"
    ));
    assert!(has_adjacent(
        &benchmark_status.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &benchmark_status.args,
        &["/benchmark", "status", "--json"]
    ));

    let benchmark_gate = run_wrapper(&["deepseek", "benchmark", "gate", "--json"]);
    assert!(has_adjacent(&benchmark_gate.args, "--provider", "deepseek"));
    assert!(has_adjacent(
        &benchmark_gate.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &benchmark_gate.args,
        &["/benchmark", "gate", "--json"]
    ));

    let benchmark_trends = run_wrapper(&["deepseek", "benchmark", "trends", "--json"]);
    assert!(has_adjacent(
        &benchmark_trends.args,
        "--provider",
        "deepseek"
    ));
    assert!(has_adjacent(
        &benchmark_trends.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &benchmark_trends.args,
        &["/benchmark", "trends", "--json"]
    ));

    let benchmark_clean = run_wrapper(&["deepseek", "benchmark", "clean", "--dry-run", "--json"]);
    assert!(has_adjacent(
        &benchmark_clean.args,
        "--provider",
        "deepseek"
    ));
    assert!(has_adjacent(
        &benchmark_clean.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &benchmark_clean.args,
        &["/benchmark", "clean", "--dry-run", "--json"]
    ));

    let preflight = run_wrapper(&["deepseek", "preflight", "--dry-run", "--json"]);
    assert!(has_adjacent(&preflight.args, "--provider", "deepseek"));
    assert!(has_adjacent(&preflight.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(
        &preflight.args,
        &["/preflight", "--dry-run", "--json"]
    ));

    let release_check = run_wrapper(&["deepseek", "release-check", "--dry-run"]);
    assert!(has_adjacent(&release_check.args, "--provider", "deepseek"));
    assert!(has_adjacent(
        &release_check.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &release_check.args,
        &["/release-check", "--dry-run"]
    ));

    let completion = run_wrapper(&["deepseek", "completion", "json"]);
    assert!(has_adjacent(&completion.args, "--provider", "deepseek"));
    assert!(has_adjacent(&completion.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&completion.args, &["/completion", "json"]));

    let completion_install = run_wrapper(&["deepseek", "completion", "install", "zsh"]);
    assert!(has_adjacent(
        &completion_install.args,
        "--provider",
        "deepseek"
    ));
    assert!(has_adjacent(
        &completion_install.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &completion_install.args,
        &["/completion", "install", "zsh"]
    ));

    let completion_status = run_wrapper(&["deepseek", "completion", "status", "zsh"]);
    assert!(has_adjacent(
        &completion_status.args,
        "--provider",
        "deepseek"
    ));
    assert!(has_adjacent(
        &completion_status.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &completion_status.args,
        &["/completion", "status", "zsh"]
    ));

    let setup = run_wrapper(&["deepseek", "setup", "docker", "--smoke"]);
    assert!(has_adjacent(&setup.args, "--provider", "deepseek"));
    assert!(has_adjacent(&setup.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(
        &setup.args,
        &["/setup", "docker", "--smoke"]
    ));

    let health = run_wrapper(&["deepseek", "health"]);
    assert!(has_adjacent(&health.args, "--provider", "deepseek"));
    assert!(has_adjacent(&health.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&health.args, &["/health"]));

    let health_shell = run_wrapper(&["deepseek", "health", "shell", "--json"]);
    assert!(has_adjacent(&health_shell.args, "--provider", "deepseek"));
    assert!(has_adjacent(
        &health_shell.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &health_shell.args,
        &["/health", "shell", "--json"]
    ));

    let providers = run_wrapper(&["deepseek", "providers"]);
    assert!(has_adjacent(&providers.args, "--provider", "deepseek"));
    assert!(has_adjacent(&providers.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&providers.args, &["/providers"]));

    let use_default = run_wrapper(&["deepseek", "use"]);
    assert!(has_adjacent(&use_default.args, "--provider", "deepseek"));
    assert!(has_adjacent(
        &use_default.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(&use_default.args, &["/use", "deepseek"]));

    let switch_kimi = run_wrapper(&["deepseek", "switch", "kimi"]);
    assert!(has_adjacent(&switch_kimi.args, "--provider", "deepseek"));
    assert!(has_adjacent(
        &switch_kimi.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(&switch_kimi.args, &["/switch", "kimi"]));

    let provider_show = run_wrapper(&["deepseek", "provider", "--json"]);
    assert!(has_adjacent(&provider_show.args, "--provider", "deepseek"));
    assert!(has_adjacent(
        &provider_show.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &provider_show.args,
        &["/provider", "--json"]
    ));

    let version = run_wrapper(&["deepseek", "version", "--json"]);
    assert!(has_adjacent(&version.args, "--provider", "deepseek"));
    assert!(has_adjacent(&version.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&version.args, &["/version", "--json"]));

    let timeout = run_wrapper(&["deepseek", "timeout", "900"]);
    assert!(has_adjacent(&timeout.args, "--provider", "deepseek"));
    assert!(has_adjacent(&timeout.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&timeout.args, &["/timeout", "900"]));

    let history = run_wrapper(&["deepseek", "history"]);
    assert!(has_adjacent(&history.args, "--provider", "deepseek"));
    assert!(has_adjacent(&history.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&history.args, &["/history"]));

    let login = run_wrapper(&["deepseek", "login", "--stdin"]);
    assert!(has_adjacent(&login.args, "--provider", "deepseek"));
    assert!(has_adjacent(&login.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(
        &login.args,
        &["/login", "deepseek", "--stdin"]
    ));

    let auth = run_wrapper(&["deepseek", "auth", "kimi", "--stdin"]);
    assert!(has_adjacent(&auth.args, "--provider", "deepseek"));
    assert!(has_adjacent(&auth.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&auth.args, &["/auth", "kimi", "--stdin"]));

    let logout = run_wrapper(&["deepseek", "logout"]);
    assert!(has_adjacent(&logout.args, "--provider", "deepseek"));
    assert!(has_adjacent(&logout.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&logout.args, &["/logout", "deepseek"]));

    let doctor_env = run_wrapper(&["deepseek", "doctor", "docker"]);
    assert!(has_adjacent(&doctor_env.args, "--provider", "deepseek"));
    assert!(has_adjacent(&doctor_env.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&doctor_env.args, &["/doctor", "docker"]));

    let diagnose_env = run_wrapper(&["deepseek", "diagnose", "compiler"]);
    assert!(has_adjacent(&diagnose_env.args, "--provider", "deepseek"));
    assert!(has_adjacent(
        &diagnose_env.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(
        &diagnose_env.args,
        &["/diagnose", "compiler"]
    ));

    let check = run_wrapper(&["deepseek", "check", "docker"]);
    assert!(has_adjacent(&check.args, "--provider", "deepseek"));
    assert!(has_adjacent(&check.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&check.args, &["/check", "docker"]));

    let docker = run_wrapper(&["deepseek", "docker", "--json"]);
    assert!(has_adjacent(&docker.args, "--provider", "deepseek"));
    assert!(has_adjacent(&docker.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&docker.args, &["/docker", "--json"]));

    let compiler = run_wrapper(&["deepseek", "compiler", "setup", "--smoke"]);
    assert!(has_adjacent(&compiler.args, "--provider", "deepseek"));
    assert!(has_adjacent(&compiler.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(
        &compiler.args,
        &["/compiler", "setup", "--smoke"]
    ));

    let env_test = run_wrapper(&["deepseek", "test", "compiler"]);
    assert!(has_adjacent(&env_test.args, "--provider", "deepseek"));
    assert!(has_adjacent(&env_test.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(
        &env_test.args,
        &["/env", "test", "compiler"]
    ));

    let project_test = run_wrapper(&["deepseek", "test", "run"]);
    assert!(has_adjacent(&project_test.args, "--provider", "deepseek"));
    assert!(has_adjacent(
        &project_test.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(&project_test.args, &["/test", "run"]));

    let quickstart = run_wrapper(&["deepseek", "quickstart"]);
    assert!(has_adjacent(&quickstart.args, "--provider", "deepseek"));
    assert!(has_adjacent(&quickstart.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&quickstart.args, &["/quickstart"]));

    let cleanup = run_wrapper(&["deepseek", "cleanup", "sessions", "--json"]);
    assert!(has_adjacent(&cleanup.args, "--provider", "deepseek"));
    assert!(has_adjacent(&cleanup.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(
        &cleanup.args,
        &["/cleanup", "sessions", "--json"]
    ));

    let accept = run_wrapper(&["deepseek", "accept", "--json"]);
    assert!(has_adjacent(&accept.args, "--provider", "deepseek"));
    assert!(has_adjacent(&accept.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&accept.args, &["/accept", "--json"]));

    let gate = run_wrapper(&["deepseek", "gate", "--json"]);
    assert!(has_adjacent(&gate.args, "--provider", "deepseek"));
    assert!(has_adjacent(&gate.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&gate.args, &["/gate", "--json"]));
}

#[test]
fn provider_aliases_accept_help_topics() {
    let run = run_wrapper(&["deepseek", "help", "doctor"]);

    assert!(has_adjacent(&run.args, "--provider", "deepseek"));
    assert!(has_adjacent(&run.args, "--model", "deepseek-v4-pro"));
    assert!(ends_with_args(&run.args, &["/help", "doctor"]));
}

#[test]
fn wrapper_preserves_ask_and_stream_modes_for_binary_validation() {
    let ask = run_wrapper(&["ask", "doctro", "--quick"]);
    assert!(ends_with_args(&ask.args, &["ask", "doctro", "--quick"]));

    let stream = run_wrapper(&["stream", "hello"]);
    assert!(ends_with_args(&stream.args, &["stream", "hello"]));

    let provider_ask = run_wrapper(&["deepseek", "ask", "hello"]);
    assert!(has_adjacent(&provider_ask.args, "--provider", "deepseek"));
    assert!(has_adjacent(
        &provider_ask.args,
        "--model",
        "deepseek-v4-pro"
    ));
    assert!(ends_with_args(&provider_ask.args, &["ask", "hello"]));

    let provider_stream = run_wrapper(&["kimi", "stream", "hello"]);
    assert!(has_adjacent(&provider_stream.args, "--provider", "kimi"));
    assert!(has_adjacent(
        &provider_stream.args,
        "--model",
        "kimi-for-coding"
    ));
    assert!(ends_with_args(&provider_stream.args, &["stream", "hello"]));
}
